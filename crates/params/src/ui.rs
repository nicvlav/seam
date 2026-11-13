//! The UI thread's writes: user edits through gesture tokens, the plugin's
//! own edits, and undo replays, all sharing one gesture count per parameter.
//!
//! A listed parameter's gesture state sits in an array by id. One made at
//! run time has its state in a map by store only while a gesture is open on
//! it, so an idle one costs nothing here. Both kinds take one path: a
//! [`Which`] names the parameter a call reaches.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::ops::Deref;
use std::rc::Rc;
use std::sync::Arc;

use crate::param::sealed::Token;
use crate::param::{Naming, ParamRef};
use crate::set::ParamCell;
use crate::{GestureRecord, Origin, ParamId, ParamSet, RestoreReader, RunTimeRecord, Writable};

/// Who is writing through a gesture. Only a user's writes become undo
/// steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Writer {
    User,
    Program,
    Replay,
}

impl Writer {
    fn origin(self) -> Origin {
        match self {
            Writer::User => Origin::User,
            Writer::Program => Origin::Program,
            Writer::Replay => Origin::Replay,
        }
    }
}

/// The parameter a UI-side call reaches: one the set lists, by its place,
/// or one this instance made at run time, by its store.
#[derive(Debug, Clone)]
enum Which {
    Listed(ParamId),
    RunTime(Arc<ParamCell>),
}

impl Which {
    fn cell<'a>(&'a self, set: &'a ParamSet) -> &'a ParamCell {
        match self {
            Which::Listed(id) => set.cell(*id),
            Which::RunTime(cell) => cell,
        }
    }

