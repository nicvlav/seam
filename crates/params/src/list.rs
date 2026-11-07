//! Owned lists: plugin state kept as a list, whose verbs undo records one
//! step each, restoring elements by identity.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use crate::origin::Author;
use crate::{Origin, ParamSet, Reading, RestoreReader, Stamped, audio_thread};

/// A verb a [`ListTracker`] hands on, in the order it happened. A removed
/// element travels with its verb, so it has exactly one owner at a time:
/// the list while present, undo while removed.
#[doc(hidden)]
#[derive(Debug, PartialEq)]
pub enum ListCommit<T> {
    /// An element was inserted at `index`.
    Insert {
        /// Where it went.
        index: usize,
    },
    /// The element at `index` was removed.
    Remove {
        /// Where it was.
        index: usize,
        /// The element itself.
        element: T,
    },
    /// The element at `from` moved to `to`.
    Move {
        /// Where it was.
        from: usize,
        /// Where it is.
        to: usize,
    },
    /// Every element was removed.
    Clear {
        /// The elements, in order.
        elements: Vec<T>,
    },
}

#[derive(Debug)]
struct State<T> {
    items: Vec<T>,
    origin: Origin,
    tracked: bool,
    /// Moves wherever earlier steps go stale: at every track, untrack and
    /// reset, and at every verb that records nothing. Each of these clears
    /// the queue, so every queued verb was made at the current generation.
    generation: u64,
    commits: VecDeque<Stamped<ListCommit<T>>>,
}

#[derive(Debug)]
struct Shared<T> {
    state: Mutex<State<T>>,
    changes: AtomicU64,
    clock: RestoreReader,
}

impl<T> Shared<T> {
    fn lock(&self) -> MutexGuard<'_, State<T>> {
        audio_thread::assert_not_here("a List call");
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Unlocks, then moves the change count, so under the lock the elements
    /// are always at least as new as the count.
    fn publish(&self, state: MutexGuard<'_, State<T>>) {
        drop(state);
        self.changes.fetch_add(1, Ordering::Release);
    }
}

/// A list the plugin owns: samples, layers, markers. Readable and writable
/// from any thread but the audio thread; clones share it.
///
/// Its verbs are [`insert`](Self::insert), [`push`](Self::push),
/// [`remove`](Self::remove), [`move_item`](Self::move_item) and
/// [`clear`](Self::clear). Each one that changes the list is one undo step
/// once undo tracks it; one that changes nothing records nothing. Three kinds
/// of change never record: a verb derived from another change, through
/// [`derived`](Self::derived), a [`reset`](Self::reset), and any verb made
/// while a restore applies. Each makes every step recorded before it replay as
/// nothing, since those steps index into contents that have changed.
///
/// # Examples
///
/// ```
/// use seam_params::{List, ParamSet};
///
/// // A list belongs to a parameter set, so restores and undo see it.
/// let set = ParamSet::from_decls([])?;
/// let samples = List::new(vec![String::from("kick")], &set);
///
/// samples.push(String::from("snare"));
/// samples.push(String::from("hat"));
///
/// let names = samples.with(|items| items.join(", "));
/// assert_eq!(names, "kick, snare, hat");
/// assert_eq!(samples.len(), 3);
/// # Ok::<(), seam_params::DeclError>(())
/// ```
///
/// # Panics
///
/// In a build with `debug_assertions`, each call that takes the list's lock
/// panics while the thread holds an [`audio_thread`] mark:
/// [`len`](Self::len), [`is_empty`](Self::is_empty), [`with`](Self::with),
/// [`read`](Self::read), [`reset`](Self::reset), and the verbs here and on
/// [`Derived`]. Making, cloning and comparing a list, its
/// [`changes`](Self::changes) and [`derived`](Self::derived) take no lock and
/// do not.
#[derive(Debug)]
pub struct List<T> {
    shared: Arc<Shared<T>>,
}

