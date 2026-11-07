//! The history: its two stacks of entries, the bound, and the replay walk.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fmt;
use std::mem;
use std::rc::Rc;

use seam_params::{GestureRecords, RestoreReader, RestoreStamp, UiParams};

use crate::enroll::{Enrolment, Polled};
use crate::{Bound, Entry, HistoryError};

/// What a step books on top of its stored values.
pub(crate) const STEP_OVERHEAD: usize = 32;

/// What a step keeping `bytes` alive books, once, when recorded.
pub(crate) fn booked(bytes: usize) -> usize {
    bytes + STEP_OVERHEAD
}

/// One recorded change, which replays itself either way. Replaying never
/// records.
pub(crate) trait Step: fmt::Debug {
    fn undo(&mut self);
    fn redo(&mut self);
    /// Its booked size, fixed when it is recorded.
    fn units(&self) -> usize;
    /// Whether it writes parameters, the one kind of write a load makes, so
    /// its replay holds loads off.
    ///
    /// Nothing else may run under that hold, so it stays a leaf lock. A
    /// value's or list's step takes its own lock, which a thread loading
    /// inside that lock would take in the other order.
    fn writes_parameters(&self) -> bool {
        false
    }
}

#[derive(Debug)]
struct Recorded {
    name: String,
    steps: Vec<Box<dyn Step>>,
    units: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Replaying,
}

#[derive(Debug)]
struct State {
    /// The applied entries, oldest first, so pruning drops from the front.
    applied: VecDeque<Recorded>,
    /// The undone entries, the next to redo last.
    undone: Vec<Recorded>,
    /// The booked size of every entry.
    units: usize,
    bound: Bound,
    phase: Phase,
    /// Moves when the listing visibly changes.
    version: u64,
    scope: Option<Scope>,
    /// The restore clock's counts the history last saw. When they move, the
    /// history clears.
    seen: RestoreStamp,
}

/// An open transaction: its depth lives on the history, so every path that
/// records joins the one outermost scope.
#[derive(Debug)]
struct Scope {
    /// The outermost transaction's name.
    name: String,
    depth: usize,
    /// Whether the newest applied entry is the scope's and new steps join it.
    /// An undo or redo inside the scope ends it, so later steps form a new
    /// entry, which keeps the scope's name.
    joining: bool,
}

#[derive(Debug)]
pub(crate) struct Shared {
    state: RefCell<State>,
    pub(crate) ui: UiParams,
    /// The UI handle's one record drain, so no other history takes its
    /// records.
    pub(crate) records: GestureRecords,
    clock: RestoreReader,
    /// Per parameter index, its enrolment while enrolled.
    pub(crate) params: RefCell<Vec<Option<Enrolment>>>,
    pub(crate) values: RefCell<Vec<Box<dyn Polled>>>,
}

/// The undo history of one plugin instance. Clones share it, so an editor,
/// its menus and its shortcuts can each hold one.
///
/// Each call that reads or changes the listing first clears it if a restore
/// was stored or completed since the history last looked. Recording a new
/// entry drops the undone ones. A replay (undo, redo, jump) applies its steps
/// with nothing borrowed and refuses every other request with
/// [`HistoryError::Replaying`] until it ends; one that panics leaves the
/// history empty rather than keep steps and values that disagree.
///
/// Thread: UI. A history is neither `Send` nor `Sync`.
///
/// # Examples
///
/// ```
/// use seam_params::{Kind, ParamDecl, ParamSet};
/// use seam_undo::{Bound, History};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let set = ParamSet::from_decls([
///     ParamDecl::new("gain", "Gain", Kind::continuous(0.0, 10.0, 1.0, None), 5.0)
///         .with_unit("dB"),
/// ])?;
/// let gain = set.id("gain").ok_or("declared")?;
/// let ui = set.take_ui().ok_or("the first UI handle")?;
///
/// let history = History::new(&ui, Bound::default())?;
/// history.enroll_param(gain)?;
///
/// // One user gesture is one step.
/// ui.write(gain, 8.0);
/// history.tick();
/// assert_eq!(history.len(), 1);
///
/// assert_eq!(history.undo(), Ok(true));
/// assert_eq!(set.value(gain), 5.0);
/// assert_eq!(history.redo(), Ok(true));
/// assert_eq!(set.value(gain), 8.0);
/// # Ok(())
/// # }
/// ```
///
/// # Panics
///
/// In a build with `debug_assertions`, a replay that writes parameters
/// ([`undo`](Self::undo), [`redo`](Self::redo) and [`jump`](Self::jump))
/// panics while the thread holds an
/// [`audio_thread`](seam_params::audio_thread) mark.
#[derive(Debug, Clone)]
pub struct History {
    pub(crate) shared: Rc<Shared>,
}

