//! Owned lists: one step per verb, elements restored by identity.

use std::any::Any;
use std::fmt;

use seam_params::{List, ListCommit, ListTracker, RestoreStamp, Stamped, TrackedList};

use crate::enroll::{Polled, Record};
use crate::history::{Step, booked};
use crate::{History, HistoryError, UndoSize};

struct EnrolledList<T> {
    tracker: ListTracker<T>,
    name: String,
}

impl<T> fmt::Debug for EnrolledList<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnrolledList")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl<T: UndoSize + Send + 'static> Polled for EnrolledList<T> {
    fn poll(&mut self, seen: RestoreStamp, cut: u64, record: &mut Record<'_>) {
        let (commits, target) = self.tracker.take_before(cut);
        for Stamped {
            stamp,
            sequence,
            commit,
        } in commits
        {
            // A verb from before a restore the history has since seen
            // records nothing; a removed element it carries drops here.
            if stamp != seen {
                continue;
            }
            let target = target.clone();
            let step: Box<dyn Step> = match commit {
                ListCommit::Insert { index } => Box::new(InsertStep {
                    target,
                    index,
                    element: None,
                }),
                ListCommit::Remove { index, element } => Box::new(RemoveStep {
                    target,
                    index,
                    units: booked(element.undo_size()),
                    element: Some(element),
                }),
                ListCommit::Move { from, to } => Box::new(MoveStep { target, from, to }),
                ListCommit::Clear { elements } => {
                    let units = booked(elements.iter().map(UndoSize::undo_size).sum());
                    Box::new(ClearStep {
                        target,
                        elements: Some(elements),
                        units,
                    })
                }
            };
            record(sequence, &self.name, step);
        }
    }

    fn gesture_open(&self) -> bool {
        false
    }

    fn alive(&self) -> bool {
        self.tracker.alive()
    }

    fn tracks(&self, handle: &dyn Any) -> bool {
        handle
            .downcast_ref::<List<T>>()
            .is_some_and(|list| self.tracker.tracks(list))
    }
}

/// An insert: undo takes the element out into the step, redo puts it back.
struct InsertStep<T> {
    target: TrackedList<T>,
    index: usize,
    element: Option<T>,
}

/// A remove: the step owns the element until undo puts it back.
struct RemoveStep<T> {
    target: TrackedList<T>,
    index: usize,
    element: Option<T>,
    units: usize,
}

/// A move. Once stale it stays stale, so it never moves an element back
/// that it did not move.
struct MoveStep<T> {
    target: TrackedList<T>,
    from: usize,
    to: usize,
}

/// A clear: the step owns every element until undo puts them back into a
/// list that is still empty.
struct ClearStep<T> {
    target: TrackedList<T>,
    elements: Option<Vec<T>>,
    units: usize,
}

macro_rules! opaque_debug {
    ($($step:ident),*) => {$(
        impl<T> fmt::Debug for $step<T> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($step)).finish_non_exhaustive()
            }
        }
    )*};
}

opaque_debug!(InsertStep, RemoveStep, MoveStep, ClearStep);

/// Takes the element at `index` into `slot`, unless it already holds one.
fn take_into<T>(target: &TrackedList<T>, index: usize, slot: &mut Option<T>) {
    if slot.is_none() {
        *slot = target.remove(index);
    }
}

/// Puts `slot`'s element back at `index`; a stale step keeps it.
fn put_from<T>(target: &TrackedList<T>, index: usize, slot: &mut Option<T>) {
    if let Some(element) = slot.take()
        && let Err(element) = target.insert(index, element)
    {
        *slot = Some(element);
    }
}

impl<T> Step for InsertStep<T> {
    fn undo(&mut self) {
        take_into(&self.target, self.index, &mut self.element);
    }

    fn redo(&mut self) {
        put_from(&self.target, self.index, &mut self.element);
    }

    // The element is still in the list when the step records, and a step's
    // units are fixed then, so an undone insert holds its element unbooked.
    // That is never more than the list held, since an element only moves
    // from the list into a step.
    fn units(&self) -> usize {
        booked(size_of::<T>())
    }
}

impl<T> Step for RemoveStep<T> {
    fn undo(&mut self) {
        put_from(&self.target, self.index, &mut self.element);
    }

    fn redo(&mut self) {
        take_into(&self.target, self.index, &mut self.element);
    }

    fn units(&self) -> usize {
        self.units
    }
}

impl<T> Step for MoveStep<T> {
    fn undo(&mut self) {
        self.target.move_item(self.to, self.from);
    }

    fn redo(&mut self) {
        self.target.move_item(self.from, self.to);
    }

    fn units(&self) -> usize {
        booked(2 * size_of::<usize>())
    }
}

impl<T> Step for ClearStep<T> {
    fn undo(&mut self) {
        if let Some(elements) = self.elements.take()
            && let Err(elements) = self.target.restore_all(elements)
        {
            self.elements = Some(elements);
        }
    }

    fn redo(&mut self) {
        if self.elements.is_none() {
            self.elements = self.target.take_all();
        }
    }

    fn units(&self) -> usize {
        self.units
    }
}