impl<T> Clone for List<T> {
    fn clone(&self) -> Self {
        List {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<T> List<T> {
    /// Creates a list holding `items`, tied to `set` so restores and undo
    /// see it. [`List`] shows one made and written.
    pub fn new(items: Vec<T>, set: &ParamSet) -> Self {
        let clock = set.clock().reader();
        List {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    items,
                    origin: Origin::Restore,
                    tracked: false,
                    generation: 0,
                    commits: VecDeque::new(),
                }),
                changes: AtomicU64::new(0),
                clock,
            }),
        }
    }

    /// Returns how many elements it holds.
    pub fn len(&self) -> usize {
        self.shared.lock().items.len()
    }

    /// Returns whether it holds none.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Reads the elements, under the list's lock.
    ///
    /// `read` must not call this list, whose lock is not reentrant, nor
    /// another [`List`]: two threads each reading one list from inside a read
    /// of the other would deadlock. A [`Value`](crate::Value) may be called,
    /// since it runs none of its caller's code under its lock, only its type's
    /// `Clone` and `Drop`. [`List`] shows a read.
    pub fn with<R>(&self, read: impl FnOnce(&[T]) -> R) -> R {
        read(&self.shared.lock().items)
    }

    /// Returns how many changes it has taken.
    pub fn changes(&self) -> u64 {
        self.shared.changes.load(Ordering::Acquire)
    }

    /// Reads the elements as [`with`](Self::with) does, and returns what
    /// `read` made with the change count and the origin of the newest change,
    /// from one lock, as a [`Reading`].
    ///
    /// The origin is a verb's, a replay's or a reset's ([`Origin::Restore`]).
    /// `read` runs under that lock, so it may call no list, as for
    /// [`with`](Self::with).
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{List, Origin, ParamSet, Value};
    ///
    /// let set = ParamSet::from_decls([])?;
    /// let samples = List::new(vec!["kick"], &set);
    /// let count = Value::new(1, &set);
    ///
    /// // A count kept in step with the list passes on who changed it.
    /// samples.derived(Origin::Host).push("snare");
    /// let reading = samples.read(<[_]>::len);
    /// count.set_from(reading.value, reading.origin);
    /// assert_eq!((count.get(), count.read().origin), (2, Origin::Host));
    /// # Ok::<(), seam_params::DeclError>(())
    /// ```
    pub fn read<R>(&self, read: impl FnOnce(&[T]) -> R) -> Reading<R> {
        let state = self.shared.lock();
        // See `Shared::publish`.
        Reading {
            changes: self.shared.changes.load(Ordering::Acquire),
            value: read(&state.items),
            origin: state.origin,
        }
    }

    /// Inserts `element` at `index`.
    ///
    /// # Errors
    ///
    /// Hands `element` back when `index` is past the end.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{List, ParamSet};
    ///
    /// let set = ParamSet::from_decls([])?;
    /// let list = List::new(vec!["kick", "hat"], &set);
    ///
    /// assert_eq!(list.insert(1, "snare"), Ok(())); // between the two
    /// assert_eq!(list.insert(9, "clap"), Err("clap")); // past the end: handed back
    /// assert_eq!(list.with(|items| items.to_vec()), ["kick", "snare", "hat"]);
    /// # Ok::<(), seam_params::DeclError>(())
    /// ```
    pub fn insert(&self, index: usize, element: T) -> Result<(), T> {
        self.own().insert(index, element)
    }

    /// Appends `element`.
    pub fn push(&self, element: T) {
        self.own().push(element);
    }

    /// Removes the element at `index`. Returns whether there was one. A
    /// tracked list hands it to undo; otherwise it drops here.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{List, ParamSet};
    ///
    /// let set = ParamSet::from_decls([])?;
    /// let list = List::new(vec!["kick", "snare"], &set);
    ///
    /// assert!(list.remove(0));
    /// assert!(!list.remove(5)); // no such element: nothing changes
    /// assert_eq!(list.with(|items| items.to_vec()), ["snare"]);
    /// # Ok::<(), seam_params::DeclError>(())
    /// ```
    pub fn remove(&self, index: usize) -> bool {
        self.own().remove(index)
    }

    /// Moves the element at `from` to `to`. Returns whether it moved: a
    /// move onto its own slot or out of range changes nothing.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{List, ParamSet};
    ///
    /// let set = ParamSet::from_decls([])?;
    /// let list = List::new(vec!["kick", "snare", "hat"], &set);
    ///
    /// assert!(list.move_item(0, 2));
    /// assert_eq!(list.with(|items| items.to_vec()), ["snare", "hat", "kick"]);
    /// assert!(!list.move_item(1, 1)); // onto its own slot
    /// assert!(!list.move_item(0, 3)); // out of range
    /// # Ok::<(), seam_params::DeclError>(())
    /// ```
    pub fn move_item(&self, from: usize, to: usize) -> bool {
        self.own().move_item(from, to)
    }

    /// Removes every element. Returns whether there were any.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{List, ParamSet};
    ///
    /// let set = ParamSet::from_decls([])?;
    /// let list = List::new(vec!["kick"], &set);
    ///
    /// assert!(list.clear());
    /// assert!(!list.clear()); // already empty: nothing changes
    /// assert!(list.is_empty());
    /// # Ok::<(), seam_params::DeclError>(())
    /// ```
    pub fn clear(&self) -> bool {
        self.own().clear()
    }

    /// Returns the verbs, derived from a change with `origin`.
    ///
    /// A list kept in step with a value passes on that value's origin, as
    /// [`Value::set_from`](crate::Value::set_from) does. Undo never records
    /// them, and each one makes earlier steps replay as nothing, as [`List`]
    /// says; it also drops the list's verbs the history has not taken yet.
    /// [`Derived`] shows one.
    pub fn derived(&self, origin: Origin) -> Derived<'_, T> {
        Derived {
            list: self,
            author: Author::Derived(origin),
        }
    }

    /// The user's or the plugin's own verbs, which undo records.
    fn own(&self) -> Derived<'_, T> {
        Derived {
            list: self,
            author: Author::Own,
        }
    }

    /// Replaces every element, as a load does. It records nothing, and makes
    /// earlier steps replay as nothing, as [`List`] says.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{List, Origin, ParamSet};
    ///
    /// let set = ParamSet::from_decls([])?;
    /// let list = List::new(vec!["kick"], &set);
    ///
    /// list.push("snare");
    /// assert_eq!(list.read(<[_]>::len).origin, Origin::User);
    ///
    /// list.reset(vec!["hat"]);
    /// assert_eq!(list.with(|items| items.to_vec()), ["hat"]);
    /// assert_eq!(list.read(<[_]>::len).origin, Origin::Restore);
    /// # Ok::<(), seam_params::DeclError>(())
    /// ```
    pub fn reset(&self, items: Vec<T>) {
        let mut state = self.shared.lock();
        state.commits.clear();
        state.generation += 1;
        state.origin = Origin::Restore;
        let old = std::mem::replace(&mut state.items, items);
        self.shared.publish(state);
        // The old elements drop outside the lock.
        drop(old);
    }

    /// Starts handing verbs to a tracker: undo's enrollment. `None` when one
    /// already takes them.
    #[doc(hidden)]
    pub fn take_tracker(&self) -> Option<ListTracker<T>> {
        let mut state = self.shared.lock();
        if state.tracked {
            return None;
        }
        state.tracked = true;
        state.commits.clear();
        state.generation += 1;
        Some(ListTracker {
            shared: Arc::downgrade(&self.shared),
        })
    }

    /// Returns whether `self` and `other` share one list, as [`Arc::ptr_eq`]
    /// does.
    pub fn ptr_eq(&self, other: &List<T>) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }

    /// Returns the `RestoreClock` it was made with.
    #[doc(hidden)]
    pub fn clock(&self) -> &RestoreReader {
        &self.shared.clock
    }
}

