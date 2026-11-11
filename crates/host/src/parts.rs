//! What Seam builds for each instance of a plugin and hands to its
//! [`Plugin::new`]: its parameters, and its [`Parts`].

use std::cell::{Cell, RefCell};
use std::ops::DerefMut;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use seam_learn::{Config, Learn, LearnAudio, Learned};
use seam_params::{DeclError, ParamSet, UiParams};
use seam_session::{Session, SessionError, SessionUi};
use seam_undo::{Bound, History};

use crate::{Plugin, SavedState};

/// One instance's parts, already joined: its parameter set, the session
/// over it, the saved state the host saves and loads through, and undo and
/// MIDI learn once the plugin turns them on.
///
/// Seam builds them for each instance and hands them to [`Plugin::new`], with
/// the parameters the plugin's [`Params`](Plugin::Params) declared. A plugin
/// that keeps them calls [`tick`](Self::tick) from its
/// [`idle`](Plugin::idle). They hold the main thread's handles, so a plugin
/// that keeps them is not `Send`. One that wants to be drops them and keeps
/// its parameters, which are.
///
/// # Examples
///
/// A plugin built with no host, as a test builds one: the parts come from
/// [`for_plugin`](Self::for_plugin), as an exporter's do.
///
/// ```
/// use seam_host::{Block, Parts, Plugin, ProcessSetup, Processor, Status};
/// use seam_params::{Declarer, Float, FloatDecl, Params};
/// use seam_undo::{Bound, History};
/// # use seam_host::{AuCodes, Category, Layout, PluginInfo};
///
/// struct Level {
///     gain: Float,
/// }
///
/// impl Params for Level {
///     fn declare(p: &mut Declarer) -> Self {
///         Level { gain: p.add(FloatDecl::new("gain", "Gain", -24.0..=24.0, 0.0)) }
///     }
/// }
///
/// # struct Silent;
/// # impl Processor for Silent {
/// #     fn process(&mut self, _block: Block<'_, '_>) -> Status {
/// #         Status::Sleep
/// #     }
/// # }
/// #
/// struct Gain {
///     parts: Parts,
///     params: Level,
///     history: History,
/// }
///
/// impl Plugin for Gain {
/// #   const INFO: PluginInfo = PluginInfo {
/// #       name: "Gain",
/// #       vendor: "Example",
/// #       url: "",
/// #       email: "",
/// #       description: "",
/// #       version: "0.1.0",
/// #       id: "dev.example.gain",
/// #       vst3_class_id: [0; 16],
/// #       au_codes: AuCodes { subtype: *b"gain", manufacturer: *b"Exmp" },
/// #       category: Category::Effect,
/// #       features: &[],
/// #       layouts: &[Layout::STEREO],
/// #       midi_input: false,
/// #       midi_output: false,
/// #   };
///     type Params = Level;
/// #   type Processor = Silent;
///
///     fn new(params: Level, mut parts: Parts) -> Self {
///         let history = parts.undo(Bound::default());
///         Gain { parts, params, history }
///     }
///
///     fn idle(&mut self) {
///         self.parts.tick();
///     }
/// #
/// #   fn activate(&mut self, _setup: &ProcessSetup) -> Silent {
/// #       Silent
/// #   }
/// }
///
/// let (params, parts) = Parts::for_plugin::<Gain>()?;
/// let mut gain = Gain::new(params, parts);
///
/// // An editor's drag, recorded as one undo step at the next tick.
/// gain.parts.params().write(gain.params.gain.id(), -6.0);
/// gain.idle();
/// assert_eq!(gain.history.len(), 1);
///
/// // What the host's next save holds.
/// let block: serde_json::Value = serde_json::from_slice(&gain.parts.saved().save())?;
/// assert_eq!(block["parameters"]["gain"], -6.0);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct Parts {
    set: Arc<ParamSet>,
    saved: Arc<SavedState>,
    params: UiParams,
    session: SessionUi,
    history: Option<History>,
    learn: Option<Rc<RefCell<Learn>>>,
    /// Whether the plugin declares MIDI input, which learn needs.
    midi_input: bool,
    /// Moved by every tick, so the engine can tell a plugin that keeps its
    /// parts and never ticks them.
    ticks: Arc<AtomicU64>,
}

