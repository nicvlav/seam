//! Plain values: plugin state that is not a parameter, with gestures and
//! origins like a parameter's but no host.

use std::collections::VecDeque;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use crate::origin::Author;
use crate::{Origin, ParamSet, Reading, RestoreReader, Stamped, audio_thread};

/// A change a [`Tracker`] hands on, in the order it happened.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub enum Commit<T> {
    /// A write outside any gesture, or one inside a gesture that records
    /// nothing. Writes between two takes coalesce, the latest winning, when
    /// they agree on whether they record and carry the same stamp.
    Set {
        /// What it stored.
        value: T,
        /// Whether undo records it: true for a write through
        /// [`Value::set`]; false for a derived write, a replay, or any write
        /// made while a restore applies.
        recorded: bool,
    },
    /// A gesture that closed after a recorded write inside it.
    Gesture {
        /// The value when it opened, or the value a restore or a replay wrote
        /// during it.
        before: T,
        /// The last recorded write inside it, or derived from a user's or
        /// the plugin's own change during it.
        after: T,
    },
}

#[derive(Debug)]
struct State<T> {
    value: T,
    origin: Origin,
    /// Open gesture tokens.
    gestures: u32,
    /// The open gesture's [`Commit::Gesture`] values so far.
    before: Option<T>,
    after: Option<T>,
    /// Whether a tracker takes commits; none queue otherwise.
    tracked: bool,
    commits: VecDeque<Stamped<Commit<T>>>,
}

#[derive(Debug)]
struct Shared<T> {
    state: Mutex<State<T>>,
    changes: AtomicU64,
    /// Moves at every track and untrack, so a step from an old tracking
    /// replays as nothing.
    generation: AtomicU64,
    clock: RestoreReader,
}

impl<T> Shared<T> {
    fn lock(&self) -> MutexGuard<'_, State<T>> {
        audio_thread::assert_not_here("a Value call");
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A plain value: the plugin's own state that is not a parameter, a sample
/// path or a view setting, readable and writable from any thread but the
/// audio thread. Clones share it.
///
/// A number the DSP reads is a parameter, a [`Float`](crate::Float) say,
/// not a value: the audio thread reads a parameter with no lock.
///
/// Every write moves a change counter and names its [`Origin`], so a
/// surface polls it as it polls a parameter. Undo records a write through
/// [`set`](Self::set) and never one through [`set_from`](Self::set_from).
/// A plain value has its own gestures, from [`begin`](Self::begin), with no
/// host to tell.
///
/// `T`'s `Clone` runs under the value's lock, which is not reentrant, so a
/// `Clone` that calls the same value deadlocks. Its `Drop` runs after the
/// unlock and may call it.
///
/// # Examples
///
/// ```
/// use seam_params::{ParamSet, Value};
///
/// // A value belongs to a parameter set, so restores and undo see it.
/// let set = ParamSet::from_decls([])?;
/// let sample_path = Value::new(String::from("kick.wav"), &set);
///
/// sample_path.set(String::from("snare.wav"));
/// assert_eq!(sample_path.get(), "snare.wav");
/// assert_eq!(sample_path.changes(), 1);
/// # Ok::<(), seam_params::DeclError>(())
/// ```
///
/// # Panics
///
/// In a build with `debug_assertions`, each call that takes the value's lock
/// panics while the thread holds an [`audio_thread`] mark. Making and cloning
/// a value, [`changes`](Self::changes) and [`ptr_eq`](Self::ptr_eq) take
/// none.
#[derive(Debug)]
pub struct Value<T> {
    shared: Arc<Shared<T>>,
}

impl<T> Clone for Value<T> {
    fn clone(&self) -> Self {
        Value {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<T: Clone> Value<T> {
    /// Creates a value holding `initial`, tied to `set` so restores and undo
    /// see it. [`Value`] shows one made and written.
    pub fn new(initial: T, set: &ParamSet) -> Self {
        let clock = set.clock().reader();
        Value {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    value: initial,
                    origin: Origin::Restore,
                    gestures: 0,
                    before: None,
                    after: None,
                    tracked: false,
                    commits: VecDeque::new(),
                }),
                changes: AtomicU64::new(0),
                generation: AtomicU64::new(0),
                clock,
            }),
        }
    }