impl History {
    /// Creates a history over `ui`'s parameters, pruned to `bound`.
    ///
    /// # Errors
    ///
    /// Returns [`HistoryError::HistoryExists`] while another history over `ui`
    /// is alive: make one and clone it.
    pub fn new(ui: &UiParams, bound: Bound) -> Result<History, HistoryError> {
        let records = ui
            .take_gesture_records()
            .ok_or(HistoryError::HistoryExists)?;
        let clock = ui.clock().clone();
        Ok(History {
            shared: Rc::new(Shared {
                state: RefCell::new(State {
                    applied: VecDeque::new(),
                    undone: Vec::new(),
                    units: 0,
                    bound,
                    phase: Phase::Idle,
                    version: 0,
                    scope: None,
                    seen: clock.stamp(),
                }),
                ui: ui.clone(),
                records,
                clock,
                params: RefCell::new(vec![None; ui.set().len()]),
                values: RefCell::new(Vec::new()),
            }),
        })
    }

    /// The listing: applied entries oldest first, then undone ones in redo
    /// order, as the [`Entry`] example shows.
    pub fn entries(&self) -> Vec<Entry> {
        self.observe_clock();
        let state = self.shared.state.borrow();
        let applied = state.applied.iter().map(|entry| (entry, false));
        let undone = state.undone.iter().rev().map(|entry| (entry, true));
        applied
            .chain(undone)
            .map(|(entry, undone)| Entry {
                name: entry.name.clone(),
                undone,
            })
            .collect()
    }

    /// Returns how many entries are applied: the index of the first undone one.
    pub fn cursor(&self) -> usize {
        self.observe_clock();
        self.shared.state.borrow().applied.len()
    }

    /// Returns how many entries there are, applied and undone.
    pub fn len(&self) -> usize {
        self.observe_clock();
        self.shared.state.borrow().len()
    }