impl Parts {
    /// Returns the parameters and the parts of a `P`: its set, declared by
    /// `P::Params`, the session over it, and the saved state, which upgrades
    /// an older block with [`P::migrate`](Plugin::migrate).
    ///
    /// The exporters call it for each instance. So does a test that builds a
    /// plugin with no host; the [`Parts`] example is one.
    ///
    /// # Errors
    ///
    /// Returns the [`DeclError`] of the first declaration refused, as
    /// [`ParamSet::declare`] does. An exporter then refuses to build the
    /// instance.
    #[expect(
        clippy::missing_panics_doc,
        reason = "a set and a session made here still have their first UI handles"
    )]
    pub fn for_plugin<P: Plugin>() -> Result<(P::Params, Parts), DeclError> {
        let (set, params) = ParamSet::declare::<P::Params>()?;
        let session = Session::new(&set);
        let mut saved = SavedState::new(&set, &session);
        if P::SCHEMA_VERSION > 0 {
            saved = saved.with_migration(P::SCHEMA_VERSION, P::migrate);
        }
        let parts = Parts {
            saved: Arc::new(saved),
            // Both were made above, so each hands out its first handle.
            params: set.take_ui().expect("a new set's first UI handle"),
            session: session.take_ui().expect("a new session's first UI handle"),
            set,
            history: None,
            learn: None,
            midi_input: P::INFO.midi_input,
            ticks: Arc::default(),
        };
        Ok((params, parts))
    }

    /// Returns the parameter set, which the host lists.
    pub fn set(&self) -> &Arc<ParamSet> {
        &self.set
    }

    /// Returns the handle an editor writes parameters through, which undo
    /// records.
    pub fn params(&self) -> &UiParams {
        &self.params
    }

    /// Returns the session, for a participant the plugin registers itself.
    pub fn session(&self) -> &SessionUi {
        &self.session
    }

    /// Returns the saved state the host's save and load go through, for a
    /// test that saves with no host.
    pub fn saved(&self) -> &Arc<SavedState> {
        &self.saved
    }

    /// Turns undo on, with every parameter enrolled, and returns the
    /// history. A second call returns the same history: clones share it.
    ///
    /// The history is pruned to `bound`; a later call's is ignored.
    #[expect(
        clippy::missing_panics_doc,
        reason = "only the parts make a history over their handle, and only once"
    )]
    pub fn undo(&mut self, bound: Bound) -> History {
        self.history
            .get_or_insert_with(|| {
                let history = History::new(&self.params, bound)
                    .expect("only the parts make a history over their handle");
                history.enroll_params();
                history
            })
            .clone()
    }

    /// Turns MIDI learn on, its mappings saved with the session.
    ///
    /// A processor takes learn's audio side at each activation, from
    /// [`learn_audio`](Self::learn_audio), and the plugin's main thread reaches
    /// learn through [`learn`](Self::learn).
    ///
    /// # Errors
    ///
    /// Returns [`SessionError`] when learn's section key,
    /// [`SESSION_KEY`](seam_learn::SESSION_KEY), is taken: learn is on
    /// already, or a participant registered under it.
    ///
    /// # Panics
    ///
    /// When the plugin declares no MIDI input
    /// ([`PluginInfo::midi_input`](crate::PluginInfo::midi_input)): no
    /// controller would ever reach learn. In an exporter, the instance is
    /// then not built.
    ///
    /// # Examples
    ///
    /// The [`idle`](Plugin::idle) example turns learn on and learns a
    /// controller.
    pub fn midi_learn(&mut self, config: Config) -> Result<(), SessionError> {
        assert!(
            self.midi_input,
            "MIDI learn needs MIDI input: set the plugin's PluginInfo::midi_input"
        );
        // Each activation takes an audio side of its own.
        let (learn, _audio) = Learn::new(&self.set, config);
        let learn = Rc::new(RefCell::new(learn));
        Learn::attach(&learn, &self.session)?;
        self.learn = Some(learn);
        Ok(())
    }

    /// Returns learn's main-thread side, to arm, map or clear.
    ///
    /// # Panics
    ///
    /// While learn is off ([`midi_learn`](Self::midi_learn) turns it on), or
    /// while the guard from an earlier call is alive.
    pub fn learn(&self) -> impl DerefMut<Target = Learn> + '_ {
        self.learn
            .as_ref()
            .expect("MIDI learn is off: call Parts::midi_learn first")
            .borrow_mut()
    }

    /// Returns a fresh audio side of learn, for one activation's processor.
    ///
    /// # Panics
    ///
    /// As [`learn`](Self::learn).
    pub fn learn_audio(&self) -> LearnAudio {
        self.learn().replace_audio()
    }

    /// Runs one main-thread tick of every part, in the one order that works,
    /// and returns what learn committed, if anything.
    ///
    /// A plugin that keeps its parts calls it from every
    /// [`idle`](Plugin::idle). An exporter reports a plugin that keeps them
    /// and lets an idle pass without a tick, once, as a fault: until it
    /// ticks, a session the host's load restored never applies.
    ///
    /// The order:
    ///
    /// 1. Learn's poll, when learn is on: it commits a mapping learned since
    ///    the last tick and writes the learned values. Learn is held as it
    ///    commits, so the session can capture the mapping only at its next
    ///    tick. Polled after the session, a new mapping would miss the save
    ///    until the tick after.
    /// 2. The session's tick: it captures what changed for the next save, and
    ///    applies a session the host's load restored.
    /// 3. The history's tick, when undo is on: it records what changed since
    ///    the last tick. Its place does not matter: it clears the history
    ///    whenever a restore was stored or applied since it last looked.
    ///
    /// When nothing changed it allocates nothing, and its cost does not grow
    /// with the number of parameters.
    ///
    /// Thread: main.
    ///
    /// # Examples
    ///
    /// The [`Parts`] example records an undo step, and the
    /// [`idle`](Plugin::idle) example saves a learned mapping.
    pub fn tick(&mut self) -> Option<Learned> {
        self.ticks.fetch_add(1, Ordering::Relaxed);
        let learned = self
            .learn
            .as_ref()
            .and_then(|learn| learn.borrow_mut().poll());
        self.session.tick();
        if let Some(history) = &self.history {
            history.tick();
        }
        learned
    }

    /// What the engine watches for a forgotten tick.
    pub(crate) fn watch(&self) -> TickWatch {
        TickWatch {
            ticks: Cell::new(Arc::downgrade(&self.ticks)),
            seen: Cell::new(self.ticks.load(Ordering::Relaxed)),
        }
    }
}

/// The engine's side of a plugin's tick count: it tells whether a plugin
/// that still holds its parts let an idle pass without ticking them.
///
/// It holds the count weakly, so a plugin that dropped its parts is never
/// reported.
pub(crate) struct TickWatch {
    /// Emptied for good once the parts are dropped or a tick is missed, so
    /// the miss is told once.
    ticks: Cell<Weak<AtomicU64>>,
    seen: Cell<u64>,
}

impl TickWatch {
    /// Whether the parts are still held and were not ticked since the last
    /// call: true at most once. Called after each of the plugin's idles.
    pub(crate) fn missed(&self) -> bool {
        let ticks = self.ticks.take();
        let Some(count) = ticks.upgrade() else {
            return false;
        };
        let now = count.load(Ordering::Relaxed);
        if now == self.seen.get() {
            return true;
        }
        self.seen.set(now);
        drop(count);
        self.ticks.set(ticks);
        false
    }
}
