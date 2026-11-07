//! The zero-allocation gate.
//!
//! Code the audio thread runs never allocates or frees. A gate test runs that
//! code inside [`assert_no_allocations`]. The count is the calling thread's
//! own, so tests running in parallel never see each other's allocations; run
//! the gated code on the thread that calls the gate.
//!
//! The gate works only in a test binary whose global allocator is
//! [`AllocDisabler`], which [`install_alloc_gate!`](crate::install_alloc_gate)
//! installs. It counts allocations, not locks or system calls
//! (`docs/testing.md` says what covers those).

use std::sync::atomic::{AtomicBool, Ordering};

pub use assert_no_alloc::{AllocDisabler, permit_alloc};

/// Installs [`AllocDisabler`] as the test binary's global allocator.
///
/// Put it once at the top level of each test file that runs a gate, since
/// each file is a binary of its own.
///
/// # Examples
///
/// ```
/// seam_testkit::install_alloc_gate!();
///
/// fn main() {
///     let (_, count) = seam_testkit::alloc::allocations_in(|| ());
///     assert_eq!(count, 0);
/// }
/// ```
#[macro_export]
macro_rules! install_alloc_gate {
    () => {
        #[global_allocator]
        static SEAM_ALLOC_GATE: $crate::alloc::AllocDisabler = $crate::alloc::AllocDisabler;
    };
}

/// Set once the installation probe has seen the gate count an allocation.
static ARMED: AtomicBool = AtomicBool::new(false);

/// Runs `f` and returns its result with the number of allocations and frees
/// it made on this thread.
///
/// # Panics
///
/// If the binary's global allocator is not [`AllocDisabler`]: the count
/// would read zero whatever `f` did.
///
/// # Examples
///
/// ```
/// use seam_testkit::alloc::allocations_in;
///
/// seam_testkit::install_alloc_gate!();
///
/// fn main() {
///     // Code the audio thread runs: nothing to allocate.
///     let (sum, allocations) = allocations_in(|| [1.0_f32, 2.0, 3.0].iter().sum::<f32>());
///     assert_eq!((sum, allocations), (6.0, 0));
///
///     // A vector that grows shows up in the count.
///     let (_, allocations) = allocations_in(|| vec![0.0_f32; 64]);
///     assert!(allocations > 0);
/// }
/// ```
pub fn allocations_in<T>(f: impl FnOnce() -> T) -> (T, u32) {
    assert_armed();
    let before = assert_no_alloc::violation_count();
    let out = assert_no_alloc::assert_no_alloc(f);
    (out, assert_no_alloc::violation_count() - before)
}

/// Runs `f` and fails the test if it allocated or freed on this thread.
///
/// # Panics
///
/// If `f` allocated or freed, naming `what`; or if the gate is not installed.
///
/// # Examples
///
/// ```
/// use seam_testkit::alloc::assert_no_allocations;
///
/// seam_testkit::install_alloc_gate!();
///
/// fn main() {
///     let mut block = [0.5_f32; 64];
///     assert_no_allocations("the gain stage", || {
///         for sample in &mut block {
///             *sample *= 0.5;
///         }
///     });
///     assert_eq!(block[0], 0.25);
/// }
/// ```
pub fn assert_no_allocations<T>(what: &str, f: impl FnOnce() -> T) -> T {
    let (out, count) = allocations_in(f);
    assert_eq!(
        count, 0,
        "{what}: {count} allocation(s) or free(s) where none may happen"
    );
    out
}

/// Proves the gate counts: an allocation made on purpose must register.
fn assert_armed() {
    if ARMED.load(Ordering::Relaxed) {
        return;
    }
    let before = assert_no_alloc::violation_count();
    assert_no_alloc::assert_no_alloc(|| drop(std::hint::black_box(Box::new(0u64))));
    assert!(
        assert_no_alloc::violation_count() > before,
        "the allocation gate is not installed: add `seam_testkit::install_alloc_gate!();` to this test file"
    );
    ARMED.store(true, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    // This unit-test binary keeps the system allocator, which is the mistake
    // the probe exists to catch.
    #[test]
    #[should_panic(expected = "the allocation gate is not installed")]
    fn refuses_to_count_without_the_gate() {
        super::allocations_in(|| ());
    }
}