    /// Returns whether there are no entries.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns a counter that moves only when [`entries`](Self::entries) would
    /// read differently, so a history panel redraws only then.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_undo::{Bound, History};
    #[doc = include_str!("../doc/setup.rs")]
    /// # let history = History::new(&ui, Bound::default())?;
    /// history.enroll_param(gain)?;
    /// let drawn = history.version();
    ///
    /// // Nothing changed, so the panel keeps what it drew.
    /// history.tick();
    /// assert_eq!(history.version(), drawn);
    ///
    /// ui.write(gain, 8.0);
    /// history.tick();
    /// assert_ne!(history.version(), drawn);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn version(&self) -> u64 {
        self.observe_clock();
        self.shared.state.borrow().version
    }

    /// Returns the bound the history prunes to.
    pub fn bound(&self) -> Bound {
        self.shared.state.borrow().bound
    }

    /// Undoes the newest applied entry.
    ///
    /// Returns `Ok(false)` when there is none, or when a restore stopped it.
    /// A stopped walk clears the history, and the steps it applied before the
    /// stop stand.
    ///
    /// # Errors
    ///
    /// Returns [`HistoryError::Replaying`] during another replay, and
    /// [`HistoryError::GestureOpen`] while a user gesture is open.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_undo::{Bound, History, HistoryError};
    #[doc = include_str!("../doc/setup.rs")]
    /// # let history = History::new(&ui, Bound::default())?;
    /// history.enroll_param(gain)?;
    /// assert_eq!(history.undo(), Ok(false)); // nothing recorded yet
    ///
    /// ui.write(gain, 8.0);
    /// // A replay never runs inside a drag.
    /// let drag = ui.begin(gain);
    /// assert_eq!(history.undo(), Err(HistoryError::GestureOpen));
    /// drop(drag);
    ///
    /// assert_eq!(history.undo(), Ok(true));
    /// assert_eq!(set.value(gain), 5.0);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn undo(&self) -> Result<bool, HistoryError> {
        self.ready_to_replay()?;
        Ok(self.walk(Direction::Undo, 1))
    }

    /// Redoes the oldest undone entry. Returns `Ok(false)` when there is none,
    /// or when a restore stopped it, as for [`undo`](Self::undo).
    ///
    /// # Errors
    ///
    /// As for [`undo`](Self::undo).
    pub fn redo(&self) -> Result<bool, HistoryError> {
        self.ready_to_replay()?;
        Ok(self.walk(Direction::Redo, 1))
    }

    /// Undoes or redoes until exactly `index` entries are applied.
    ///
    /// A jump replays one entry at a time and moves [`version`](Self::version)
    /// once. Returns `Ok(false)` when `index` entries are applied already, or
    /// when a restore stopped the walk, which clears the history as for
    /// [`undo`](Self::undo).
    ///
    /// # Errors
    ///
    /// Returns [`HistoryError::OutOfRange`] when `index` is past the last
    /// entry, and otherwise the errors of [`undo`](Self::undo).
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_undo::{Bound, History};
    #[doc = include_str!("../doc/setup.rs")]
    /// # let history = History::new(&ui, Bound::default())?;
    /// history.enroll_param(gain)?;
    /// for value in [2.0, 4.0, 6.0] {
    ///     ui.write(gain, value);
    ///     history.tick();
    /// }
    ///
    /// // Clicking a row of the history panel goes to that state.
    /// assert_eq!(history.jump(1), Ok(true));
    /// assert_eq!(set.value(gain), 2.0);
    /// assert_eq!(history.jump(3), Ok(true));
    /// assert_eq!(set.value(gain), 6.0);
    ///
    /// // Clicking the current row replays nothing.
    /// assert_eq!(history.jump(3), Ok(false));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn jump(&self, index: usize) -> Result<bool, HistoryError> {
        self.ready_to_replay()?;
        let (applied, len) = {
            let state = self.shared.state.borrow();
            (state.applied.len(), state.len())
        };
        if index > len {
            return Err(HistoryError::OutOfRange { index, len });
        }
        let direction = if index < applied {
            Direction::Undo
        } else {
            Direction::Redo
        };
        Ok(self.walk(direction, applied.abs_diff(index)))
    }

    /// Drops every entry, the undone ones too. Values stay where the last edit
    /// left them.
    ///
    /// # Errors
    ///
    /// Returns [`HistoryError::Replaying`] during a replay.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_undo::{Bound, History};
    #[doc = include_str!("../doc/setup.rs")]
    /// # let history = History::new(&ui, Bound::default())?;
    /// history.enroll_param(gain)?;
    /// ui.write(gain, 8.0);
    /// history.tick();
    ///
    /// // Nothing to undo afterwards, and the parameter keeps its value.
    /// history.clear()?;
    /// assert!(history.is_empty());
    /// assert_eq!(history.undo(), Ok(false));
    /// assert_eq!(set.value(gain), 8.0);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn clear(&self) -> Result<(), HistoryError> {
        if self.replaying() {
            return Err(HistoryError::Replaying);
        }
        // Takes the writes made before the clear first, so the next tick
        // does not record them as a step after it.
        self.tick();
        self.shared.state.borrow_mut().clear_all();
        Ok(())
    }

    /// Opens a transaction named `name`, or joins the one open: every step
    /// recorded until the outermost one drops is one entry, named by the
    /// outermost.
    ///
    /// Changes made before the outermost opens record first, as their own
    /// entries, and those made inside it record by its close. An empty
    /// transaction leaves no entry, and one dropped by an early return or a
    /// panic keeps what it recorded as one entry.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_undo::{Bound, History};
    #[doc = include_str!("../doc/setup.rs")]
    /// # let history = History::new(&ui, Bound::default())?;
    /// history.enroll_param(gain)?;
    /// {
    ///     let _scope = history.transaction("Reset");
    ///     ui.write(gain, 8.0);
    ///     ui.write(gain, 1.0);
    /// }
    ///
    /// // Two writes, one step, named by the transaction.
    /// assert_eq!(history.len(), 1);
    /// assert_eq!(history.entries()[0].name, "Reset");
    /// history.undo()?;
    /// assert_eq!(set.value(gain), 5.0);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn transaction(&self, name: &str) -> Transaction {
        let outermost = self.shared.state.borrow().scope.is_none();
        if outermost {
            self.tick();
        }
        let mut state = self.shared.state.borrow_mut();
        match &mut state.scope {
            Some(scope) => scope.depth += 1,
            None => {
                state.scope = Some(Scope {
                    name: name.to_owned(),
                    depth: 1,
                    joining: false,
                });
            }
        }
        Transaction {
            history: self.clone(),
        }
    }

    fn close_transaction(&self) {
        let outermost = self
            .shared
            .state
            .borrow()
            .scope
            .as_ref()
            .is_some_and(|scope| scope.depth == 1);
        // Writes are recorded when a tick takes them, so this one runs while
        // the scope is open and the transaction's last writes join its entry.
        if outermost {
            self.tick();
        }
        let mut state = self.shared.state.borrow_mut();
        if let Some(scope) = &mut state.scope {
            scope.depth -= 1;
            if scope.depth == 0 {
                state.scope = None;
            }
        }
    }

    /// Records `steps` as one entry named `name`, or as part of the open
    /// transaction's. Refused while a replay applies.
    pub(crate) fn record(&self, name: &str, steps: Vec<Box<dyn Step>>) -> Result<(), HistoryError> {
        let mut state = self.shared.state.borrow_mut();
        let state = &mut *state;
        if state.phase == Phase::Replaying {
            return Err(HistoryError::Replaying);
        }
        if steps.is_empty() {
            return Ok(());
        }
        let units: usize = steps.iter().map(|step| step.units()).sum();
        state.units += units;
        if state.scope.as_ref().is_some_and(|scope| scope.joining) {
            let entry = state
                .applied
                .back_mut()
                .expect("a joining scope has its entry");
            entry.steps.extend(steps);
            entry.units += units;
            // The listing reads the same unless pruning dropped an entry.
            if state.prune() {
                state.version += 1;
            }
            return Ok(());
        }
        let name = match &mut state.scope {
            Some(scope) => {
                scope.joining = true;
                scope.name.clone()
            }
            None => name.to_owned(),
        };
        let redo = state.drop_undone();
        state.units -= redo;
        state.applied.push_back(Recorded { name, steps, units });
        state.prune();
        state.version += 1;
        Ok(())
    }

    /// Refuses a value or list made with another clock: its sequence
    /// numbers would never fall before this history's cut.
    pub(crate) fn same_clock(&self, clock: &RestoreReader) -> Result<(), HistoryError> {
        if self.shared.clock.ptr_eq(clock) {
            Ok(())
        } else {
            Err(HistoryError::OtherClock)
        }
    }

    pub(crate) fn replaying(&self) -> bool {
        self.shared.state.borrow().phase == Phase::Replaying
    }

    pub(crate) fn seen(&self) -> RestoreStamp {
        self.shared.state.borrow().seen
    }

    /// A number from the clock's sequence: the tick records only changes
    /// numbered before it.
    pub(crate) fn cut(&self) -> u64 {
        self.shared.clock.sequence()
    }

    /// Clears the history if the restore clock moved since it last looked:
    /// once when a restore is stored and again when it completes, so what was
    /// recorded in between drops too. No option keeps history across a
    /// restore, and a walk the clock moves under clears when it ends.
    pub(crate) fn observe_clock(&self) {
        if !self.replaying() {
            let stamp = self.shared.clock.stamp();
            self.shared.state.borrow_mut().observe(stamp);
        }
    }

    /// Refuses a replay inside another, and one inside a user gesture on a
    /// parameter or an enrolled plain value, changing nothing; then records
    /// what changed since the last tick.
    fn ready_to_replay(&self) -> Result<(), HistoryError> {
        if self.replaying() {
            return Err(HistoryError::Replaying);
        }
        let value_gesture = self
            .shared
            .values
            .borrow()
            .iter()
            .any(|value| value.gesture_open());
        if self.shared.ui.user_gesture_open() || value_gesture {
            return Err(HistoryError::GestureOpen);
        }
        self.tick();
        Ok(())
    }

    /// Replays `count` entries in `direction`, one at a time. Returns whether
    /// it replayed any and all of them, and only then moves the version.
    fn walk(&self, direction: Direction, count: usize) -> bool {
        let walked = count > 0 && (0..count).all(|_| self.replay(direction));
        if walked {
            self.shared.state.borrow_mut().version += 1;
        }
        walked
    }

    /// Replays the next entry in `direction`. Returns false when there is
    /// none, or when a restore stopped it.
    fn replay(&self, direction: Direction) -> bool {
        let Some(mut walk) = Walk::begin(self, direction) else {
            return false;
        };
        walk.apply();
        walk.finish()
    }
}