    /// Returns the value now.
    pub fn get(&self) -> T {
        self.shared.lock().value.clone()
    }

    /// Returns the change count, the value, and the origin of the write that
    /// stored it, from one lock, as a [`Reading`].
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{Origin, ParamSet, Value};
    ///
    /// let set = ParamSet::from_decls([])?;
    /// let name = Value::new(String::from("Init"), &set);
    ///
    /// // Before any write: the initial value, which counts as a restore's.
    /// let first = name.read();
    /// assert_eq!((first.changes, first.value.as_str(), first.origin), (0, "Init", Origin::Restore));
    ///
    /// name.set(String::from("Lead"));
    /// let second = name.read();
    /// assert_eq!((second.changes, second.value.as_str(), second.origin), (1, "Lead", Origin::User));
    /// # Ok::<(), seam_params::DeclError>(())
    /// ```
    pub fn read(&self) -> Reading<T> {
        let state = self.shared.lock();
        // Every write moves the count after it unlocks, so under the lock the
        // value is at least as new as the count.
        Reading {
            changes: self.shared.changes.load(Ordering::Acquire),
            value: state.value.clone(),
            origin: state.origin,
        }
    }

    /// Returns how many writes it has taken.
    pub fn changes(&self) -> u64 {
        self.shared.changes.load(Ordering::Acquire)
    }

    /// Writes `value` as the user or the plugin's own code: undo records it,
    /// unless a restore is applying, so a session participant's apply needs no
    /// other setter. [`Value`] shows it.
    pub fn set(&self, value: T) {
        self.write(value, Author::Own);
    }

    /// Writes `value` derived from a change with `origin`, which it passes on
    /// for the next derivation.
    ///
    /// Undo never records it: the source's own step covers it, and undoing the
    /// source derives it again, so one user action stays one step however many
    /// values follow it. A change that must undo with its source as one step, a
    /// selection clamped after a remove say, is a [`set`](Self::set) inside the
    /// source's transaction instead.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{Origin, ParamSet, Value};
    ///
    /// let set = ParamSet::from_decls([])?;
    /// let name = Value::new(String::from("Init"), &set);
    ///
    /// name.set(String::from("Lead"));
    /// assert_eq!(name.read().origin, Origin::User);
    /// name.set_from(String::from("Pad"), Origin::Host);
    /// assert_eq!(name.read().origin, Origin::Host);
    /// # Ok::<(), seam_params::DeclError>(())
    /// ```
    pub fn set_from(&self, value: T, origin: Origin) {
        self.write(value, Author::Derived(origin));
    }

    /// Opens a gesture, or joins the one open. Writes until the last token
    /// closes are one undo step at most. The token stays on the thread that
    /// made it. [`ValueGesture`] shows one.
    pub fn begin(&self) -> ValueGesture<T> {
        let mut state = self.shared.lock();
        if state.gestures == 0 {
            // The last close took both, so these assignments drop nothing
            // under the lock.
            debug_assert!(state.before.is_none() && state.after.is_none());
            state.before = Some(state.value.clone());
            state.after = None;
        }
        state.gestures = state.gestures.saturating_add(1);
        ValueGesture {
            value: self.clone(),
            local: PhantomData,
        }
    }

    /// Returns whether a gesture is open.
    pub fn gesture_open(&self) -> bool {
        self.shared.lock().gestures > 0
    }

    /// Starts handing commits to a tracker: undo's enrollment. `None` when
    /// one already takes them, so a value is never recorded twice.
    #[doc(hidden)]
    pub fn take_tracker(&self) -> Option<Tracker<T>> {
        let mut state = self.shared.lock();
        if state.tracked {
            return None;
        }
        state.tracked = true;
        state.commits.clear();
        let generation = self.shared.generation.fetch_add(1, Ordering::AcqRel) + 1;
        Some(Tracker {
            shared: Arc::downgrade(&self.shared),
            generation,
        })
    }

