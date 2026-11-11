//! The UI thread's writes: user edits through gesture tokens, the plugin's
//! own edits, and undo replays, all sharing one gesture count per parameter.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;

use crate::param::sealed::Token;
use crate::set::ParamCell;
use crate::{GestureRecord, Origin, ParamId, ParamSet, RestoreReader, Writable};

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

#[derive(Debug)]
struct UiState {
    set: Arc<ParamSet>,
    clock: RestoreReader,
    gestures: Box<[ParamGesture]>,
    /// User gesture tokens alive, across every parameter.
    user_tokens: Cell<u32>,
    /// Whether a gesture-records drain is alive. Records queue only
    /// while one is, so none pile up with no history to take them.
    drained: Cell<bool>,
    records: RefCell<VecDeque<GestureRecord>>,
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

    /// The snapped value, if writing `value` would change what this writer
    /// compares against: the shown value inside a gesture, the store
    /// otherwise.
    fn change(&self, id: ParamId, value: f32) -> Option<f32> {
        let snapped = self.set.decl(id).edit_value(value)?;
        let gesture = self.gesture(id);
        let current = if gesture.count.get() > 0 {
            gesture.shown.get()
        } else {
            self.set.value(id)
        };
        (snapped != current).then_some(snapped)
    }

    fn open(self: &Rc<Self>, id: ParamId, writer: Writer) -> Gesture {
        let gesture = self.gesture(id);
        let count = gesture.count.get();
        if count == 0 {
            gesture.shown.set(self.set.value(id));
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
            self.tell_host_begin(id);
        }
        Gesture {
            state: Rc::clone(self),
            id,
            writer,
        }
    }

    fn tell_host_begin(&self, id: ParamId) {
        if !self.gesture(id).told_host.replace(true) {
            self.set.host().begin(id);
        }
    }