/// An open transaction. Dropping it closes it.
///
/// # Examples
///
/// ```
/// # use seam_undo::{Bound, History};
#[doc = include_str!("../doc/setup.rs")]
/// # let history = History::new(&ui, Bound::default())?;
/// // An empty transaction leaves no step.
/// let scope = history.transaction("Nothing happened");
/// drop(scope);
/// assert!(history.is_empty());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
#[must_use = "a transaction closes as soon as it drops"]
pub struct Transaction {
    history: History,
}

impl Drop for Transaction {
    fn drop(&mut self) {
        self.history.close_transaction();
    }
}

impl State {
    fn len(&self) -> usize {
        self.applied.len() + self.undone.len()
    }

    fn next_to_replay(&mut self, direction: Direction) -> Option<&mut Recorded> {
        match direction {
            Direction::Undo => self.applied.back_mut(),
            Direction::Redo => self.undone.last_mut(),
        }
    }

    /// Puts `steps` back in the entry a replay in `direction` applied, and
    /// moves that entry to the other stack.
    fn settle(&mut self, direction: Direction, steps: Vec<Box<dyn Step>>) {
        let taken = match direction {
            Direction::Undo => self.applied.pop_back(),
            Direction::Redo => self.undone.pop(),
        };
        // During a walk, every request that could move an entry is refused.
        let mut entry = taken.expect("a walk's entry stays in place until it settles");
        entry.steps = steps;
        match direction {
            Direction::Undo => self.undone.push(entry),
            Direction::Redo => self.applied.push_back(entry),
        }
        self.end_joining();
    }