    /// Returns whether `self` and `other` share one value, as [`Arc::ptr_eq`]
    /// does.
    pub fn ptr_eq(&self, other: &Value<T>) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }

    /// Returns the `RestoreClock` it was made with.
    #[doc(hidden)]
    pub fn clock(&self) -> &RestoreReader {
        &self.shared.clock
    }

    fn write(&self, value: T, author: Author) {
        let (origin, recorded) = author.resolve(self.shared.clock.applying());
        let stamp = self.shared.clock.stamp();
        // Both clones are made before the lock, so a panicking `Clone`
        // changes nothing. What the write replaces is declared before the
        // guard, so it drops after the unlock on every path, since a `T`'s
        // `Drop` may call this value.
        let stored = value.clone();
        let mut rebase = matches!(origin, Origin::Restore | Origin::Replay).then(|| value.clone());
        let old;
        let stale;
        let mut gesture = (None, None);
        let mut coalesced = None;
        let mut state = self.shared.lock();
        old = std::mem::replace(&mut state.value, stored);
        state.origin = origin;
        stale = if origin == Origin::Replay {
            // A replay writes over whatever another thread wrote since the
            // tick began; those commits would record on top of it and drop
            // the redo, so they go.
            std::mem::take(&mut state.commits)
        } else {
            VecDeque::new()
        };
        if state.gestures > 0 && recorded {
            gesture.1 = state.after.replace(value);
        } else if state.gestures > 0 && matches!(origin, Origin::User | Origin::Program) {
            // Derived from the user's or the plugin's own change: part of the
            // gesture, its newest after-value, so a linked value echoing the
            // drag back keeps the drag.
            gesture.1 = state.after.replace(value);
        } else {
            if state.gestures > 0
                && let Some(rebase) = rebase.take()
            {
                // A restore or a replay during a gesture: the gesture
                // records from the value it wrote, and the commit
                // below moves the baseline even if the gesture writes
                // nothing more. Host and learned writes leave the gesture's
                // before and after alone.
                gesture = (state.before.replace(rebase), state.after.take());
            }
            if state.tracked {
                let sequence = self.shared.clock.sequence();
                match state.commits.back_mut() {
                    Some(Stamped {
                        stamp: then,
                        sequence: at,
                        commit:
                            Commit::Set {
                                value: last,
                                recorded: was,
                            },
                    }) if *was == recorded && *then == stamp => {
                        coalesced = Some(std::mem::replace(last, value));
                        *at = sequence;
                    }
                    _ => state.commits.push_back(Stamped {
                        stamp,
                        sequence,
                        commit: Commit::Set { value, recorded },
                    }),
                }
            }
        }
        drop(state);
        self.shared.changes.fetch_add(1, Ordering::Release);
        drop((old, stale, gesture, coalesced));
    }

    fn close(&self) {
        let mut state = self.shared.lock();
        if state.gestures == 0 {
            return;
        }
        state.gestures -= 1;
        if state.gestures > 0 {
            return;
        }
        // The close writes nothing. Whatever the value holds stands, a
        // restore or any other write that landed during the gesture
        // included. Enrolling the value in undo therefore never changes how it
        // behaves.
        let gesture = (state.before.take(), state.after.take());
        let unrecorded = match gesture {
            (Some(before), Some(after)) if state.tracked => {
                state.commits.push_back(Stamped {
                    stamp: self.shared.clock.stamp(),
                    sequence: self.shared.clock.sequence(),
                    commit: Commit::Gesture { before, after },
                });
                None
            }
            unrecorded => Some(unrecorded),
        };
        drop(state);
        // After the unlock, since a `T`'s `Drop` may call this value.
        drop(unrecorded);
    }
}

/// An open gesture on a plain value. Dropping it closes it. It stays on the
/// thread that made it.
///
/// # Examples
///
/// ```
/// use seam_params::Value;
/// # let set = seam_params::ParamSet::from_decls([])?;
/// let name = Value::new(String::from("Init"), &set);
///
/// let drag = name.begin();
/// drag.set(String::from("Lead"));
/// assert!(name.gesture_open());
/// drop(drag); // closing writes nothing
/// assert_eq!(name.get(), "Lead");
/// # Ok::<(), seam_params::DeclError>(())
/// ```
#[derive(Debug)]
#[must_use = "a gesture closes as soon as it drops, so its writes would land outside it"]
pub struct ValueGesture<T: Clone> {
    value: Value<T>,
    local: PhantomData<*const ()>,
}