impl History {
    /// Enrolls an owned list under `name`: each verb that changes it is then
    /// one step, and undo puts elements back by identity, never by copy.
    /// Like [`enroll`](Self::enroll), and for its reasons, enrolling does not
    /// tick.
    ///
    /// The history holds the list weakly: dropping its last handle frees its
    /// elements at once, and the history forgets it at the next tick. Its
    /// steps stay listed, replay as nothing, and free the elements they hold
    /// when they drop.
    ///
    /// # Errors
    ///
    /// Returns [`HistoryError::AlreadyEnrolled`] when the list is enrolled
    /// already, in any history, and [`HistoryError::OtherClock`] when it was
    /// made on another parameter set than this history's.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::List;
    /// use seam_undo::{Bound, History};
    #[doc = include_str!("../doc/setup.rs")]
    /// # let history = History::new(&ui, Bound::default())?;
    /// let layers = List::new(vec!["Pad".to_owned()], &set);
    /// history.enroll_list(&layers, "Layers")?;
    ///
    /// layers.push("Strings".to_owned());
    /// history.tick();
    /// history.undo()?;
    /// assert_eq!(layers.len(), 1);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn enroll_list<T: UndoSize + Send + 'static>(
        &self,
        list: &List<T>,
        name: &str,
    ) -> Result<(), HistoryError> {
        self.same_clock(list.clock())?;
        let tracker = list.take_tracker().ok_or(HistoryError::AlreadyEnrolled)?;
        self.shared.values.borrow_mut().push(Box::new(EnrolledList {
            tracker,
            name: name.to_owned(),
        }));
        Ok(())
    }

    /// Stops recording a list and returns whether it was enrolled here. Its
    /// steps stay listed and replay as nothing; the elements they hold drop
    /// with them.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::List;
    /// use seam_undo::{Bound, History};
    #[doc = include_str!("../doc/setup.rs")]
    /// # let history = History::new(&ui, Bound::default())?;
    /// let layers = List::new(vec!["Pad".to_owned()], &set);
    /// history.enroll_list(&layers, "Layers")?;
    /// layers.push("Strings".to_owned());
    /// history.tick();
    ///
    /// assert!(history.forget_list(&layers));
    /// history.undo()?;
    /// assert_eq!(layers.len(), 2); // the step replays as nothing
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn forget_list<T: Send + 'static>(&self, list: &List<T>) -> bool {
        self.forget_handle(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(list: &List<char>) -> Vec<char> {
        list.with(<[char]>::to_vec)
    }

    fn set() -> std::sync::Arc<seam_params::ParamSet> {
        seam_params::ParamSetBuilder::new().build()
    }

    #[test]
    fn the_tick_forgets_a_list_whose_last_handle_dropped() {
        // No poll and no handle is left behind for a dead list.
        let set = set();
        let ui = set.take_ui().expect("the first UI handle");
        let history = History::new(&ui, crate::Bound::default()).expect("the first history");
        let list = List::new(vec!['a'], &set);
        history.enroll_list(&list, "list").expect("first");
        history.tick();
        assert_eq!(history.shared.values.borrow().len(), 1);
        drop(list);
        history.tick();
        assert!(history.shared.values.borrow().is_empty());
    }

    #[test]
    fn a_verb_waiting_when_a_step_replays_makes_the_lists_steps_stale_for_good() {
        // A verb from another thread between the tick and the replay.
        // Neither index accounts for the other, so the verb records
        // nothing and every step of the list goes inert.
        let list = List::new(vec!['a', 'b', 'c'], &set());
        let tracker = list.take_tracker().expect("untracked");
        assert!(list.move_item(0, 2));
        let (_, target) = tracker.take();
        let mut step = MoveStep {
            target,
            from: 0,
            to: 2,
        };
        step.undo();
        assert_eq!(chars(&list), ['a', 'b', 'c']);
        list.push('d');
        step.redo();
        assert_eq!(chars(&list), ['a', 'b', 'c', 'd'], "stale");
        assert!(tracker.take().0.is_empty(), "the waiting verb was dropped");
        step.undo();
        step.redo();
        assert_eq!(chars(&list), ['a', 'b', 'c', 'd'], "stale for good");
    }

    #[test]
    fn a_clear_undone_into_a_list_that_is_not_empty_keeps_its_elements() {
        // The elements stay in the step until the list is empty.
        let list = List::new(vec!['a'], &set());
        let tracker = list.take_tracker().expect("untracked");
        assert!(list.clear());
        let (mut commits, target) = tracker.take();
        let Some(Stamped {
            commit: ListCommit::Clear { elements },
            ..
        }) = commits.pop()
        else {
            panic!("a clear commit");
        };
        let mut step = ClearStep {
            target,
            elements: Some(elements),
            units: 0,
        };
        list.push('b');
        drop(tracker.take());
        step.undo();
        assert_eq!(chars(&list), ['b']);
        assert_eq!(step.elements.as_deref(), Some(&['a'][..]));
        assert!(list.remove(0));
        drop(tracker.take());
        step.undo();
        assert_eq!(chars(&list), ['a']);
    }
}