    /// Takes `stamp` as the clock's counts, dropping every entry if they
    /// moved since the history last looked. Returns whether they had.
    fn observe(&mut self, stamp: RestoreStamp) -> bool {
        if stamp == self.seen {
            return false;
        }
        self.seen = stamp;
        self.clear_all();
        true
    }

    /// Drops the undone entries in listing order, the oldest first, and
    /// returns the units they held.
    fn drop_undone(&mut self) -> usize {
        self.undone.drain(..).rev().map(|entry| entry.units).sum()
    }

    /// Drops every entry in listing order, moving the version if there were
    /// any.
    fn clear_all(&mut self) {
        if self.len() > 0 {
            self.applied.clear();
            self.drop_undone();
            self.units = 0;
            self.version += 1;
        }
        self.end_joining();
    }

    fn end_joining(&mut self) {
        if let Some(scope) = &mut self.scope {
            scope.joining = false;
        }
    }

    /// Drops the oldest applied entries, whole, while the booked size is
    /// over the cap and more than the floor are applied. Returns whether it
    /// dropped any.
    fn prune(&mut self) -> bool {
        let before = self.applied.len();
        while self.units > self.bound.max_units
            && self.applied.len() > self.bound.min_entries
            && let Some(oldest) = self.applied.pop_front()
        {
            self.units -= oldest.units;
        }
        // The joining entry is the newest, so it went only with the rest.
        if self.applied.is_empty() {
            self.end_joining();
        }
        self.applied.len() < before
    }
}

