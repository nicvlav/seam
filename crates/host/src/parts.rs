//! What Seam builds for each instance of a plugin and hands to its
//! [`Plugin::new`]: its parameters, and its [`Parts`]; and the [`Driver`]
//! an engine runs the parts through.

use std::cell::{Cell, OnceCell, RefCell};
use std::ops::DerefMut;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use seam_learn::{Config, Learn, LearnAudio};
use seam_params::{DeclError, Declaration, ParamSet, Params, UiParams, Value};
use seam_session::{Session, SessionError, SessionUi};
use seam_undo::{Bound, History};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::{Plugin, SavedState};

mod run_time;
mod saved;

pub use run_time::Maker;
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
///         let history = parts.undo(Bound::default());
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
/// gain.parts.ui().write(gain.params.gain.id(), -6.0);
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
    /// The values [`Parts::saved_value`] made, in the order they were made.
    values: RefCell<Vec<Box<dyn Synced>>>,
    saved: Arc<SavedState>,
    /// Learn's revision when the tick last handed a save the mappings of
    /// the parameters made at run time.
    learned_at: Cell<Option<u64>>,
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
        self.assign_held();
        if let Some(history) = self.history.get() {
            history.tick();
        }
        self.sync();
        self.capture_learned();
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
        let saved = Arc::new(saved);
        let parts = Parts {
            saved: Arc::clone(&saved),
            // Both were made above, so each hands out its first handle.
            params: set.take_ui().expect("a new set's first UI handle"),
            ticked: Rc::new(Ticked {
                session: session.take_ui().expect("a new session's first UI handle"),
                history: OnceCell::new(),
                learn: OnceCell::new(),
                values: RefCell::new(Vec::new()),
                saved,
                learned_at: Cell::new(None),
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
    pub fn ui(&self) -> &UiParams {
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
    /// history. A second call returns the same history: clones share it. A
    /// parameter made at run time ([`declare`](Self::declare)) needs no
    /// enrolment: its gestures are steps whenever undo is on.
    ///
    /// The history is pruned to `bound`; a later call's is ignored.
    ///
    /// # Panics
    ///
    /// When the plugin made a history of its own over
    /// [`ui`](Self::ui) that is still alive: it uses the one this call
    /// returns instead. In an exporter, the instance is then not built.
    pub fn undo(&mut self, bound: Bound) -> History {
        if let Some(history) = self.ticked.history.get() {
            return history.clone();
        }
        let history = History::new(&self.params, bound).unwrap_or_else(|error| {
            panic!(
                "undo is on through a history the plugin made over Parts::ui ({error}): \
                 use the one Parts::undo returns, and make no other"
            )
        });
        history.enroll_params();
        // Empty, as just checked: nothing between the two runs plugin code.
        let _ = self.ticked.history.set(history.clone());
        history
    }

    /// Turns MIDI learn on, its mappings saved with the session.
    ///
    /// Seam runs both of learn's sides: the poll at each
    /// [tick](Driver::tick), and the audio side over each block's MIDI
    /// before the processor runs, so learned writes are in the parameters
    /// when it reads them. The plugin turns learn on in
    /// [`new`](Plugin::new). Turned on later, the audio side arrives with
    /// the next activation. The plugin's main thread reaches learn through
    /// [`learn`](Self::learn).
    ///
    /// # Panics
    ///
    /// When the plugin declares no MIDI input
    /// ([`PluginInfo::midi_input`](crate::PluginInfo::midi_input)): no
    /// controller would ever reach learn. And when learn's section key,
    /// [`SESSION_KEY`](seam_learn::SESSION_KEY), is taken: learn is on
    /// already, or a participant the plugin registered holds it. In an
    /// exporter, the instance is then not built.
    ///
    /// # Examples
    ///
    /// The [`idle`](Plugin::idle) example turns learn on and learns a
    /// controller.
    pub fn midi_learn(&mut self, config: Config) {
        assert!(
            self.midi_input,
            "MIDI learn needs MIDI input: set the plugin's PluginInfo::midi_input"
        );
        // Each activation takes an audio side of its own.
        let (learn, _audio) = Learn::new(&self.set, config);
        let learn = Rc::new(RefCell::new(learn));
        if let Err(error) = Learn::attach(&learn, &self.ticked.session) {
            panic!(
                "MIDI learn's session key is taken ({error}): turn learn on once, \
                 and register no participant under seam_learn::SESSION_KEY"
            );
        }
        // Empty: learn holds its key while it lives, so the attach above
        // would have failed had learn been on.
        let _ = self.ticked.learn.set(learn);
    }

    /// Returns learn's main-thread side, to arm, map or clear.
    ///
    /// Under an exporter, learn's audio side is Seam's. Calling
    /// [`replace_audio`](Learn::replace_audio) on this learn retires the one
    /// the running activation's blocks go through, which then keeps the
    /// mappings it last read and sees no later one.
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

    /// Returns a plain value that starts at `default` and is saved with the
    /// session under `key`. A project saved without the key loads `default`
    /// into it too, so a new instance and an older project cannot differ.
    ///
    /// It is for text, a path or structure. A number the DSP reads is a
    /// parameter instead: [`declare`](Self::declare) says which to use.
    ///
    /// The value is shared: clone it to whoever else reads or writes it.
    /// Undo records it once it is [enrolled](History::enroll), a separate
    /// call, since a value such as a view setting is saved and not undone.
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
    /// [`SessionError::EmptyKey`] for an empty key. Nothing is saved then. A
    /// key may be built at run time, from a name say, so this is an error to
    /// handle and not a panic.
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
    ///
    /// let (_params, mut parts) = Parts::for_plugin::<Gain>()?;
    /// let driver = Driver::of(&parts);
    /// let label = parts.saved_value("label", String::from("Untitled"))?;
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
    pub fn saved_value<T>(&mut self, key: &str, default: T) -> Result<Value<T>, SessionError>
    where
        T: Clone + Serialize + DeserializeOwned + 'static,
    {
        let value = Value::new(default.clone(), &self.set);
        let saved = SavedValue::register(&self.ticked.session, key, &value, default)?;
        self.ticked.values.borrow_mut().push(Box::new(saved));
        Ok(value)
    }

    /// Captures each value [`saved_value`](Self::saved_value) made that moved
    /// since the last tick, as the tick does, for a caller whose next save must
    /// already hold a change.
    ///
    /// Thread: main.
    pub fn sync(&mut self) {
        self.ticked.sync();
    }

    /// Makes `P`'s parameters while the plugin runs, under `scope`, and
    /// hands them back: the same [`Float`](seam_params::Float),
    /// [`Bool`](seam_params::Bool) and [`Choice`](seam_params::Choice) that
    /// the listed ones are, listed to no host.
    ///
    /// Which to use for state a plugin keeps: a number the DSP reads is a
    /// parameter. It is listed in the plugin's [`Params`](Plugin::Params)
    /// when a host should see it, and made here when it should not, or when
    /// how many there are is known only as the plugin runs: a tree of
    /// instruments, articulations or groups, with parameters at each node.
    /// Text, a path or structure is a [`Value`], made with
    /// [`saved_value`](Self::saved_value).
    ///
    /// A parameter made here is written through [`ui`](Self::ui) as a listed
    /// one is, read with `get` from any thread, so the audio thread reads a
    /// write in its next block, and taken by undo and learn as a listed one
    /// is. The host hears nothing of it.
    ///
    /// Each is saved under its path, `scope/key` (`group/17/volume`), its
    /// value read at the save, and its learned mapping beside it. A scope is
    /// any string; `/` separates its levels by convention, and the empty
    /// scope puts the key at the top. After a load, a declare under a path
    /// the load holds makes the parameter holding the saved value, before
    /// anything else can see it.
    ///
    /// The owner saves its structure only, the ids its scopes are made from,
    /// in a session participant whose apply declares each owner again through
    /// a [`Maker`]. Register that participant in [`Plugin::new`]: a load's
    /// values are held for the declares the pass that applies it makes, so a
    /// participant that registers after that pass, or whose apply fails,
    /// declares its owners at their defaults. When that pass finishes, every
    /// parameter made here and still held takes the load's mapping for its
    /// path, and one declared before the load, made once in `new` say, takes
    /// the load's value for its path too, or its default.
    ///
    /// Two parameters the plugin holds under one path are a fault, and a save
    /// writes only the later; declaring the paths of a tree a load replaces,
    /// in that load's apply, is not. So an owner's id stays unique for as
    /// long as undo can bring the owner back: a counter that only grows,
    /// saved with the tree, does it.
    ///
    /// Thread: main.
    ///
    /// # Panics
    ///
    /// On a mistake fixed in the plugin's source: a declaration refused, a
    /// bypass, a key the struct declares twice, or a plain table
    /// ([`Declarer::extend`](seam_params::Declarer::extend)). A struct the
    /// derive wrote was checked while compiling, so it never panics here.
    /// And while learn is on and borrowed, as [`learn`](Self::learn) says.
    ///
    /// # Examples
    ///
    /// ```
    #[doc = include_str!("../doc/gain.rs")]
    /// struct Group {
    ///     volume: Float,
    /// }
    ///
    /// impl Params for Group {
    ///     fn declare(p: &mut Declarer) -> Self {
    ///         Group { volume: p.add(FloatDecl::new("volume", "Volume", -60.0..=12.0, 0.0)) }
    ///     }
    /// }
    ///
    /// let (_params, parts) = Parts::for_plugin::<Gain>()?;
    /// let group: Group = parts.declare("group/17");
    ///
    /// // Written as a listed parameter is, and read with `get`.
    /// assert!(parts.ui().write(&group.volume, -6.0));
    /// assert_eq!(group.volume.get(), -6.0);
    ///
    /// // The host lists only the gain; a save writes the group's volume too.
    /// assert_eq!(parts.set().len(), 1);
    /// let saved = parts.saved().save();
    /// let block: serde_json::Value = serde_json::from_slice(&saved)?;
    /// assert_eq!(block["runTime"]["group/17/volume"]["value"], -6.0);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn declare<P: Params>(&self, scope: &str) -> P {
        self.maker().declare(scope)
    }

    /// Makes one parameter while the plugin runs, under `scope`, as
    /// [`declare`](Self::declare) makes a struct's: a parameter the DSP
    /// reads that no host should see, made once in [`Plugin::new`], say.
    ///
    /// Thread: main.
    ///
    /// # Panics
    ///
    /// As [`declare`](Self::declare) says.
    ///
    /// # Examples
    ///
    /// ```
    #[doc = include_str!("../doc/gain.rs")]
    /// let (_params, parts) = Parts::for_plugin::<Gain>()?;
    /// let drive = FloatDecl::new("drive", "Drive", 0.0..=1.0, 0.0);
    /// let drive: Float = parts.add("", drive);
    /// assert!(parts.ui().write(&drive, 0.5));
    ///
    /// let saved = parts.saved().save();
    /// let block: serde_json::Value = serde_json::from_slice(&saved)?;
    /// assert_eq!(block["runTime"]["drive"]["value"], 0.5);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn add<D: Declaration>(&self, scope: &str, decl: D) -> D::Param {
        self.maker().add(scope, decl)
    }

    /// Returns the handle that makes parameters while the plugin runs, for
    /// code that cannot reach the parts. [`Maker`] shows a session
    /// participant that brings a tree back through one.
    pub fn maker(&self) -> Maker {
        Maker::new(Rc::downgrade(&self.ticked), Arc::clone(&self.saved))
    }
}

/// What an exporter's engine holds to run a plugin's [`Parts`]: a link to
/// them that does not keep them alive.
///
/// Seam makes one for each instance before [`Plugin::new`] takes the parts.
/// Through it the engine [ticks](Self::tick) the parts before each
/// [`idle`](Plugin::idle), and takes learn's [audio side](Self::learn_audio)
/// at each activation, which it runs over each block's MIDI before the
/// processor. So a plugin never ticks its parts and its processor never
/// runs learn. It is public for a test that builds a plugin with no host,
/// which does both as the engine would. Clones share the link.
///
/// # Examples
///
/// A test ticks the parts as the engine does, and once the plugin that kept
/// them is dropped, a tick does nothing. The [`Parts`] example records an
/// undo step, and the [`idle`](Plugin::idle) example learns a controller and
/// saves its mapping.
///
/// ```
#[doc = include_str!("../doc/gain.rs")]
/// use seam_host::Driver;
///
/// let (params, mut parts) = Parts::for_plugin::<Gain>()?;
/// let label = parts.saved_value("label", String::from("Untitled"))?;
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
    ///    applies a session the host's load restored. Learn's restore clears
    ///    every mapping, those of parameters made at run time
    ///    ([`Parts::declare`]) included.
    /// 3. Once a pass that began after a load has finished, the load's
    ///    parameters made at run time, in one step: each one made before
    ///    the load, and still held, takes the load's value for its path, or
    ///    its default; every one still held takes the load's mapping for its
    ///    path. Then the load's entries for them go. One step, so the result
    ///    does not depend on whether the plugin's tree or learn applied
    ///    first, and no mapping is refused as full for one of the tree the
    ///    load replaced.
    /// 4. The history's tick, when undo is on: it records what changed since
    ///    the last tick. Its place does not matter: it clears the history
    ///    whenever a restore was stored or applied since it last looked.
    /// 5. [`Parts::sync`]: it captures each value [`Parts::saved_value`]
    ///    made that moved since the last tick, a write undo replayed
    ///    included. It skips one whose last write was its own apply's, which
    ///    the session's tick captured already, but not one another
    ///    participant's apply wrote after it. Until this step a save holds
    ///    the value from before the change.
    /// 6. When learn's mappings moved since the last tick: it hands the next
    ///    save the mappings of the parameters made at run time, so they too
    ///    are captured at the tick.
    ///
    /// Nothing of the parts is held while the session's tick runs, so a
    /// participant the plugin registered may call [`Parts::saved_value`] or
    /// [`Parts::sync`] from its apply. Learn is held while it polls, so a
    /// learn target's code may not reach learn through [`Parts::learn`].
    ///
    /// When nothing changed it allocates nothing, and its cost does not grow
    /// with the number of parameters: of those made at run time, learn's
    /// poll looks only at each one armed or mapped. Each saved value costs
    /// one atomic load.
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

    /// Returns a fresh audio side of learn for one activation, or `None`
    /// while learn is off or once the parts are dropped.
    ///
    /// The engine takes one at each activation and runs it over each block's
    /// MIDI before the processor's call. The one it replaces keeps the
    /// mappings it last read until it is dropped, off the audio thread, with
    /// the activation it served. A test with no host takes one after
    /// [`Plugin::activate`] and runs it before each block it hands the
    /// processor; the [`idle`](Plugin::idle) example does.
    ///
    /// Thread: main.
    ///
    /// # Panics
    ///
    /// While the guard [`Parts::learn`] returned is alive. The engine never
    /// holds one when it calls this.
    pub fn learn_audio(&self) -> Option<LearnAudio> {
        let ticked = self.ticked.upgrade()?;
        let audio = ticked.learn.get()?.borrow_mut().replace_audio();
        Some(audio)
    }
}
