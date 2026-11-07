//! Enrollment by kind, and the tick that records what enrolled values did.

use std::any::Any;
use std::fmt;
use std::rc::{Rc, Weak};

use seam_params::{Commit, ParamId, RestoreStamp, Stamped, TrackedRef, Tracker, UiParams, Value};

use crate::history::{Step, booked};
use crate::{History, HistoryError, UndoEq, UndoSize};

/// A parameter's enrolment. Its steps hold it weakly and replay only while
/// it lives, so forgetting the parameter, which drops it, makes them inert.
pub(crate) type Enrolment = Rc<()>;

/// Where a poll hands each step it makes: its place in the clock's
/// sequence, the enrolled name, the step.
pub(crate) type Record<'a> = dyn FnMut(u64, &str, Box<dyn Step>) + 'a;

/// An enrolled plain value or list, as the tick polls it.
pub(crate) trait Polled: fmt::Debug {
    /// Hands each step the commits numbered before `cut` make to `record`,
    /// oldest first. A commit stamped other than `seen` happened before a
    /// restore the history has since seen: it records nothing.
    fn poll(&mut self, seen: RestoreStamp, cut: u64, record: &mut Record<'_>);
    fn gesture_open(&self) -> bool;
    /// Whether what it tracks still has a handle somewhere.
    fn alive(&self) -> bool;
    /// Whether `handle` is a handle to what it tracks: a `Value` or a `List`
    /// of the type it was enrolled with.
    fn tracks(&self, handle: &dyn Any) -> bool;
}

struct Enrolled<T: Clone> {
    tracker: Tracker<T>,
    /// The value the next step outside a gesture starts from.
    baseline: T,
    name: String,
}

impl<T: Clone> fmt::Debug for Enrolled<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Enrolled")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl<T: Clone + UndoEq + UndoSize + 'static> Enrolled<T> {
    fn step(&self, before: T, after: T) -> Box<dyn Step> {
        Box::new(ValueStep {
            target: self.tracker.downgrade(),
            before,
            after,
        })
    }

    /// Records the coalesced writes outside a gesture as one step from the
    /// baseline, if they changed it. The step takes the sequence of the
    /// newest write, when the value settled.
    fn flush(&mut self, pending: &mut Option<(T, u64)>, record: &mut Record<'_>) {
        if let Some((value, sequence)) = pending.take() {
            if !self.baseline.undo_eq(&value) {
                let step = self.step(self.baseline.clone(), value.clone());
                record(sequence, &self.name, step);
            }
            self.baseline = value;
        }
    }
}

impl<T: Clone + UndoEq + UndoSize + 'static> Polled for Enrolled<T> {
    fn poll(&mut self, seen: RestoreStamp, cut: u64, record: &mut Record<'_>) {
        let mut pending = None;
        for Stamped {
            stamp,
            sequence,
            commit,
        } in self.tracker.take_before(cut)
        {
            if stamp != seen {
                // From before a restore the history has since seen: the
                // baseline follows it and nothing records.
                self.flush(&mut pending, record);
                self.baseline = match commit {
                    Commit::Set { value, .. } => value,
                    Commit::Gesture { after, .. } => after,
                };
                continue;
            }
            match commit {
                Commit::Set {
                    value,
                    recorded: true,
                } => pending = Some((value, sequence)),
                // A restore, a replay or a write derived from one: the
                // baseline follows it and nothing records.
                Commit::Set {
                    value,
                    recorded: false,
                } => {
                    self.flush(&mut pending, record);
                    self.baseline = value;
                }
                Commit::Gesture { before, after } => {
                    self.flush(&mut pending, record);
                    if !before.undo_eq(&after) {
                        let step = self.step(before, after.clone());
                        record(sequence, &self.name, step);
                    }
                    self.baseline = after;
                }
            }
        }
        self.flush(&mut pending, record);
    }

    fn gesture_open(&self) -> bool {
        self.tracker.gesture_open()
    }

    fn alive(&self) -> bool {
        self.tracker.alive()
    }

    fn tracks(&self, handle: &dyn Any) -> bool {
        handle
            .downcast_ref::<Value<T>>()
            .is_some_and(|value| self.tracker.tracks(value))
    }
}

/// A parameter's step: a closed user gesture, replayed through the UI
/// handle as one complete host gesture.
struct ParamStep {
    ui: UiParams,
    id: ParamId,
    before: f32,
    after: f32,
    enrolment: Weak<()>,
}

impl ParamStep {
    fn replay(&self, value: f32) {
        if self.enrolment.strong_count() > 0 {
            self.ui.replay(self.id, value);
        }
    }
}

impl fmt::Debug for ParamStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParamStep")
            .field("id", &self.id)
            .field("before", &self.before)
            .field("after", &self.after)
            .finish_non_exhaustive()
    }
}

impl Step for ParamStep {
    fn undo(&mut self) {
        self.replay(self.before);
    }

    fn redo(&mut self) {
        self.replay(self.after);
    }

    fn units(&self) -> usize {
        booked(2 * size_of::<f32>())
    }