#[derive(Debug, Clone, Copy)]
enum Direction {
    Undo,
    Redo,
}

impl Direction {
    fn apply(self, step: &mut dyn Step) {
        match self {
            Direction::Undo => step.undo(),
            Direction::Redo => step.redo(),
        }
    }
}

/// One replay, holding its entry's steps so they apply with nothing
/// borrowed; the entry stays listed while they are out.
///
/// While it lives the history is replaying. [`finish`](Self::finish) ends
/// it; dropped without that, by a panic in a step, it empties the history,
/// and the steps it held are lost with the unwinding stack.
struct Walk<'a> {
    history: &'a History,
    direction: Direction,
    steps: Vec<Box<dyn Step>>,
    /// How many restores had been stored when it began.
    stored: u64,
}

impl<'a> Walk<'a> {
    /// Takes the steps of the next entry in `direction`.
    fn begin(history: &'a History, direction: Direction) -> Option<Walk<'a>> {
        let mut state = history.shared.state.borrow_mut();
        let steps = mem::take(&mut state.next_to_replay(direction)?.steps);
        state.phase = Phase::Replaying;
        let stored = state.seen.stored();
        Some(Walk {
            history,
            direction,
            steps,
            stored,
        })
    }

    /// Applies the steps, the last first for an undo, stopping before the
    /// next one once a restore is stored. A step that writes parameters holds
    /// loads off across that check and its write.
    fn apply(&mut self) {
        let clock = &self.history.shared.clock;
        let (direction, stored) = (self.direction, self.stored);
        let apply_one = |step: &mut Box<dyn Step>| {
            let _held = step.writes_parameters().then(|| clock.hold_loads());
            let unmoved = clock.stamp().stored() == stored;
            if unmoved {
                direction.apply(step.as_mut());
            }
            unmoved
        };
        match direction {
            Direction::Undo => self.steps.iter_mut().rev().all(apply_one),
            Direction::Redo => self.steps.iter_mut().all(apply_one),
        };
    }

    /// Ends the replay: it settles the entry on the other stack, or clears
    /// the history if the clock moved during it. Returns whether it settled.
    fn finish(mut self) -> bool {
        let (history, direction) = (self.history, self.direction);
        let steps = mem::take(&mut self.steps);
        // Disarmed: the normal end below replaces the panic path's clear.
        mem::forget(self);
        let stamp = history.shared.clock.stamp();
        let mut state = history.shared.state.borrow_mut();
        state.phase = Phase::Idle;
        if state.observe(stamp) {
            // The steps drop once the history is no longer borrowed.
            drop(state);
            drop(steps);
            return false;
        }
        state.settle(direction, steps);
        true
    }
}

impl Drop for Walk<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.history.shared.state.try_borrow_mut() {
            state.phase = Phase::Idle;
            state.clear_all();
        }
    }
}

#[cfg(test)]
mod tests;