    /// Writes a changing, snapped `value` inside an open gesture, telling the
    /// host between the store and the publish.
    ///
    /// A program edit or replay tells the host of its gesture here, at its
    /// first changing write.
    fn write(&self, id: ParamId, value: f32, writer: Writer) {
        debug_assert!(
            writer == Writer::Replay || !self.clock.applying(),
            "a user or edit write while a restore applies; use ProgramParams::sync"
        );
        let gesture = self.gesture(id);
        self.set.store_then_publish(id, value, writer.origin(), || {
            self.tell_host_begin(id);
            self.set.host().value(id, value);
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
    fn close(&self, id: ParamId, writer: Writer) {
        if writer == Writer::User {
            self.user_tokens
                .set(self.user_tokens.get().saturating_sub(1));
        }
        let gesture = self.gesture(id);
        let count = gesture.count.get();
        if count == 0 {
            return;
        }
        gesture.count.set(count - 1);
        if count > 1 {
            return;
        }
        if gesture.told_host.replace(false) {
            self.set.host().end(id);
        }
        // The close writes nothing. Whatever the store holds stands, a
        // restore or host write that landed during the drag included. So
        // enrolling a parameter in undo never changes how it behaves.
        if let (Some(before), Some(after)) = (gesture.user_before.take(), gesture.last_user.take())
            && after != before
            && self.drained.get()
            && let Ok(mut records) = self.records.try_borrow_mut()
        {
            records.push_back(GestureRecord {
                id,
                before,
                after,
                stamp: self.clock.stamp(),
                sequence: self.clock.sequence(),
            });
        }
    }
}

/// The UI thread's handle to the parameters. Clones share one gesture
/// state; it stays on the thread that made it.
///
/// Every write here snaps, does nothing when it would not change the value,
/// and otherwise stores, tells the host and publishes.
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
                user_tokens: Cell::new(0),
                drained: Cell::new(false),
                records: RefCell::new(VecDeque::new()),
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
    pub fn is_open(&self, id: ParamId) -> bool {
        self.state.gesture(id).count.get() > 0
    }

    /// Returns the value surfaces show while a gesture is open on the
    /// parameter: the last UI-side write's, never a host-shaped write's.
    ///
    /// `None` when no gesture is open and surfaces show the store. A
    /// [`Watch`](crate::Watch) polls it for you.
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
    pub fn shown(&self, id: ParamId) -> Option<f32> {
        let gesture = self.state.gesture(id);
        (gesture.count.get() > 0).then(|| gesture.shown.get())
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
    pub fn would_change(&self, id: ParamId, value: f32) -> bool {
        self.state.change(id, value).is_some()
    }

    /// Opens a user gesture on the parameter, or joins the one open.
    ///
    /// Its begin is queued now, at the touch, ahead of any value it writes;
    /// the host hears it when the exporter next drains the queue
    /// ([`HostEventDrain`](crate::HostEventDrain)), after the plugin's next
    /// idle. One end follows when the last token closes, however many
    /// gestures overlap. [`UiParams`] shows a drag.
    pub fn begin(&self, id: ParamId) -> Gesture {
        self.state.open(id, Writer::User)
    }

    /// Writes `value` as a lone user write: a complete gesture of one value,
    /// opened only when it changes the value. Returns whether it did; a value
    /// that is not finite is refused.
    ///
    /// `param` is a typed parameter, by reference, or a [`ParamId`], and
    /// `value` is of its type: a `bool` for a [`Bool`](crate::Bool), the
    /// plugin's enum for a [`Choice`](crate::Choice), real units for a
    /// [`Float`](crate::Float) or a [`ParamId`]. A typed parameter of
    /// another set is refused.
    ///
    /// # Panics
    ///
    /// In a debug build, when it changes a value while a restore applies, as
    /// in a session participant's apply, which writes through
    /// [`ProgramParams::sync`](crate::ProgramParams::sync) instead; and when
    /// `param` is a typed parameter of another set.
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
        param.write_through(self, value, Token(()))
    }

    /// [`write`](Self::write) for a [`ParamId`], with the stored `f32`.
    pub(crate) fn user_write(&self, id: ParamId, value: f32) -> bool {
        self.gesture_write(id, value, Writer::User)
    }

    /// [`write`](Self::write) for a typed parameter, with the `f32` its
    /// value encodes to: refused when the store is another set's.
    pub(crate) fn user_write_cell(&self, cell: &Arc<ParamCell>, value: f32) -> bool {
        let ours = self.state.set.owns(cell);
        debug_assert!(
            ours,
            "a parameter of another set was written through this UiParams"
        );
        ours && self.gesture_write(cell.id(), value, Writer::User)
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
        self.gesture_write(id, value, Writer::Replay)
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

    fn gesture_write(&self, id: ParamId, value: f32, writer: Writer) -> bool {
        let Some(value) = self.state.change(id, value) else {
            return false;
        };
        let gesture = self.state.open(id, writer);
        self.state.write(id, value, writer);
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
    /// Takes the user gestures that closed with their value changed, oldest
    /// first, since the last call.
    pub fn take(&self) -> Vec<GestureRecord> {
        self.state.records.borrow_mut().drain(..).collect()
    }
}

impl Drop for GestureRecords {
    fn drop(&mut self) {
        if let Ok(mut records) = self.state.records.try_borrow_mut() {
            records.clear();
        }
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
    id: ParamId,
    writer: Writer,
}

impl Gesture {
    /// Returns the parameter it is open on.
    pub fn id(&self) -> ParamId {
        self.id
    }

    /// Writes `value` inside the gesture. It is compared with the value the
    /// gesture shows, so a host write that landed during the drag cannot
    /// make it a no-op. Returns whether it wrote.
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
        let Some(value) = self.state.change(self.id, value) else {
            return false;
        };
        self.state.write(self.id, value, self.writer);
        true
    }
}

impl Drop for Gesture {
    fn drop(&mut self) {
        self.state.close(self.id, self.writer);
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
    /// Writes `value`, in real units. Returns whether it changed the value;
    /// a value that is not finite is refused. [`Edit`] shows one.
    ///
    /// # Panics
    ///
    /// In a debug build, when it changes a value while a restore applies, as
    /// in a session participant's apply, which writes through
    /// [`ProgramParams::sync`](crate::ProgramParams::sync) instead.
    pub fn write(&mut self, id: ParamId, value: f32) -> bool {
        let Some(value) = self.state.change(id, value) else {
            return false;
        };
        if !self.gestures.iter().any(|gesture| gesture.id == id) {
            self.gestures.push(self.state.open(id, Writer::Program));
        }
        self.state.write(id, value, Writer::Program);
        true
    }
}
