//! What Seam builds for each instance of a plugin and hands to its
//! [`Plugin::new`]: its parameters, and its [`Parts`]; and the [`Driver`]
//! that runs the parts' tick.

use std::cell::{OnceCell, RefCell};
use std::ops::DerefMut;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use seam_learn::{Config, Learn, LearnAudio};
use seam_params::{DeclError, ParamSet, UiParams, Value};
use seam_session::{Session, SessionError, SessionUi};
use seam_undo::{Bound, History, HistoryError};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::{Plugin, SavedState};

mod saved;

use saved::{SavedValue, Synced};

/// One instance's parts, already joined: its parameter set, the session
/// over it, the saved state the host saves and loads through, and undo and
/// MIDI learn once the plugin turns them on.
///
/// Seam builds them for each instance and hands them to [`Plugin::new`], with
/// the parameters the plugin's [`Params`](Plugin::Params) declared, and ticks
/// them before each [`idle`](Plugin::idle) for as long as the plugin keeps
/// them ([`Driver::tick`] says what the tick does). They hold the main
/// thread's handles, so a plugin that keeps them is not `Send`. One that
/// wants to be drops them and keeps its parameters, which are.
///
/// # Examples
///
/// A plugin built with no host, as a test builds one: the parts come from
/// [`for_plugin`](Self::for_plugin), as an exporter's do, and a [`Driver`]
/// ticks them, as the exporter's engine does.
///
/// ```
/// use seam_host::{Block, Driver, Parts, Plugin, ProcessSetup, Processor, Status};
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
///         let history = parts.undo(Bound::default()).expect("no other history");
///         Gain { parts, params, history }
///     }
/// #
/// #   fn activate(&mut self, _setup: &ProcessSetup) -> Silent {
/// #       Silent
/// #   }
/// }
///
/// let (params, parts) = Parts::for_plugin::<Gain>()?;
/// let driver = Driver::of(&parts);
/// let gain = Gain::new(params, parts);
///
/// // An editor's drag, recorded as one undo step at the next tick.
/// gain.parts.params().write(gain.params.gain.id(), -6.0);
/// driver.tick();
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
    /// What the tick reaches, which a [`Driver`] holds weakly.
    ticked: Rc<Ticked>,
    /// Whether the plugin declares MIDI input, which learn needs.
    midi_input: bool,
}

/// The parts a tick runs.
///
/// Undo and learn are set once, and the saved values are borrowed only
/// while they are captured. So a participant's apply or capture, which runs
/// inside the session's tick, can reach back into the [`Parts`] and finds
/// nothing held. Learn is held across its poll, which runs a learn target's
/// code.
#[derive(Debug)]
struct Ticked {
    session: SessionUi,
    history: OnceCell<History>,
    learn: OnceCell<Rc<RefCell<Learn>>>,
    /// The values [`Parts::save`] keeps saved, in the order it was called.
    values: RefCell<Vec<Box<dyn Synced>>>,
}

impl Ticked {
    /// Runs one tick, in the order [`Driver::tick`] gives.
    fn tick(&self) {
        if let Some(learn) = self.learn.get() {
            // What it committed is the mapping, which a plugin asks learn
            // for.
            let _learned = learn.borrow_mut().poll();
        }
        self.session.tick();
        if let Some(history) = self.history.get() {
            history.tick();
        }
        self.sync();
    }

    fn sync(&self) {
        for value in self.values.borrow_mut().iter_mut() {
            value.sync();
        }
    }
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
            ticked: Rc::new(Ticked {
                session: session.take_ui().expect("a new session's first UI handle"),
                history: OnceCell::new(),
                learn: OnceCell::new(),
                values: RefCell::new(Vec::new()),
            }),
            set,
            midi_input: P::INFO.midi_input,
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
        &self.ticked.session
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
    ///
    /// # Errors
    ///
    /// Returns [`HistoryError::HistoryExists`] when the plugin made a history
    /// of its own over [`params`](Self::params), which is still alive.
    pub fn undo(&mut self, bound: Bound) -> Result<History, HistoryError> {
        if let Some(history) = self.ticked.history.get() {
            return Ok(history.clone());
        }
        let history = History::new(&self.params, bound)?;
        history.enroll_params();
        // Empty, as just checked: nothing between the two runs plugin code.
        let _ = self.ticked.history.set(history.clone());
        Ok(history)
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
        Learn::attach(&learn, &self.ticked.session)?;
        // Empty: learn holds its key while it lives, so the attach above
        // would have failed had learn been on.
        let _ = self.ticked.learn.set(learn);
        Ok(())
    }

    /// Returns learn's main-thread side, to arm, map or clear.
    ///
    /// # Panics
    ///
    /// While learn is off ([`midi_learn`](Self::midi_learn) turns it on), or
    /// while learn is borrowed: the guard from an earlier call is alive, or a
    /// learn target's code calls it while [`Driver::tick`] polls learn.
    pub fn learn(&self) -> impl DerefMut<Target = Learn> + '_ {
        self.ticked
            .learn
            .get()
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