impl<T: Clone> ValueGesture<T> {
    /// Writes `value` inside the gesture, as [`Value::set`] does, so the
    /// writes until the gesture closes are one undo step at most.
    pub fn set(&self, value: T) {
        self.value.set(value);
    }
}

impl<T: Clone> Drop for ValueGesture<T> {
    fn drop(&mut self) {
        self.value.close();
    }
}

/// What undo enrolls: the commits of one [`Value`], in order.
///
/// It holds the value weakly, so it never keeps a value alive: once the
/// value's last handle drops, its queued commits drop with it and the tracker
/// takes nothing more. Dropping the tracker stops the commits and makes every
/// [`TrackedRef`] from it replay as nothing.
#[doc(hidden)]
#[derive(Debug)]
pub struct Tracker<T> {
    shared: Weak<Shared<T>>,
    generation: u64,
}

impl<T: Clone> Tracker<T> {
    /// Returns whether it tracks `value`.
    pub fn tracks(&self, value: &Value<T>) -> bool {
        std::ptr::eq(self.shared.as_ptr(), Arc::as_ptr(&value.shared))
    }

    /// Returns whether the value still has a handle somewhere.
    pub fn alive(&self) -> bool {
        self.shared.strong_count() > 0
    }

    /// Returns whether a gesture is open on the value; false once it is gone.
    pub fn gesture_open(&self) -> bool {
        self.shared
            .upgrade()
            .is_some_and(|shared| shared.lock().gestures > 0)
    }

    /// Takes every commit since the last take, oldest first. Nothing once the
    /// value is gone.
    pub fn take(&self) -> Vec<Stamped<Commit<T>>> {
        self.take_before(u64::MAX)
    }

    /// Takes the commits numbered before `cut`, oldest first; later ones wait
    /// for a later take. The queue is in number order, coalescing
    /// included, so this takes a prefix of it.
    pub fn take_before(&self, cut: u64) -> Vec<Stamped<Commit<T>>> {
        let Some(shared) = self.shared.upgrade() else {
            return Vec::new();
        };
        let mut state = shared.lock();
        let end = state
            .commits
            .iter()
            .position(|commit| commit.sequence >= cut)
            .unwrap_or(state.commits.len());
        state.commits.drain(..end).collect()
    }

    /// Returns a handle a recorded step keeps: it replays while this tracker
    /// lives and the value does, and does nothing after.
    pub fn downgrade(&self) -> TrackedRef<T> {
        TrackedRef {
            shared: Weak::clone(&self.shared),
            generation: self.generation,
        }
    }
}

impl<T> Drop for Tracker<T> {
    fn drop(&mut self) {
        let Some(shared) = self.shared.upgrade() else {
            return;
        };
        let mut state = shared.lock();
        state.tracked = false;
        let stale = std::mem::take(&mut state.commits);
        drop(state);
        shared.generation.fetch_add(1, Ordering::AcqRel);
        // After the unlock, since a `T`'s `Drop` may call this value.
        drop(stale);
    }
}

/// A recorded step's handle to a tracked [`Value`].
#[doc(hidden)]
#[derive(Debug)]
pub struct TrackedRef<T> {
    shared: Weak<Shared<T>>,
    generation: u64,
}

impl<T: Clone> TrackedRef<T> {
    /// Writes `value` as a replay: it moves the value and records
    /// nothing. Does nothing once the value is dropped or no longer
    /// tracked by the tracker this came from. Returns whether it wrote.
    pub fn replay(&self, value: T) -> bool {
        let Some(shared) = self.shared.upgrade() else {
            return false;
        };
        if shared.generation.load(Ordering::Acquire) != self.generation {
            return false;
        }
        Value { shared }.write(value, Author::Derived(Origin::Replay));
        true
    }
}
