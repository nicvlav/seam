//! Waiting in real-thread tests.
//!
//! A real-thread test proves an ordering by making the threads meet, not by
//! sleeping and hoping the other thread got there: one side counts a
//! [`Latch`] down while the other waits for it to open, or a side waits for a
//! condition with [`wait_until`].
//!
//! Both fail the test after [`PATIENCE`], which only bounds a hang: it is
//! long, so a slow CI runner costs time, never a false failure.

use std::sync::{Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// How long a test waits before deciding the other thread is stuck.
pub const PATIENCE: Duration = Duration::from_secs(10);

/// A count-down latch: [`wait`](Self::wait) returns once
/// [`count_down`](Self::count_down) has been called `count` times.
///
/// # Examples
///
/// ```
/// use std::sync::atomic::{AtomicBool, Ordering};
///
/// use seam_testkit::sync::Latch;
///
/// let ready = Latch::new(1);
/// let handed_over = AtomicBool::new(false);
/// std::thread::scope(|scope| {
///     scope.spawn(|| {
///         handed_over.store(true, Ordering::Release);
///         ready.count_down();
///     });
///     // The test proceeds once the other thread got there, not after a sleep.
///     ready.wait();
///     assert!(handed_over.load(Ordering::Acquire));
/// });
/// ```
#[derive(Debug)]
pub struct Latch {
    remaining: Mutex<usize>,
    reached: Condvar,
}

impl Latch {
    /// Creates a latch that opens after `count` count-downs, or at once for
    /// a `count` of 0.
    pub fn new(count: usize) -> Self {
        Latch {
            remaining: Mutex::new(count),
            reached: Condvar::new(),
        }
    }

    /// Counts down once, and does nothing to an open latch.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_testkit::sync::Latch;
    ///
    /// let latch = Latch::new(2);
    /// latch.count_down();
    /// assert!(!latch.is_open()); // one more to go
    /// latch.count_down();
    /// assert!(latch.is_open());
    /// latch.count_down(); // an open latch stays open
    /// assert!(latch.is_open());
    /// ```
    pub fn count_down(&self) {
        let mut remaining = self
            .remaining
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *remaining = remaining.saturating_sub(1);
        if *remaining == 0 {
            self.reached.notify_all();
        }
    }

    /// Waits for the latch to open, and returns at once when it is open
    /// already.
    ///
    /// # Panics
    ///
    /// If it is still closed after [`PATIENCE`].
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_testkit::sync::Latch;
    ///
    /// // A latch of 0 is open: the wait returns without another thread.
    /// Latch::new(0).wait();
    /// ```
    pub fn wait(&self) {
        let remaining = self
            .remaining
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (remaining, timeout) = self
            .reached
            .wait_timeout_while(remaining, PATIENCE, |remaining| *remaining > 0)
            .unwrap_or_else(PoisonError::into_inner);
        assert!(
            !timeout.timed_out(),
            "latch still waiting on {} count-down(s) after {PATIENCE:?}",
            *remaining
        );
    }

    /// Whether the latch has opened.
    pub fn is_open(&self) -> bool {
        *self
            .remaining
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            == 0
    }
}

/// Waits until `condition` holds, checking it again after each short pause.
///
/// # Panics
///
/// If it still does not hold after [`PATIENCE`], naming `what`.
///
/// # Examples
///
/// ```
/// use std::sync::atomic::{AtomicU32, Ordering};
///
/// use seam_testkit::sync::wait_until;
///
/// let processed = AtomicU32::new(0);
/// std::thread::scope(|scope| {
///     scope.spawn(|| processed.store(3, Ordering::Release));
///     wait_until("three blocks processed", || {
///         processed.load(Ordering::Acquire) == 3
///     });
/// });
/// ```
pub fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let start = Instant::now();
    while !condition() {
        assert!(
            start.elapsed() < PATIENCE,
            "still waiting for {what} after {PATIENCE:?}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn a_latch_opens_after_its_count() {
        let latch = Latch::new(2);
        std::thread::scope(|scope| {
            scope.spawn(|| latch.count_down());
            scope.spawn(|| latch.count_down());
            latch.wait();
        });
        assert!(latch.is_open());
    }

    #[test]
    fn waiting_returns_once_the_condition_holds() {
        let flag = AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| flag.store(true, Ordering::Release));
            wait_until("the flag", || flag.load(Ordering::Acquire));
        });
    }
}