/// A [`List`]'s verbs, made because of a change with one origin; see
/// [`List::derived`].
///
/// # Examples
///
/// ```
/// use seam_params::List;
/// # let set = seam_params::ParamSet::from_decls([])?;
/// let samples = List::new(vec!["kick", "snare"], &set);
/// use seam_params::Origin;
///
/// let derived = samples.derived(Origin::Host);
/// derived.push("hat");
/// derived.remove(0);
/// assert_eq!(samples.with(|items| items.to_vec()), ["snare", "hat"]);
/// # Ok::<(), seam_params::DeclError>(())
/// ```
#[derive(Debug)]
pub struct Derived<'a, T> {
    list: &'a List<T>,
    author: Author,
}

impl<T> Derived<'_, T> {
    /// Inserts `element` at `index`, as [`List::insert`] does, but derived.
    ///
    /// # Errors
    ///
    /// Hands `element` back when `index` is past the end.
    pub fn insert(&self, index: usize, element: T) -> Result<(), T> {
        let shared = &self.list.shared;
        let mut state = shared.lock();
        if index > state.items.len() {
            return Err(element);
        }
        state.items.insert(index, element);
        self.commit(&mut state, ListCommit::Insert { index });
        shared.publish(state);
        Ok(())
    }

    /// Appends `element`, as [`List::push`] does, but derived. [`Derived`]
    /// shows it.
    pub fn push(&self, element: T) {
        let shared = &self.list.shared;
        let mut state = shared.lock();
        let index = state.items.len();
        state.items.push(element);
        self.commit(&mut state, ListCommit::Insert { index });
        shared.publish(state);
    }

    /// Removes the element at `index`, as [`List::remove`] does, but derived.
    /// Returns whether there was one. [`Derived`] shows it.
    pub fn remove(&self, index: usize) -> bool {
        let shared = &self.list.shared;
        let mut state = shared.lock();
        if index >= state.items.len() {
            return false;
        }
        let element = state.items.remove(index);
        self.commit(&mut state, ListCommit::Remove { index, element });
        shared.publish(state);
        true
    }

    /// Moves the element at `from` to `to`, as [`List::move_item`] does, but
    /// derived. Returns whether it moved.
    pub fn move_item(&self, from: usize, to: usize) -> bool {
        let shared = &self.list.shared;
        let mut state = shared.lock();
        let len = state.items.len();
        if from == to || from >= len || to >= len {
            return false;
        }
        let element = state.items.remove(from);
        state.items.insert(to, element);
        self.commit(&mut state, ListCommit::Move { from, to });
        shared.publish(state);
        true
    }

    /// Removes every element, as [`List::clear`] does, but derived. Returns
    /// whether there were any.
    pub fn clear(&self) -> bool {
        let shared = &self.list.shared;
        let mut state = shared.lock();
        if state.items.is_empty() {
            return false;
        }
        let elements = std::mem::take(&mut state.items);
        self.commit(&mut state, ListCommit::Clear { elements });
        shared.publish(state);
        true
    }

    /// Queues `commit` for the tracker, or, for a verb that records nothing,
    /// drops it and makes earlier steps stale.
    fn commit(&self, state: &mut State<T>, commit: ListCommit<T>) {
        let clock = &self.list.shared.clock;
        let (origin, recorded) = self.author.resolve(clock.applying());
        state.origin = origin;
        if !recorded {
            state.commits.clear();
            state.generation += 1;
        } else if state.tracked {
            state.commits.push_back(Stamped {
                stamp: clock.stamp(),
                sequence: clock.sequence(),
                commit,
            });
        }
    }
}