    /// Saves `value` with the session under `key` from now on, and loads
    /// `default` into it from a project saved without the key.
    ///
    /// A change is captured at the next [tick](Driver::tick), not when it is
    /// written. So a save the host makes between the two, about one tick or
    /// 16 ms, still holds the value from before the change; a parameter is
    /// never stale this way. [`sync`](Self::sync) captures at once.
    ///
    /// The whole value is captured whenever it moved, so state that is large
    /// and changes a part at a time saves under several keys, one value each.
    /// A section waiting under `key` from an earlier load applies before this
    /// returns. A section that cannot be read as a `T` fails its apply: the
    /// value keeps what it held, and the section stays saved.
    ///
    /// Thread: main.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::KeyTaken`] when another saved value, learn or
    /// a participant the plugin registered holds `key`, and
    /// [`SessionError::EmptyKey`] for an empty key. Nothing is saved then.
    ///
    /// # Panics
    ///
    /// In a build with debug assertions, here or at a later tick, when serde
    /// cannot write the value as JSON: a map whose keys are not strings, say.
    ///
    /// # Examples
    ///
    /// ```
    #[doc = include_str!("../doc/gain.rs")]
    /// use seam_host::Driver;
    /// use seam_params::Value;
    ///
    /// let (_params, mut parts) = Parts::for_plugin::<Gain>()?;
    /// let driver = Driver::of(&parts);
    /// let label = Value::new(String::from("Untitled"), parts.set());
    /// parts.save("label", &label, String::from("Untitled"))?;
    ///
    /// // What the host's next save holds.
    /// let saved = |parts: &Parts| -> serde_json::Result<serde_json::Value> {
    ///     serde_json::from_slice(&parts.saved().save())
    /// };
    ///
    /// // Captured at the tick, so a save before it holds the label from
    /// // before the change.
    /// label.set(String::from("Drums"));
    /// assert_eq!(saved(&parts)?["extra"]["label"], "Untitled");
    /// driver.tick();
    /// assert_eq!(saved(&parts)?["extra"]["label"], "Drums");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn save<T>(&mut self, key: &str, value: &Value<T>, default: T) -> Result<(), SessionError>
    where
        T: Clone + Serialize + DeserializeOwned + 'static,
    {
        let saved = SavedValue::register(&self.ticked.session, key, value, default)?;
        self.ticked.values.borrow_mut().push(Box::new(saved));
        Ok(())
    }

    /// Captures each value [`save`](Self::save) keeps saved that moved since
    /// the last tick, as the tick does, for a caller whose next save must
    /// already hold a change.
    ///
    /// Thread: main.
    pub fn sync(&mut self) {
        self.ticked.sync();
    }
}

/// What an exporter's engine holds to run a plugin's [`Parts`]: a link to
/// them that does not keep them alive.
///
/// Seam makes one for each instance before [`Plugin::new`] takes the parts,
/// and [ticks](Self::tick) through it before each [`idle`](Plugin::idle), so
/// a plugin never ticks its parts. It is public for a test that builds a
/// plugin with no host, which ticks it as the engine would. Clones share the
/// link.
///
/// # Examples
///
/// A test ticks the parts as the engine does, and a dropped plugin's not at
/// all. The [`Parts`] example records an undo step, and the
/// [`idle`](Plugin::idle) example saves a learned mapping.
///
/// ```
#[doc = include_str!("../doc/gain.rs")]
/// use seam_host::Driver;
/// use seam_params::Value;
///
/// let (params, mut parts) = Parts::for_plugin::<Gain>()?;
/// let label = Value::new(String::from("Untitled"), parts.set());
/// parts.save("label", &label, String::from("Untitled"))?;
/// let driver = Driver::of(&parts);
/// let saved = std::sync::Arc::clone(parts.saved());
/// let saved_label = || -> serde_json::Result<serde_json::Value> {
///     let block: serde_json::Value = serde_json::from_slice(&saved.save())?;
///     Ok(block["extra"]["label"].clone())
/// };
/// let gain = Gain::new(params, parts);
///
/// label.set(String::from("Drums"));
/// driver.tick();
/// assert_eq!(saved_label()?, "Drums");
///
/// // The parts went with the plugin, so nothing is captured any more.
/// drop(gain);
/// label.set(String::from("Bass"));
/// driver.tick();
/// assert_eq!(saved_label()?, "Drums");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct Driver {
    ticked: Weak<Ticked>,
}

impl Driver {
    /// Returns the driver of `parts`.
    ///
    /// Thread: main.
    pub fn of(parts: &Parts) -> Driver {
        Driver {
            ticked: Rc::downgrade(&parts.ticked),
        }
    }

    /// Runs one main-thread tick of every part, in the one order that works.
    /// Once the parts are dropped, it does nothing.
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
    /// 4. [`Parts::sync`]: it captures each value [`Parts::save`] keeps saved
    ///    that moved since the last tick, a write undo replayed included. It
    ///    skips one whose last write was its own apply's, which the session's
    ///    tick captured already, but not one another participant's apply
    ///    wrote after it. Until this step a save holds the value from before
    ///    the change.
    ///
    /// Nothing of the parts is held while the session's tick runs, so a
    /// participant the plugin registered may call [`Parts::save`] or
    /// [`Parts::sync`] from its apply. Learn is held while it polls, so a
    /// learn target's code may not reach learn through [`Parts::learn`].
    ///
    /// When nothing changed it allocates nothing, and its cost does not grow
    /// with the number of parameters; each saved value costs one atomic load.
    ///
    /// Thread: main.
    ///
    /// # Panics
    ///
    /// When learn is on and borrowed: the guard [`Parts::learn`] returned is
    /// alive, or a learn target's code calls [`Parts::learn`] during the
    /// poll.
    pub fn tick(&self) {
        if let Some(ticked) = self.ticked.upgrade() {
            ticked.tick();
        }
    }
}
