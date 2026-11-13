//! The `RestoreClock`, which session and undo share through the parameter set
//! without depending on each other.

use std::cell::RefCell;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::audio_thread;

/// Keeps undo neutral to restores and orders what undo records. Plugins never
/// touch it.
#[doc(hidden)]
#[derive(Debug)]
pub struct RestoreClock {
    /// Restores stored, and restore passes completed: a history compares the
    /// two.
    stored: AtomicU64,
    completed: AtomicU64,
    /// Numbers every change undo can record, so one tick's changes replay in
    /// order.
    sequence: AtomicU64,
    /// Held by a load's writes and by a replay, so the two never interleave.
    loads: Mutex<()>,
}

impl RestoreClock {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(RestoreClock {
            stored: AtomicU64::new(0),
            completed: AtomicU64::new(0),
            sequence: AtomicU64::new(0),
            loads: Mutex::new(()),
        })
    }

    /// The side that marks restores: the session and the saved block's load.
    pub fn writer(self: &Arc<Self>) -> RestoreWriter {
        RestoreWriter {
            clock: Arc::clone(self),
        }
    }

    /// The side undo reads.
    pub fn reader(self: &Arc<Self>) -> RestoreReader {
        RestoreReader {
            clock: Arc::clone(self),
        }
    }

    /// Names the clock in a thread's marks. A guard borrows the clock, so the
    /// address belongs to it as long as the mark exists.
    fn address(&self) -> usize {
        std::ptr::from_ref(self) as usize
    }

    /// Never poisoned for good: a panic while it is held leaves nothing
    /// half-written that the lock protects.
    fn hold_loads(&self) -> MutexGuard<'_, ()> {
        audio_thread::assert_not_here("holding off loads");
        self.loads.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

thread_local! {
    /// The clocks this thread is applying a restore for, each with how many
    /// guards nest. A mark is one thread's: a worker that writes while the
    /// UI thread applies is not part of the apply.
    static MARKS: RefCell<Vec<(usize, u32)>> = const { RefCell::new(Vec::new()) };
}

/// Marks restores on a [`RestoreClock`].
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct RestoreWriter {
    clock: Arc<RestoreClock>,
}

impl RestoreWriter {
    /// A restore has been stored. Call it before the restore changes anything.
    ///
    /// Thread: any but audio.
    pub fn stored(&self) {
        self.clock.stored.fetch_add(1, Ordering::Release);
    }

    /// Returns how many restores have been stored: every load's write and
    /// every session restore moves it.
    ///
    /// Thread: any but audio.
    pub fn stored_count(&self) -> u64 {
        self.clock.stored.load(Ordering::Acquire)
    }

    /// Holds the applying mark on the calling thread while one participant's
    /// section applies, a late participant's included.
    ///
    /// Undo records nothing that thread writes while any guard is alive.
    /// Guards nest; the mark clears when the last one drops.
    pub fn applying(&self) -> ApplyingGuard<'_> {
        MARKS.with(|marks| {
            let mut marks = marks.borrow_mut();
            match marks
                .iter_mut()
                .find(|(clock, _)| *clock == self.clock.address())
            {
                Some((_, depth)) => *depth += 1,
                None => marks.push((self.clock.address(), 1)),
            }
        });
        ApplyingGuard {
            clock: &self.clock,
            _one_thread: PhantomData,
        }
    }

    /// A restore pass has completed. Once per pass, also a pass with no
    /// participants; a late participant's apply does not call it.
    pub fn completed(&self) {
        self.clock.completed.fetch_add(1, Ordering::Release);
    }

    pub(crate) fn hold_loads(&self) -> MutexGuard<'_, ()> {
        self.clock.hold_loads()
    }

    /// Whether it marks `clock`.
    pub fn on(&self, clock: &RestoreClock) -> bool {
        std::ptr::eq(&*self.clock, clock)
    }
}

/// The applying mark, held until dropped. It marks the thread that took it
/// and stays there.
#[doc(hidden)]
#[derive(Debug)]
#[must_use = "the mark clears as soon as the guard drops"]
pub struct ApplyingGuard<'a> {
    clock: &'a RestoreClock,
    _one_thread: PhantomData<*const ()>,
}

impl Drop for ApplyingGuard<'_> {
    fn drop(&mut self) {
        // The thread's list is gone only while the thread itself is exiting,
        // when no write can follow.
        let _ = MARKS.try_with(|marks| {
            let mut marks = marks.borrow_mut();
            if let Some(at) = marks
                .iter()
                .position(|(clock, _)| *clock == self.clock.address())
            {
                marks[at].1 -= 1;
                if marks[at].1 == 0 {
                    marks.swap_remove(at);
                }
            }
        });
    }
}

/// Reads a [`RestoreClock`].
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct RestoreReader {
    clock: Arc<RestoreClock>,
}

impl RestoreReader {
    /// Both counts now. A history keeps the stamp it last saw and clears when
    /// a new one differs.
    pub fn stamp(&self) -> RestoreStamp {
        RestoreStamp {
            stored: self.clock.stored.load(Ordering::Acquire),
            completed: self.clock.completed.load(Ordering::Acquire),
        }
    }

    /// Whether a restore is applying on the calling thread. While it is,
    /// this thread's writes record nothing and baselines follow them.
    pub fn applying(&self) -> bool {
        let address = self.clock.address();
        MARKS.with(|marks| marks.borrow().iter().any(|(clock, _)| *clock == address))
    }

    /// Whether `self` and `other` read the same clock.
    pub fn ptr_eq(&self, other: &RestoreReader) -> bool {
        Arc::ptr_eq(&self.clock, &other.clock)
    }

    /// Holds off every [`Load`](crate::Load)'s writes until it drops.
    ///
    /// Each load write marks a restore stored under this hold, so a replay
    /// step that checks the stored count under it and writes only while the
    /// count is unmoved never overwrites a value loaded since its walk began.
    ///
    /// Hold it only around parameter writes. They take no other lock, so this
    /// stays a leaf lock; holding it around anything else can deadlock with a
    /// load.
    ///
    /// # Panics
    ///
    /// In a build with `debug_assertions`, while the thread holds an
    /// [`audio_thread`](crate::audio_thread) mark.
    pub fn hold_loads(&self) -> LoadHold<'_> {
        LoadHold {
            _held: self.clock.hold_loads(),
        }
    }

    /// Takes the next number in the one sequence every recordable change takes.
    ///
    /// A history takes one as each tick starts and records only the changes
    /// numbered before it, so a change another thread makes mid-tick waits for
    /// the next tick, in order.
    pub fn sequence(&self) -> u64 {
        self.clock.sequence.fetch_add(1, Ordering::Relaxed)
    }
}

/// Holds off loads while alive; see [`RestoreReader::hold_loads`].
#[doc(hidden)]
#[derive(Debug)]
#[must_use = "loads wait only while it is held"]
pub struct LoadHold<'a> {
    _held: MutexGuard<'a, ()>,
}

/// A commit with the clock's counts when it happened and its place in the one
/// sequence. Undo can then drop one made before a restore and record the rest
/// in time order.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub struct Stamped<C> {
    /// The counts when it happened.
    pub stamp: RestoreStamp,
    /// When it happened, against every other change on the same clock.
    pub sequence: u64,
    /// What happened.
    pub commit: C,
}

/// The clock's two counts at one moment.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RestoreStamp {
    stored: u64,
    completed: u64,
}

impl RestoreStamp {
    /// How many restores had been stored.
    pub fn stored(self) -> u64 {
        self.stored
    }

    /// How many restore passes had completed.
    pub fn completed(self) -> u64 {
        self.completed
    }
}