/// What undo enrolls: the verbs of one [`List`], in order.
///
/// It holds the list weakly, so it never keeps a list or its elements
/// alive: once the list's last handle drops, the tracker takes nothing more.
/// Dropping the tracker stops the verbs and makes every [`TrackedList`] from
/// it replay as nothing.
#[doc(hidden)]
#[derive(Debug)]
pub struct ListTracker<T> {
    shared: Weak<Shared<T>>,
}

impl<T> ListTracker<T> {
    /// Returns whether it tracks `list`.
    pub fn tracks(&self, list: &List<T>) -> bool {
        std::ptr::eq(self.shared.as_ptr(), Arc::as_ptr(&list.shared))
    }

    /// Returns whether the list still has a handle somewhere.
    pub fn alive(&self) -> bool {
        self.shared.strong_count() > 0
    }

    /// Takes every verb since the last take, oldest first, with the handle
    /// their steps replay through. Nothing once the list is gone.
    pub fn take(&self) -> (Vec<Stamped<ListCommit<T>>>, TrackedList<T>) {
        self.take_before(u64::MAX)
    }

    /// Takes the verbs numbered before `cut`, oldest first, and their
    /// handle; later ones wait for a later take.
    pub fn take_before(&self, cut: u64) -> (Vec<Stamped<ListCommit<T>>>, TrackedList<T>) {
        let target = |generation| TrackedList {
            shared: Weak::clone(&self.shared),
            generation,
        };
        let Some(shared) = self.shared.upgrade() else {
            // A list that is gone never comes back, so no generation replays.
            return (Vec::new(), target(0));
        };
        let mut state = shared.lock();
        let end = state
            .commits
            .iter()
            .position(|commit| commit.sequence >= cut)
            .unwrap_or(state.commits.len());
        let commits = state.commits.drain(..end).collect();
        (commits, target(state.generation))
    }
}