    fn writes_parameters(&self) -> bool {
        true
    }
}

/// A plain value's step, which replays as nothing once the value is dropped
/// or forgotten.
struct ValueStep<T> {
    target: TrackedRef<T>,
    before: T,
    after: T,
}

impl<T> fmt::Debug for ValueStep<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ValueStep").finish_non_exhaustive()
    }
}

impl<T: Clone + UndoSize> Step for ValueStep<T> {
    fn undo(&mut self) {
        self.target.replay(self.before.clone());
    }

    fn redo(&mut self) {
        self.target.replay(self.after.clone());
    }

    fn units(&self) -> usize {
        booked(self.before.undo_size() + self.after.undo_size())
    }
}

impl History {
    /// Enrolls a parameter: each user gesture on it that closes with its value
    /// changed is then one step, from its value before the user's first write
    /// to the user's last. No other write to it records.
    ///
    /// Enrolling ticks first, once the parameter is known not to be enrolled:
    /// the UI handle queues every parameter's closed gestures, so that tick
    /// drops this one's earlier gestures, and records what the enrolled items
    /// did, as any tick does. A gesture still open records at its close.
    ///
    /// # Errors
    ///
    /// Returns [`HistoryError::AlreadyEnrolled`] when the parameter is enrolled
    /// already, before the tick or by a destructor the tick ran.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_undo::{Bound, History, HistoryError};
    #[doc = include_str!("../doc/setup.rs")]
    /// # let history = History::new(&ui, Bound::default())?;
    /// history.enroll_param(gain)?;
    /// let again = history.enroll_param(gain);
    /// assert_eq!(again, Err(HistoryError::AlreadyEnrolled));
    ///
    /// // The plugin's own edit is not the user's, so it records nothing.
    /// let mut edit = ui.edit();
    /// edit.write(gain, 9.0);
    /// drop(edit);
    /// history.tick();
    /// assert!(history.is_empty());
    ///
    /// ui.write(gain, 2.0);
    /// history.tick();
    /// assert_eq!(history.len(), 1);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn enroll_param(&self, id: ParamId) -> Result<(), HistoryError> {
        if self.shared.params.borrow()[id.index()].is_some() {
            return Err(HistoryError::AlreadyEnrolled);
        }
        self.tick();
        let mut params = self.shared.params.borrow_mut();
        let slot = &mut params[id.index()];
        // Again, since the tick drops values whose destructors may run any
        // code.
        if slot.is_some() {
            return Err(HistoryError::AlreadyEnrolled);
        }
        *slot = Some(Rc::new(()));
        Ok(())
    }

    /// Enrolls every parameter not enrolled yet, as
    /// [`enroll_param`](Self::enroll_param) does.
    pub fn enroll_params(&self) {
        for id in self.shared.ui.set().ids() {
            let _ = self.enroll_param(id);
        }
    }

    /// Stops recording a parameter. Its steps stay listed and replay as
    /// nothing.
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
    /// history.forget_param(gain);
    /// ui.write(gain, 2.0);
    /// history.tick();
    ///
    /// // The first step is still listed, but undoing it writes nothing.
    /// assert_eq!(history.len(), 1);
    /// assert_eq!(history.undo(), Ok(true));
    /// assert_eq!(set.value(gain), 2.0);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn forget_param(&self, id: ParamId) {
        self.shared.params.borrow_mut()[id.index()] = None;
    }

    /// Enrolls a plain value under `name`: each change through
    /// [`Value::set`] outside a gesture is then one step, the writes between
    /// two ticks coalescing into the latest, and each gesture that closes
    /// changed is one step from its value at the open.
    ///
    /// Unlike [`enroll_param`](Self::enroll_param), enrolling does not tick: a
    /// value queues changes only once enrolled, so there is nothing earlier
    /// to drop, and pending writes stay pending, so a value written on either
    /// side of this call still makes one step.
    ///
    /// The history holds the value weakly: it never keeps it alive, and
    /// forgets it at the first tick after its last handle drops. Its steps
    /// stay listed and replay as nothing.
    ///
    /// # Errors
    ///
    /// Returns [`HistoryError::AlreadyEnrolled`] when the value is enrolled
    /// already, in any history, and [`HistoryError::OtherClock`] when it was
    /// made on another parameter set than this history's.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::Value;
    /// use seam_undo::{Bound, History};
    #[doc = include_str!("../doc/setup.rs")]
    /// # let history = History::new(&ui, Bound::default())?;
    /// // A value the plugin owns besides its parameters, such as the tab.
    /// let tab = Value::new(0_u8, &set);
    /// history.enroll(&tab, "Select tab")?;
    ///
    /// tab.set(2);
    /// history.tick();
    /// history.undo()?;
    /// assert_eq!(tab.get(), 0);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn enroll<T>(&self, value: &Value<T>, name: &str) -> Result<(), HistoryError>
    where
        T: Clone + UndoEq + UndoSize + Send + 'static,
    {
        self.same_clock(value.clock())?;
        let tracker = value.take_tracker().ok_or(HistoryError::AlreadyEnrolled)?;
        let baseline = value.get();
        self.shared.values.borrow_mut().push(Box::new(Enrolled {
            tracker,
            baseline,
            name: name.to_owned(),
        }));
        Ok(())
    }

    /// Stops recording a plain value and returns whether it was enrolled here.
    /// Its steps stay listed and replay as nothing.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::Value;
    /// use seam_undo::{Bound, History};
    #[doc = include_str!("../doc/setup.rs")]
    /// # let history = History::new(&ui, Bound::default())?;
    /// let tab = Value::new(0_u8, &set);
    /// history.enroll(&tab, "Select tab")?;
    /// tab.set(2);
    /// history.tick();
    ///
    /// assert!(history.forget(&tab));
    /// assert!(!history.forget(&tab)); // no longer enrolled
    ///
    /// // The step stays listed, but undoing it leaves the value alone.
    /// assert_eq!(history.len(), 1);
    /// history.undo()?;
    /// assert_eq!(tab.get(), 2);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn forget<T: Clone + 'static>(&self, value: &Value<T>) -> bool {
        self.forget_handle(value)
    }

    /// Stops recording what `handle`, a `Value` or a `List`, points to.
    /// Returns whether it was enrolled here.
    pub(crate) fn forget_handle(&self, handle: &dyn Any) -> bool {
        let removed = {
            let mut values = self.shared.values.borrow_mut();
            let found = values.iter().position(|enrolled| enrolled.tracks(handle));
            found.map(|index| values.remove(index))
        };
        // Dropped after the borrow ends, as the tick drops what it forgets.
        removed.is_some()
    }

    /// Records what the enrolled parameters, values and lists did since the
    /// last tick.
    ///
    /// Call it once per UI tick, editor open or not: changes queue until a
    /// tick takes them. Undo, redo, jump, clear and a transaction's outermost
    /// open and close tick first, so they act on current state.
    ///
    /// Steps record in the order their changes happened, so an undo between
    /// two dependent changes lands on a state that existed; a change another
    /// thread makes during the tick waits for the next one, still in order.
    /// Values and lists whose last handle dropped are forgotten here.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_undo::{Bound, History};
    #[doc = include_str!("../doc/setup.rs")]
    /// # let history = History::new(&ui, Bound::default())?;
    /// history.enroll_param(gain)?;
    /// ui.write(gain, 8.0);
    /// // Nothing is listed until the tick takes the write.
    /// assert!(history.is_empty());
    ///
    /// history.tick();
    /// assert_eq!(history.len(), 1);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn tick(&self) {
        if self.replaying() {
            return;
        }
        self.observe_clock();
        let seen = self.seen();
        let cut = self.cut();
        let mut steps: Vec<(u64, String, Box<dyn Step>)> = Vec::new();
        let dead: Vec<Box<dyn Polled>> = {
            let shared = &*self.shared;
            let params = shared.params.borrow();
            for record in shared.records.take() {
                let Some(enrolment) = &params[record.id.index()] else {
                    continue;
                };
                if record.stamp != seen {
                    continue;
                }
                let step = ParamStep {
                    ui: shared.ui.clone(),
                    id: record.id,
                    before: record.before,
                    after: record.after,
                    enrolment: Rc::downgrade(enrolment),
                };
                let name = shared.ui.set().decl(record.id).name;
                steps.push((record.sequence, name.to_owned(), Box::new(step)));
            }
            let mut values = shared.values.borrow_mut();
            for enrolled in values.iter_mut() {
                enrolled.poll(seen, cut, &mut |sequence, name, step| {
                    steps.push((sequence, name.to_owned(), step));
                });
            }
            values
                .extract_if(.., |enrolled| !enrolled.alive())
                .collect()
        };
        // Dropped after the borrow ends, since a destructor may run any code.
        drop(dead);
        steps.sort_by_key(|(sequence, ..)| *sequence);
        for (_, name, step) in steps {
            // Not replaying, so recording cannot be refused.
            let _ = self.record(&name, vec![step]);
        }
    }
}

#[cfg(test)]
mod tests {
    use seam_params::{Kind, ParamDecl, ParamSetBuilder};

    use super::*;

    #[test]
    fn a_parameter_step_holds_loads_off_and_a_value_step_does_not() {
        // The walk holds loads only for the steps that write parameters.
        let mut builder = ParamSetBuilder::new();
        builder
            .add(ParamDecl::new("p", "p", Kind::Bool, 0.0))
            .expect("valid");
        let set = builder.build();
        let ui = set.take_ui().expect("UI");
        let step = ParamStep {
            ui,
            id: set.id("p").expect("declared"),
            before: 0.0,
            after: 1.0,
            enrolment: Weak::new(),
        };
        assert!(step.writes_parameters());
        let value = Value::new(1_i32, &set);
        let tracker = value.take_tracker().expect("untracked");
        let step = ValueStep {
            target: tracker.downgrade(),
            before: 1,
            after: 2,
        };
        assert!(!step.writes_parameters());
    }
}