    fn same(&self, other: &Which) -> bool {
        match (self, other) {
            (Which::Listed(a), Which::Listed(b)) => a == b,
            (Which::RunTime(a), Which::RunTime(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    fn id(&self) -> Option<ParamId> {
        match self {
            Which::Listed(id) => Some(*id),
            Which::RunTime(_) => None,
        }
    }
}

/// One parameter's gesture state, per parameter rather than per token,
/// since tokens nest.
#[derive(Debug, Default)]
struct ParamGesture {
    /// Open tokens.
    count: Cell<u32>,
    /// The value the UI side shows while the gesture is open: the store's
    /// at the open, then each UI-side write's. Host-shaped writes never
    /// move it, so a user's write compares with it, not the store.
    shown: Cell<f32>,
    /// What [`shown`](Self::shown) held just before the user's first
    /// changing write: the undo step's [`before`](GestureRecord::before).
    user_before: Cell<Option<f32>>,
    /// The step's [`after`](GestureRecord::after).
    last_user: Cell<Option<f32>>,
    /// Whether the host has been told this gesture opened.
    told_host: Cell<bool>,
}

/// A parameter's gesture state, wherever it lives.
enum GestureState<'a> {
    Listed(&'a ParamGesture),
    RunTime(Rc<ParamGesture>),
}

impl Deref for GestureState<'_> {
    type Target = ParamGesture;

    fn deref(&self) -> &ParamGesture {
        match self {
            GestureState::Listed(gesture) => gesture,
            GestureState::RunTime(gesture) => gesture,
        }
    }
}

#[derive(Debug)]
struct UiState {
    set: Arc<ParamSet>,
    clock: RestoreReader,
    gestures: Box<[ParamGesture]>,
    /// The gesture state of each parameter made at run time with a gesture
    /// open, by its store's serial.
    run_time: RefCell<HashMap<u64, Rc<ParamGesture>>>,
    /// User gesture tokens alive, across every parameter.
    user_tokens: Cell<u32>,
    /// Whether a gesture-records drain is alive. Records queue only
    /// while one is, so none pile up with no history to take them.
    drained: Cell<bool>,
    records: RefCell<VecDeque<GestureRecord>>,
    /// Numbered from the same clock as `records`, so a history merges the
    /// two in order.
    run_time_records: RefCell<VecDeque<RunTimeRecord>>,
}

impl Drop for UiState {
    fn drop(&mut self) {
        self.set.release_ui();
    }
}

impl UiState {
    fn gesture(&self, id: ParamId) -> &ParamGesture {
        &self.gestures[id.index()]
    }

    /// The parameter `named` names, or `None` for a store of another set
    /// or another instance: the caller's bug.
    fn find(&self, named: Naming<'_>) -> Option<Which> {
        let which = match named {
            Naming::Id(id) => Some(Which::Listed(id)),
            Naming::Store(cell) if self.set.owns(cell) => Some(Which::Listed(cell.id())),
            Naming::Store(cell) if self.set.made(cell) => Some(Which::RunTime(Arc::clone(cell))),
            Naming::Store(_) => None,
        };
        debug_assert!(
            which.is_some(),
            "a parameter of another set or instance reached this UiParams"
        );
        which
    }

    /// The gesture state of `which`, or `None` for a parameter made at run
    /// time with no gesture open.
    fn state_of(&self, which: &Which) -> Option<GestureState<'_>> {
        match which {
            Which::Listed(id) => Some(GestureState::Listed(self.gesture(*id))),
            Which::RunTime(cell) => {
                let open = self.run_time.try_borrow().ok()?;
                open.get(&cell.serial())
                    .map(|gesture| GestureState::RunTime(Rc::clone(gesture)))
            }
        }
    }

    /// The value shown while a gesture is open on `which`.
    fn shown(&self, which: &Which) -> Option<f32> {
        let gesture = self.state_of(which)?;
        (gesture.count.get() > 0).then(|| gesture.shown.get())
    }

    /// The snapped value, if writing `value` would change what this writer
    /// compares against: the shown value inside a gesture, the store
    /// otherwise.
    fn change(&self, which: &Which, value: f32) -> Option<f32> {
        let cell = which.cell(&self.set);
        let snapped = cell.decl().edit_value(value)?;
        let current = self.shown(which).unwrap_or_else(|| cell.value());
        (snapped != current).then_some(snapped)
    }

    fn open(self: &Rc<Self>, which: Which, writer: Writer) -> Gesture {
        let gesture = match &which {
            Which::Listed(id) => GestureState::Listed(self.gesture(*id)),
            Which::RunTime(cell) => GestureState::RunTime(Rc::clone(
                self.run_time.borrow_mut().entry(cell.serial()).or_default(),
            )),
        };
        let count = gesture.count.get();
        if count == 0 {
            gesture.shown.set(which.cell(&self.set).value());
            gesture.user_before.set(None);
            gesture.last_user.set(None);
            gesture.told_host.set(false);
        }
        gesture.count.set(count.saturating_add(1));
        if writer == Writer::User {
            self.user_tokens
                .set(self.user_tokens.get().saturating_add(1));
            // A user's touch is queued for the host when it opens, not at its
            // first write, so Touch and Latch modes hold the value from then
            // on.
            self.tell_host_begin(&which, &gesture);
        }
        drop(gesture);
        Gesture {
            state: Rc::clone(self),
            which: Some(which),
            writer,
        }
    }

    /// Tells the host a gesture opened on a listed parameter, once.
    fn tell_host_begin(&self, which: &Which, gesture: &ParamGesture) {
        if let Which::Listed(id) = which
            && !gesture.told_host.replace(true)
        {
            self.set.host().begin(*id);
        }
    }

    /// Writes a changing, snapped `value` inside an open gesture, telling the
    /// host between the store and the publish.
    ///
    /// A program edit or replay tells the host of its gesture here, at its
    /// first changing write. A parameter made at run time is listed to no
    /// host, so the host hears nothing of it.
    fn write(&self, which: &Which, value: f32, writer: Writer) {
        debug_assert!(
            writer == Writer::Replay || !self.clock.applying(),
            "a user or edit write while a restore applies; use ProgramParams::sync"
        );
        let Some(gesture) = self.state_of(which) else {
            // Every caller opened a gesture first.
            return;
        };
        let cell = which.cell(&self.set);
        cell.store_then_publish(value, writer.origin(), || {
            if let Which::Listed(id) = which {
                self.tell_host_begin(which, &gesture);
                self.set.host().value(*id, value);
            }
        });
        if writer == Writer::User {
            if gesture.user_before.get().is_none() {
                gesture.user_before.set(Some(gesture.shown.get()));
            }
            gesture.last_user.set(Some(value));
        }
        gesture.shown.set(value);
    }

    /// Never panics: it runs when a token drops, during unwinding too.
    fn close(&self, which: &Which, writer: Writer) {
        if writer == Writer::User {
            self.user_tokens
                .set(self.user_tokens.get().saturating_sub(1));
        }
        let Some(gesture) = self.state_of(which) else {
            return;
        };
        let count = gesture.count.get();
        if count == 0 {
            return;
        }
        gesture.count.set(count - 1);
        if count > 1 {
            return;
        }
        if gesture.told_host.replace(false)
            && let Which::Listed(id) = which
        {
            self.set.host().end(*id);
        }
        if let Which::RunTime(cell) = which
            && let Ok(mut open) = self.run_time.try_borrow_mut()
        {
            open.remove(&cell.serial());
        }
        // The close writes nothing. Whatever the store holds stands, a
        // restore or host write that landed during the drag included. So
        // enrolling a parameter in undo never changes how it behaves.
        let (Some(before), Some(after)) = (gesture.user_before.take(), gesture.last_user.take())
        else {
            return;
        };
        if after == before || !self.drained.get() {
            return;
        }
        let (stamp, sequence) = (self.clock.stamp(), self.clock.sequence());
        match which {
            Which::Listed(id) => {
                if let Ok(mut records) = self.records.try_borrow_mut() {
                    records.push_back(GestureRecord {
                        id: *id,
                        before,
                        after,
                        stamp,
                        sequence,
                    });
                }
            }
            Which::RunTime(cell) => {
                if let Ok(mut records) = self.run_time_records.try_borrow_mut() {
                    records.push_back(RunTimeRecord {
                        param: ParamRef(Arc::clone(cell)),
                        before,
                        after,
                        stamp,
                        sequence,
                    });
                }
            }
        }
    }
}

/// The UI thread's handle to the parameters. Clones share one gesture
/// state; it stays on the thread that made it.
///
/// Every write here snaps, does nothing when it would not change the value,
/// and otherwise stores, tells the host and publishes. A call that names one
/// parameter takes a typed parameter, by reference, or a [`ParamId`], as
/// [`Writable`] says. A typed parameter may be one the set lists or one the
/// instance made at run time, which the host never hears of.
///
/// # Examples
///
/// A drag is one [`Gesture`]: the host hears one begin and one end, however
/// many writes it holds.
///
/// ```
/// use seam_params::{HostEvent, Kind, ParamDecl, ParamSet};
///
/// const GAIN: ParamDecl =
///     ParamDecl::new("gain", "Gain", Kind::continuous(-60.0, 12.0, 0.1, None), 0.0)
///         .with_unit("dB");
/// let set = ParamSet::from_decls([GAIN])?;
/// let gain = set.id("gain").expect("declared above");
///
/// let ui = set.take_ui().expect("the first UI handle");
/// let mut drain = set.take_host_event_drain().expect("the first drain");
/// {
///     let drag = ui.begin(gain);
///     drag.write(-6.0);
///     drag.write(-12.0);
/// } // Dropping the token ends the gesture.
///
/// assert_eq!(ui.value(gain), -12.0);
/// let mut heard = Vec::new();
/// drain.drain(|event| heard.push(event));
/// assert_eq!(heard.len(), 3); // begin, the latest value, end
/// assert_eq!(heard[0], HostEvent::Begin(gain));
/// # Ok::<(), seam_params::DeclError>(())
/// ```
#[derive(Debug, Clone)]
pub struct UiParams {
    state: Rc<UiState>,
}

impl UiParams {
    pub(crate) fn new(set: Arc<ParamSet>, clock: RestoreReader) -> Self {
        let gestures = (0..set.len()).map(|_| ParamGesture::default()).collect();
        Self {
            state: Rc::new(UiState {
                set,
                clock,
                gestures,
                run_time: RefCell::new(HashMap::new()),
                user_tokens: Cell::new(0),
                drained: Cell::new(false),
                records: RefCell::new(VecDeque::new()),
                run_time_records: RefCell::new(VecDeque::new()),
            }),
        }
    }

    /// Returns the set these writes reach.
    pub fn set(&self) -> &ParamSet {
        &self.state.set
    }

    /// Returns the clock it stamps and numbers gesture records with.
    #[doc(hidden)]
    pub fn clock(&self) -> &RestoreReader {
        &self.state.clock
    }

    /// Returns the current value, in real units, as [`ParamSet::value`] does.
    pub fn value(&self, id: ParamId) -> f32 {
        self.state.set.value(id)
    }

    /// Returns whether any gesture is open on the parameter: a user's drag or
    /// the plugin's own [`Edit`]. [`Gesture`] shows it.
    ///
    /// # Panics
    ///
    /// In a debug build, for a typed parameter of another instance.
    pub fn is_open<P: Writable>(&self, param: P) -> bool {
        let Some(which) = self.state.find(param.named(Token(())).0) else {
            return false;
        };
        self.state.shown(&which).is_some()
    }

    /// Returns the value surfaces show while a gesture is open on the
    /// parameter: the last UI-side write's, never a host-shaped write's. It
    /// is of the parameter's type, as [`write`](Self::write) takes.
    ///
    /// `None` when no gesture is open and surfaces show the store. A
    /// [`Watch`](crate::Watch) polls it for you.
    ///
    /// # Panics
    ///
    /// In a debug build, for a typed parameter of another instance.
    ///
    /// # Examples
    ///
    /// ```
    /// # use seam_params as sp;
    /// # let set = sp::ParamSet::from_decls([
    /// #     sp::ParamDecl::new("gain", "Gain", sp::Kind::continuous(-60.0, 12.0, 0.1, None), 0.0),
    /// # ])?;
    /// # let gain = set.id("gain").expect("declared");
    /// # let ui = set.take_ui().expect("the first UI handle");
    ///
    /// assert_eq!(ui.shown(gain), None); // no gesture: surfaces show the store
    ///
    /// let drag = ui.begin(gain);
    /// drag.write(-6.0);
    /// set.audio().host_write(gain, 1.0); // automation reaches the store only
    /// assert_eq!(ui.shown(gain), Some(-6.0));
    /// assert_eq!(ui.value(gain), 12.0);
    ///
    /// drop(drag);
    /// assert_eq!(ui.shown(gain), None);
    /// # Ok::<(), seam_params::DeclError>(())
    /// ```
    pub fn shown<P: Writable>(&self, param: P) -> Option<P::Value> {
        let which = self.state.find(param.named(Token(())).0)?;
        let shown = self.state.shown(&which)?;
        Some(P::decode(shown, Token(())))
    }

    /// Returns whether a user gesture is open on any parameter. Undo refuses
    /// to replay while one is. The plugin's own [`Edit`] does not count.
    ///
    /// # Examples
    ///
    /// ```
    /// # use seam_params as sp;
    /// # let set = sp::ParamSet::from_decls([
    /// #     sp::ParamDecl::new("gain", "Gain", sp::Kind::continuous(-60.0, 12.0, 0.1, None), 0.0),
    /// # ])?;
    /// # let gain = set.id("gain").expect("declared");
    /// # let ui = set.take_ui().expect("the first UI handle");
    ///
    /// assert!(!ui.user_gesture_open());
    /// let drag = ui.begin(gain);
    /// assert!(ui.user_gesture_open());
    /// drop(drag);
    ///
    /// let mut edit = ui.edit();
    /// edit.write(gain, -3.0);
    /// assert!(ui.is_open(gain)); // a gesture, but not a user's
    /// assert!(!ui.user_gesture_open());
    /// # Ok::<(), seam_params::DeclError>(())
    /// ```
    pub fn user_gesture_open(&self) -> bool {
        self.state.user_tokens.get() > 0
    }

    /// Returns whether a user write of `value` would change the parameter:
    /// the same check the write makes.
    ///
    /// A control asks it before opening a gesture for a click, so a click that
    /// changes nothing opens none. Inside a gesture, `value` is compared with
    /// the value the gesture shows, not the store.
    ///
    /// # Panics
    ///
    /// In a debug build, for a typed parameter of another instance.
    ///
    /// # Examples
    ///
    /// ```
    /// # use seam_params as sp;
    /// # let set = sp::ParamSet::from_decls([
    /// #     sp::ParamDecl::new("gain", "Gain", sp::Kind::continuous(-60.0, 12.0, 0.1, None), 0.0),
    /// # ])?;
    /// # let gain = set.id("gain").expect("declared");
    /// # let ui = set.take_ui().expect("the first UI handle");
    ///
    /// assert!(ui.would_change(gain, -6.0));
    /// assert!(!ui.would_change(gain, 0.04)); // snaps to the 0 it already holds
    /// assert!(!ui.would_change(gain, f32::NAN)); // refused
    /// # Ok::<(), seam_params::DeclError>(())
    /// ```
    pub fn would_change<P: Writable>(&self, param: P, value: P::Value) -> bool {
        let Some(which) = self.state.find(param.named(Token(())).0) else {
            return false;
        };
        let value = P::encode(value, Token(()));
        self.state.change(&which, value).is_some()
    }

    /// Opens a user gesture on the parameter, or joins the one open.
    ///
    /// Its begin is queued now, at the touch, ahead of any value it writes;
    /// the host hears it when the exporter next drains the queue
    /// ([`HostEventDrain`](crate::HostEventDrain)), after the plugin's next
    /// idle. One end follows when the last token closes, however many
    /// gestures overlap. A gesture on a parameter made at run time tells the
    /// host nothing. [`UiParams`] shows a drag.
    ///
    /// The gesture writes the value as a store holds it, an `f32` whatever
    /// the parameter's kind: real units for a float, 0 or 1 for a bool, the
    /// index for a choice.
    ///
    /// # Panics
    ///
    /// In a debug build, for a typed parameter of another instance. A
    /// release build hands back a gesture that writes nothing.
    pub fn begin<P: Writable>(&self, param: P) -> Gesture {
        match self.state.find(param.named(Token(())).0) {
            Some(which) => self.state.open(which, Writer::User),
            None => Gesture {
                state: Rc::clone(&self.state),
                which: None,
                writer: Writer::User,
            },
        }
    }

    /// Writes `value` as a lone user write: a complete gesture of one value,
    /// opened only when it changes the value. Returns whether it did; a value
    /// that is not finite is refused.
    ///
    /// `param` is a typed parameter, by reference, or a [`ParamId`], and
    /// `value` is of its type: a `bool` for a [`Bool`](crate::Bool), the
    /// plugin's enum for a [`Choice`](crate::Choice), real units for a
    /// [`Float`](crate::Float) or a [`ParamId`]. A typed parameter of
    /// another instance is refused.
    ///
    /// # Panics
    ///
    /// In a debug build, when it changes a value while a restore applies, as
    /// in a session participant's apply, which writes through
    /// [`ProgramParams::sync`](crate::ProgramParams::sync) instead; and when
    /// `param` is a typed parameter of another instance.
    ///
    /// # Examples
    ///
    /// ```
    /// # use seam_params as sp;
    /// # let set = sp::ParamSet::from_decls([
    /// #     sp::ParamDecl::new("gain", "Gain", sp::Kind::continuous(-60.0, 12.0, 0.1, None), 0.0),
    /// # ])?;
    /// # let gain = set.id("gain").expect("declared");
    /// # let ui = set.take_ui().expect("the first UI handle");
    ///
    /// assert!(ui.write(gain, -6.04)); // a click on the control: it snaps to a step
    /// assert_eq!(ui.value(gain), -6.0);
    /// assert!(!ui.write(gain, -6.0)); // no change, so no gesture
    /// assert!(!ui.write(gain, f32::NAN)); // refused
    /// # Ok::<(), seam_params::DeclError>(())
    /// ```
    pub fn write<P: Writable>(&self, param: P, value: P::Value) -> bool {
        let Some(which) = self.state.find(param.named(Token(())).0) else {
            return false;
        };
        self.gesture_write(which, P::encode(value, Token(())), Writer::User)
    }

    /// Starts the plugin's own edit: values changed together reach the host
    /// as one gesture per parameter, shared with any gesture already open
    /// on it. It records nothing. [`Edit`] shows one.
    pub fn edit(&self) -> Edit {
        Edit {
            state: Rc::clone(&self.state),
            gestures: Vec::new(),
        }
    }

    /// Writes `value` as an undo replay: one complete host gesture when it
    /// changes the value, and never a record. Undo makes none while a user
    /// gesture is open. Returns whether it changed the value.
    ///
    /// # Examples
    ///
    /// ```
    /// # use seam_params as sp;
    /// # let set = sp::ParamSet::from_decls([
    /// #     sp::ParamDecl::new("gain", "Gain", sp::Kind::continuous(-60.0, 12.0, 0.1, None), 0.0),
    /// # ])?;
    /// # let gain = set.id("gain").expect("declared");
    /// # let ui = set.take_ui().expect("the first UI handle");
    ///
    /// use seam_params::Origin;
    ///
    /// assert!(ui.replay(gain, -6.0));
    /// assert_eq!(set.read(gain).origin, Origin::Replay);
    /// assert!(!ui.replay(gain, -6.0)); // already there: no gesture
    /// # Ok::<(), seam_params::DeclError>(())
    /// ```
    pub fn replay(&self, id: ParamId, value: f32) -> bool {
        self.gesture_write(Which::Listed(id), value, Writer::Replay)
    }

    /// [`replay`](Self::replay) for a parameter this instance made at run
    /// time: what undo replays one of its steps through.
    #[doc(hidden)]
    pub fn replay_run_time(&self, param: &ParamRef, value: f32) -> bool {
        debug_assert!(
            self.state.set.made(&param.0),
            "a replay reaches only a parameter this instance made at run time"
        );
        self.gesture_write(Which::RunTime(Arc::clone(&param.0)), value, Writer::Replay)
    }

    /// Takes the drain of the user gestures that close with their value
    /// changed: what an undo history takes them from.
    ///
    /// There is one per handle, so two histories cannot take each other's
    /// records; `None` while one is alive. Records queue only while it is.
    #[doc(hidden)]
    pub fn take_gesture_records(&self) -> Option<GestureRecords> {
        if self.state.drained.replace(true) {
            return None;
        }
        Some(GestureRecords {
            state: Rc::clone(&self.state),
        })
    }

    fn gesture_write(&self, which: Which, value: f32, writer: Writer) -> bool {
        let Some(value) = self.state.change(&which, value) else {
            return false;
        };
        let gesture = self.state.open(which, writer);
        if let Some(which) = &gesture.which {
            self.state.write(which, value, writer);
        }
        drop(gesture);
        true
    }
}

/// The one drain of a [`UiParams`]'s gesture records. Dropping it stops
/// them and drops any not taken.
#[doc(hidden)]
#[derive(Debug)]
pub struct GestureRecords {
    state: Rc<UiState>,
}

impl GestureRecords {
    /// Takes the user gestures on listed parameters that closed with their
    /// value changed, oldest first, since the last call.
    pub fn take(&self) -> Vec<GestureRecord> {
        self.state.records.borrow_mut().drain(..).collect()
    }

    /// Takes the gestures on parameters made at run time, as
    /// [`take`](Self::take) does for listed ones. The two share one
    /// numbering, so a history merges them in order.
    pub fn take_run_time(&self) -> Vec<RunTimeRecord> {
        self.state.run_time_records.borrow_mut().drain(..).collect()
    }
}

impl Drop for GestureRecords {
    fn drop(&mut self) {
        if let Ok(mut records) = self.state.records.try_borrow_mut() {
            records.clear();
        }
        // Taken out first, so a record's store drops after the borrow ends.
        let run_time = self
            .state
            .run_time_records
            .try_borrow_mut()
            .map(|mut records| std::mem::take(&mut *records));
        drop(run_time);
        self.state.drained.set(false);
    }
}

/// An open gesture on one parameter: a user's drag, from touch to release.
///
/// Dropping it closes it. It stays on the UI thread.
///
/// # Examples
///
/// ```
/// # use seam_params as sp;
/// # let set = sp::ParamSet::from_decls([
/// #     sp::ParamDecl::new("gain", "Gain", sp::Kind::continuous(-60.0, 12.0, 0.1, None), 0.0),
/// # ])?;
/// # let gain = set.id("gain").expect("declared");
/// # let ui = set.take_ui().expect("the first UI handle");
///
/// // Gestures on one parameter nest: the host hears one begin and one end.
/// let outer = ui.begin(gain);
/// let inner = ui.begin(gain);
/// drop(inner);
/// assert!(ui.is_open(gain));
/// drop(outer);
/// assert!(!ui.is_open(gain));
/// # Ok::<(), seam_params::DeclError>(())
/// ```
#[derive(Debug)]
#[must_use = "a gesture closes as soon as it drops, so its writes would land outside it"]
pub struct Gesture {
    state: Rc<UiState>,
    /// `None` for one a release build handed back for a parameter of
    /// another instance: it writes nothing, and closes nothing.
    which: Option<Which>,
    writer: Writer,
}

impl Gesture {
    /// Returns the listed parameter it is open on, or `None` for a parameter
    /// made at run time, which has no id.
    pub fn id(&self) -> Option<ParamId> {
        self.which.as_ref().and_then(Which::id)
    }

    /// Writes `value` inside the gesture, as a store holds it. It is
    /// compared with the value the gesture shows, so a host write that landed
    /// during the drag cannot make it a no-op. Returns whether it wrote.
    ///
    /// # Panics
    ///
    /// In a debug build, when it changes a value while a restore applies, as
    /// in a session participant's apply, which writes through
    /// [`ProgramParams::sync`](crate::ProgramParams::sync) instead.
    ///
    /// # Examples
    ///
    /// ```
    /// # use seam_params as sp;
    /// # let set = sp::ParamSet::from_decls([
    /// #     sp::ParamDecl::new("gain", "Gain", sp::Kind::continuous(-60.0, 12.0, 0.1, None), 0.0),
    /// # ])?;
    /// # let gain = set.id("gain").expect("declared");
    /// # let ui = set.take_ui().expect("the first UI handle");
    ///
    /// let drag = ui.begin(gain);
    /// assert!(drag.write(-6.0));
    /// assert!(!drag.write(-6.0)); // no change, nothing written
    ///
    /// // Automation stores -3 dB during the drag. The drag's own -3 still writes:
    /// // it is compared with the -6 the drag shows, not with the store.
    /// set.audio().host_write(gain, 57.0 / 72.0);
    /// assert_eq!(set.value(gain), -3.0);
    /// assert!(drag.write(-3.0));
    /// # Ok::<(), seam_params::DeclError>(())
    /// ```
    pub fn write(&self, value: f32) -> bool {
        let Some(which) = &self.which else {
            return false;
        };
        let Some(value) = self.state.change(which, value) else {
            return false;
        };
        self.state.write(which, value, self.writer);
        true
    }
}

impl Drop for Gesture {
    fn drop(&mut self) {
        if let Some(which) = &self.which {
            self.state.close(which, self.writer);
        }
    }
}

/// The plugin's own edit, open until dropped.
///
/// Each parameter it changes opens a gesture at its first changing write;
/// dropping the edit closes them all. Drop it within the call that made it,
/// so surfaces, which hold host-shaped writes back while a gesture is open,
/// are not held back across ticks.
///
/// # Examples
///
/// ```
/// # use seam_params as sp;
/// # let set = sp::ParamSet::from_decls([
/// #     sp::ParamDecl::new("low", "Low", sp::Kind::continuous(-12.0, 12.0, 0.1, None), 0.0),
/// #     sp::ParamDecl::new("high", "High", sp::Kind::continuous(-12.0, 12.0, 0.1, None), 0.0),
/// # ])?;
/// # let (low, high) = (set.id("low").expect("declared"), set.id("high").expect("declared"));
/// # let ui = set.take_ui().expect("the first UI handle");
///
/// let mut edit = ui.edit();
/// assert!(edit.write(low, -3.0));
/// assert!(ui.is_open(low)); // open until the edit drops
/// drop(edit);
/// assert!(!ui.is_open(low));
/// # Ok::<(), seam_params::DeclError>(())
/// ```
#[derive(Debug)]
#[must_use = "an edit's gestures close as soon as it drops"]
pub struct Edit {
    state: Rc<UiState>,
    gestures: Vec<Gesture>,
}

impl Edit {
    /// Writes `value`, of the parameter's type as [`UiParams::write`] takes
    /// it. Returns whether it changed the value; a value that is not finite
    /// is refused, and so is a typed parameter of another instance.
    /// [`Edit`] shows one.
    ///
    /// # Panics
    ///
    /// In a debug build, when it changes a value while a restore applies, as
    /// in a session participant's apply, which writes through
    /// [`ProgramParams::sync`](crate::ProgramParams::sync) instead; and when
    /// `param` is a typed parameter of another instance.
    pub fn write<P: Writable>(&mut self, param: P, value: P::Value) -> bool {
        let Some(which) = self.state.find(param.named(Token(())).0) else {
            return false;
        };
        let Some(value) = self.state.change(&which, P::encode(value, Token(()))) else {
            return false;
        };
        let open = self
            .gestures
            .iter()
            .any(|gesture| gesture.which.as_ref().is_some_and(|open| open.same(&which)));
        if !open {
            self.gestures
                .push(self.state.open(which.clone(), Writer::Program));
        }
        self.state.write(&which, value, Writer::Program);
        true
    }
}