impl<T> Drop for ListTracker<T> {
    fn drop(&mut self) {
        let Some(shared) = self.shared.upgrade() else {
            return;
        };
        let mut state = shared.lock();
        state.tracked = false;
        state.commits.clear();
        state.generation += 1;
    }
}

/// A recorded step's handle to a tracked [`List`].
///
/// Each replay records nothing, and a stale step's replay is a detectable
/// no-op. A step is stale when the list is gone, is no longer tracked by the
/// tracker this came from, or is too short for the index. It is also stale
/// when the list changed since by a reset or a verb that recorded nothing, or
/// holds a verb the history has not taken yet.
///
/// A waiting verb means the list changed since the tick, and neither the
/// step's index nor the verb's accounts for the other: the step drops the
/// verb, which then records nothing, and every earlier step goes stale, as
/// after a reset. Only another thread's write between a tick's start and a
/// replay does this, so edit an enrolled list on the UI thread.
#[doc(hidden)]
#[derive(Debug)]
pub struct TrackedList<T> {
    shared: Weak<Shared<T>>,
    generation: u64,
}

impl<T> Clone for TrackedList<T> {
    fn clone(&self) -> Self {
        TrackedList {
            shared: Weak::clone(&self.shared),
            generation: self.generation,
        }
    }
}

impl<T> TrackedList<T> {
    /// Whether the step may change the list now. Checked under the list's
    /// lock, so nothing changes it between the check and the step. A
    /// waiting verb makes the list's steps stale for good.
    fn current(&self, state: &mut State<T>) -> bool {
        if state.generation != self.generation {
            return false;
        }
        if !state.commits.is_empty() {
            state.commits.clear();
            state.generation += 1;
            return false;
        }
        true
    }

    /// Puts `element` back at `index`.
    ///
    /// # Errors
    ///
    /// Hands `element` back when the step is stale or `index` is past the
    /// end.
    pub fn insert(&self, index: usize, element: T) -> Result<(), T> {
        let Some(shared) = self.shared.upgrade() else {
            return Err(element);
        };
        let mut state = shared.lock();
        if !self.current(&mut state) || index > state.items.len() {
            return Err(element);
        }
        state.items.insert(index, element);
        state.origin = Origin::Replay;
        shared.publish(state);
        Ok(())
    }

    /// Takes the element at `index` out, or nothing when the step is stale.
    pub fn remove(&self, index: usize) -> Option<T> {
        let shared = self.shared.upgrade()?;
        let mut state = shared.lock();
        if !self.current(&mut state) || index >= state.items.len() {
            return None;
        }
        let element = state.items.remove(index);
        state.origin = Origin::Replay;
        shared.publish(state);
        Some(element)
    }

    /// Moves the element at `from` to `to`. Returns whether it moved.
    pub fn move_item(&self, from: usize, to: usize) -> bool {
        let Some(shared) = self.shared.upgrade() else {
            return false;
        };
        let mut state = shared.lock();
        let len = state.items.len();
        if !self.current(&mut state) || from >= len || to >= len {
            return false;
        }
        let element = state.items.remove(from);
        state.items.insert(to, element);
        state.origin = Origin::Replay;
        shared.publish(state);
        true
    }

    /// Puts every element back into an empty list.
    ///
    /// # Errors
    ///
    /// Hands the elements back when the step is stale or the list is not
    /// empty.
    pub fn restore_all(&self, elements: Vec<T>) -> Result<(), Vec<T>> {
        let Some(shared) = self.shared.upgrade() else {
            return Err(elements);
        };
        let mut state = shared.lock();
        if !self.current(&mut state) || !state.items.is_empty() {
            return Err(elements);
        }
        state.items = elements;
        state.origin = Origin::Replay;
        shared.publish(state);
        Ok(())
    }

    /// Takes every element out, or nothing when the step is stale.
    pub fn take_all(&self) -> Option<Vec<T>> {
        let shared = self.shared.upgrade()?;
        let mut state = shared.lock();
        if !self.current(&mut state) {
            return None;
        }
        let elements = std::mem::take(&mut state.items);
        state.origin = Origin::Replay;
        shared.publish(state);
        Some(elements)
    }
}
