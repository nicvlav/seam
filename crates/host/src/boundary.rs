//! What every exporter needs where a host calls in, written once: the panic
//! [`guard`] outside any instance (an instance's own entry points use its
//! [`Faults`](crate::faults::Faults)), [`FlushDenormals`], the
//! [`MainThread`] state, the [`AudioSlot`], the in-place [`Channels`], and
//! the host's window an editor opens in, [`ParentWindow`].

#[cfg(loom)]
use loom::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
#[cfg(loom)]
use loom::thread::{ThreadId, current};
use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::ops::{Deref, DerefMut};
use std::panic::{self, AssertUnwindSafe};
#[cfg(not(loom))]
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
#[cfg(not(loom))]
use std::thread::{self, ThreadId, current};
use std::time::{Duration, Instant};

use crate::faults::{Fault, record_fault};

mod channels;
mod window;

pub(crate) use channels::Channels;
pub use window::ParentWindow;

/// Runs an entry point's body outside any instance, answering `refused` if
/// it panics: a panic must never unwind into the host, which would abort
/// it. Not for the audio thread, since a fault here is reported at once.
pub(crate) fn guard<R>(refused: R, body: impl FnOnce() -> R) -> R {
    panic::catch_unwind(AssertUnwindSafe(body)).unwrap_or_else(|_| {
        record_fault(Fault::Panic);
        refused
    })
}

/// Flushes denormals to zero while it lives, and restores the host's setting
/// when dropped.
///
/// The flags are FTZ and DAZ on x86 and FZ on AArch64, as JUCE's
/// `ScopedNoDenormals` sets them. Elsewhere, and under Miri, it does nothing.
#[derive(Debug)]
pub(crate) struct FlushDenormals {
    previous: u64,
}

impl FlushDenormals {
    /// Sets the flush, remembering the host's setting.
    pub(crate) fn new() -> Self {
        let previous = control::read();
        control::write(previous | control::FLUSH);
        FlushDenormals { previous }
    }
}

impl Drop for FlushDenormals {
    fn drop(&mut self) {
        control::write(self.previous);
    }
}

#[cfg(all(
    not(miri),
    any(
        target_arch = "x86_64",
        all(target_arch = "x86", target_feature = "sse")
    )
))]
mod control {
    use std::arch::asm;

    /// MXCSR's flush-to-zero (bit 15) and denormals-are-zero (bit 6).
    pub(super) const FLUSH: u64 = 0x8040;

    pub(super) fn read() -> u64 {
        let mut mxcsr: u32 = 0;
        // SAFETY: `stmxcsr` stores the 32-bit MXCSR register at the address,
        // which is a live, aligned `u32`, and changes nothing else.
        unsafe {
            asm!("stmxcsr [{}]", in(reg) &raw mut mxcsr, options(nostack, preserves_flags));
        }
        u64::from(mxcsr)
    }

    pub(super) fn write(value: u64) {
        let mxcsr = value as u32;
        // SAFETY: `ldmxcsr` loads MXCSR from a live, aligned `u32` holding a
        // value read from MXCSR with only the flush bits added, so no
        // reserved bit is set. Rust assumes the default floating-point
        // environment wherever its code runs. Setting the flush leaves it,
        // as every Rust plugin framework does around `process`. The flush
        // changes only results that would be subnormal, which the processor
        // must tolerate as zeros. The drop restores the host's setting, on
        // unwind too. Without `nomem` or `readonly`, the compiler treats the
        // asm as reading and writing any memory, so it moves no load or
        // store across it. The block's loads and stores, and the arithmetic
        // on what they load, stay on their side of it. Arithmetic on
        // registers alone is not ordered, and needs no flush. The options
        // leave out `preserves_flags`, since the load also sets MXCSR's
        // exception flags back to what they were when the value was read.
        unsafe {
            asm!("ldmxcsr [{}]", in(reg) &raw const mxcsr, options(nostack));
        }
    }
}

#[cfg(all(not(miri), target_arch = "aarch64"))]
mod control {
    use std::arch::asm;

    /// FPCR's flush-to-zero bit (FZ, bit 24).
    pub(super) const FLUSH: u64 = 1 << 24;

    pub(super) fn read() -> u64 {
        let fpcr: u64;
        // SAFETY: `mrs` copies FPCR into a register; it touches no memory,
        // stack or flags, as the options declare.
        unsafe {
            asm!("mrs {}, fpcr", out(reg) fpcr, options(nomem, nostack, preserves_flags));
        }
        fpcr
    }

    pub(super) fn write(value: u64) {
        // SAFETY: `msr` writes back a value read from FPCR with only FZ
        // added. Like x86's write (see there), it leaves the default
        // floating-point environment Rust assumes, and leaves `nomem` off, so
        // the asm is ordered against the block's memory accesses. FPCR holds
        // none of the flags `preserves_flags` covers: the exception flags are
        // FPSR's.
        unsafe {
            asm!("msr fpcr, {}", in(reg) value, options(nostack, preserves_flags));
        }
    }
}

#[cfg(not(any(
    all(
        not(miri),
        any(
            target_arch = "x86_64",
            all(target_arch = "x86", target_feature = "sse")
        )
    ),
    all(not(miri), target_arch = "aarch64")
)))]
mod control {
    pub(super) const FLUSH: u64 = 0;

    pub(super) fn read() -> u64 {
        0
    }

    pub(super) fn write(_: u64) {}
}

/// State one thread at a time may touch.
///
/// A plugin need not be `Send`, yet the object that owns it is shared with
/// every thread a host calls from. Which call takes the state, and how, is
/// the threading rule on `Engine`.
///
/// - State made [`bound`](Self::bound) belongs to the thread that made it,
///   and [`lend`](Self::lend) hands it out there alone. Dropped on any
///   other thread, it leaks rather than drop there.
/// - State made [`unbound`](Self::unbound) is lent to one thread at a time,
///   whichever asks, until that thread's loans end. `lend` refuses every
///   other thread meanwhile; [`lend_or_wait`](Self::lend_or_wait) has such
///   a thread wait its turn, as the state's [`Patience`] says.
/// - A thread that has it on loan may borrow it again, as a nested call
///   does.
pub(crate) struct MainThread<T> {
    binding: Binding,
    value: ManuallyDrop<T>,
}

/// Who may touch [`MainThread`]'s state.
#[derive(Debug)]
enum Binding {
    /// This thread, for good.
    Bound(ThreadId),
    /// One thread at a time, each for as long as its loans last.
    Unbound {
        loans: Loans,
        /// How a call waits for a loan another thread holds.
        patience: &'static dyn Patience,
    },
}

/// Who has unbound state on loan, and how many loans that thread has open.
#[derive(Debug)]
struct Loans {
    /// [`NO_HOLDER`], or the [`here`] of the thread that has the state on loan.
    ///
    /// A loan takes it from `NO_HOLDER` with a compare-and-swap that acquires,
    /// and its last drop gives it back with a store that releases: each
    /// holder's accesses come after the last's.
    holder: AtomicUsize,
    /// How many loans the holder has open. Only the holder touches it,
    /// while it holds, so the loan's own orderings order it and it needs
    /// none of its own.
    depth: AtomicUsize,
}

/// No thread has unbound state on loan.
const NO_HOLDER: usize = 0;

/// Why [`MainThread`] refused the calling thread its state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refused {
    /// Another thread owns it.
    Elsewhere,
    /// No thread owns it, and another thread has it on loan just now.
    Lent,
}

/// How a call waits for unbound state that another thread has on loan, in
/// [`MainThread::lend_or_wait`]. A [`Duration`] is the patience itself.
pub(crate) trait Patience: Sync + std::fmt::Debug {
    /// The longest a call waits, counted from its first refusal.
    fn patience(&self) -> Duration;

    /// Hears that a call was refused once and begins to wait. Does nothing,
    /// unless a test must meet the call while it waits.
    fn waiting(&self) {}
}

impl Patience for Duration {
    fn patience(&self) -> Duration {
        *self
    }
}

#[cfg(not(loom))]
thread_local! {
    /// A byte of each thread's own, whose address names the thread.
    static HERE: u8 = const { 0 };
}

#[cfg(loom)]
loom::thread_local! {
    static HERE: u8 = 0;
}

/// A number naming the calling thread, which no other thread alive shares,
/// and never [`NO_HOLDER`]: the address of the thread's own byte.
fn here() -> usize {
    HERE.with(|byte| std::ptr::from_ref(byte).addr())
}

// SAFETY: the value is reached only through `lend` and `owned`, and dropped
// only in `Drop`. Bound state is handed to its owner thread alone and dropped
// only there. Unbound state is handed to one thread at a time. The loan's
// compare-and-swap lets one thread hold it, and lets only that thread take a
// nested loan. The acquire that takes it and the release that gives it back
// order each holder's accesses after the last's. `owned` never hands unbound
// state out. Unbound state is dropped where the wrapper drops, after every loan
// has ended (`&mut self`), ordered after the last by an acquire. So `T` is
// touched by one thread at a time: its owner's or, for unbound state, each
// holder's in turn. `unbound`'s contract makes that sound.
unsafe impl<T> Sync for MainThread<T> {}
// SAFETY: as above: moving the wrapper moves no access to `T` with it.
unsafe impl<T> Send for MainThread<T> {}

impl<T> MainThread<T> {
    /// Binds `value` to the calling thread, for good.
    pub(crate) fn bound(value: T) -> Self {
        MainThread {
            binding: Binding::Bound(current().id()),
            value: ManuallyDrop::new(value),
        }
    }

    /// Holds `value` bound to no thread, for good.
    ///
    /// [`lend`](Self::lend) hands it to one thread at a time, and it drops on
    /// whichever thread drops the wrapper. A call that waits its turn waits as
    /// `patience` says.
    ///
    /// # Safety
    ///
    /// Handing `value` from thread to thread, one at a time, and dropping
    /// it on any thread is sound. Every part of it that is not `Send` is one
    /// that nothing ever touches, and that nothing outside it shares.
    #[cfg_attr(
        not(any(feature = "cabi", test)),
        expect(
            dead_code,
            reason = "only the C interface makes state bound to no thread"
        )
    )]
    pub(crate) unsafe fn unbound(value: T, patience: &'static dyn Patience) -> Self {
        MainThread {
            binding: Binding::Unbound {
                loans: Loans {
                    holder: AtomicUsize::new(NO_HOLDER),
                    depth: AtomicUsize::new(0),
                },
                patience,
            },
            value: ManuallyDrop::new(value),
        }
    }

    /// Whether the state is bound to the calling thread. The call binds
    /// nothing.
    pub(crate) fn is_owner(&self) -> bool {
        matches!(self.binding, Binding::Bound(owner) if owner == current().id())
    }

    /// The state, when it is bound to the calling thread; `None` for state
    /// made unbound, on any thread. The call binds nothing and lends
    /// nothing.
    #[cfg_attr(
        not(any(feature = "vst3", test)),
        expect(
            dead_code,
            reason = "only the VST3 shell starts a timer on its owner thread"
        )
    )]
    pub(crate) fn owned(&self) -> Option<&T> {
        self.is_owner().then_some(&*self.value)
    }

    /// The state, on the owner thread of bound state, or for unbound state
    /// on the calling thread until its last loan drops.
    pub(crate) fn lend(&self) -> Result<Lent<'_, T>, Refused> {
        let loans = match &self.binding {
            Binding::Bound(owner) if *owner == current().id() => None,
            Binding::Bound(_) => return Err(Refused::Elsewhere),
            Binding::Unbound { loans, .. } => {
                loans.open()?;
                Some(loans)
            }
        };
        Ok(Lent {
            value: &self.value,
            loans,
            here: PhantomData,
        })
    }

    /// As [`lend`](Self::lend), but unbound state another thread has on loan is
    /// waited for, as long as its [`Patience`] allows.
    ///
    /// Once let go, the state is taken; past the patience, it is refused as
    /// `lend` refuses it. Bound state on another thread is refused at once,
    /// since it never comes back. Each try is a `lend`, so the loan's orderings
    /// are its own.
    pub(crate) fn lend_or_wait(&self) -> Result<Lent<'_, T>, Refused> {
        let Binding::Unbound { patience, .. } = &self.binding else {
            return self.lend();
        };
        let mut wait = None;
        loop {
            match self.lend() {
                Err(Refused::Lent) => {}
                answer => return answer,
            }
            let pace = wait.get_or_insert_with(|| {
                patience.waiting();
                Wait::new(patience.patience())
            });
            if !pace.again() {
                return Err(Refused::Lent);
            }
        }
    }
}

impl Loans {
    /// Opens a loan to the calling thread: the first takes the state, and a
    /// nested one counts in its depth. Refused while another thread holds
    /// it.
    fn open(&self) -> Result<(), Refused> {
        let here = here();
        match self
            .holder
            .compare_exchange(NO_HOLDER, here, Ordering::Acquire, Ordering::Relaxed)
        {
            Ok(_) => self.depth.store(1, Ordering::Relaxed),
            // Only this thread stores its own `here`, and it clears it as
            // its last loan drops, so it reads it back while one is open. A
            // thread's loans end before it does: `Lent` is not `Send`, and
            // no code in this crate leaks one (by `mem::forget` or a cycle).
            // So the `here` of a thread gone, which a new one may get, is
            // never left behind.
            Err(holder) if holder == here => {
                self.depth.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => return Err(Refused::Lent),
        }
        Ok(())
    }

    /// Closes one loan; the holder's last gives the state back.
    fn close(&self) {
        if self.depth.fetch_sub(1, Ordering::Relaxed) == 1 {
            self.holder.store(NO_HOLDER, Ordering::Release);
        }
    }
}

/// Paces a wait for a loan, so a long one costs no processor time.
///
/// It spins a few times, for a holder about to let go, then yields a few times,
/// then sleeps a millisecond at a time. It stops once the patience has passed
/// since it was made, at the call's first refusal: a call that takes the state
/// at once never reads the clock.
struct Wait {
    tries: u32,
    deadline: Instant,
}

impl Wait {
    const SPINS: u32 = 64;
    const YIELDS: u32 = 16;

    fn new(patience: Duration) -> Self {
        Wait {
            tries: 0,
            deadline: Instant::now() + patience,
        }
    }

    /// Pauses before the next try, or answers false once the patience has
    /// passed.
    fn again(&mut self) -> bool {
        if Instant::now() >= self.deadline {
            return false;
        }
        self.tries = self.tries.saturating_add(1);
        if self.tries <= Self::SPINS {
            std::hint::spin_loop();
        } else if self.tries <= Self::SPINS + Self::YIELDS {
            std::thread::yield_now();
        } else {
            std::thread::sleep(Duration::from_millis(1));
        }
        true
    }
}

impl<T> Drop for MainThread<T> {
    fn drop(&mut self) {
        let here = match &self.binding {
            Binding::Bound(owner) => *owner == current().id(),
            // Every loan has ended, as `&mut self` shows; the acquire orders
            // this drop after the last holder's release.
            Binding::Unbound { loans, .. } => loans.holder.load(Ordering::Acquire) == NO_HOLDER,
        };
        if here {
            // SAFETY: bound state on its owner thread, or unbound state with
            // no loan out and ordered after the last (see `Sync`).
            // `unbound`'s contract lets unbound state drop here. `drop` runs
            // once, and the value is not used after it.
            unsafe { ManuallyDrop::drop(&mut self.value) };
        }
    }
}

impl<T> std::fmt::Debug for MainThread<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MainThread")
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}

/// [`MainThread`]'s state as [`lend`](MainThread::lend) hands it out. A
/// loan of unbound state ends as the holder's last `Lent` drops, in any
/// order.
pub(crate) struct Lent<'a, T> {
    value: &'a T,
    /// The loans this one counts in, for unbound state; `None` for bound
    /// state, which its owner thread holds without one.
    loans: Option<&'a Loans>,
    /// Stays on the thread it was handed to: neither `Send` nor `Sync`.
    here: PhantomData<*const ()>,
}

impl<T> Deref for Lent<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.value
    }
}

impl<T> Drop for Lent<'_, T> {
    fn drop(&mut self) {
        if let Some(loans) = self.loans {
            loans.close();
        }
    }
}

const FREE: u8 = 0;
const TAKEN: u8 = 1;

/// What the audio thread works with, handed between it and the main
/// thread.
///
/// The audio thread takes it with one compare-and-swap and never waits: if
/// the main thread holds it, the block plays silence. The main thread
/// waits for a running block to finish. A host never runs `process` while
/// it activates or deactivates, so neither waits in practice; the slot
/// keeps a host that does from racing.
pub(crate) struct AudioSlot<T> {
    state: AtomicU8,
    value: UnsafeCell<T>,
}

// SAFETY: the value is reached only through a `SlotGuard`, and the state's
// compare-and-swap lets one guard exist at a time. Its acquire and the
// guard's release store order each holder's accesses after the last's.
// Holders on different threads take the value in turn, so `T` must be
// `Send`, and need not be `Sync`, since no two hold it at once.
unsafe impl<T: Send> Sync for AudioSlot<T> {}

impl<T> AudioSlot<T> {
    pub(crate) fn new(value: T) -> Self {
        AudioSlot {
            state: AtomicU8::new(FREE),
            value: UnsafeCell::new(value),
        }
    }

    /// The value, if no one else holds it. Wait-free, for the audio thread.
    pub(crate) fn try_hold(&self) -> Option<SlotGuard<'_, T>> {
        self.state
            .compare_exchange(FREE, TAKEN, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| SlotGuard {
                slot: self,
                value: PhantomData,
            })
    }

    /// The value, once the holder lets go. For the main thread.
    pub(crate) fn hold(&self) -> SlotGuard<'_, T> {
        loop {
            if let Some(guard) = self.try_hold() {
                return guard;
            }
            #[cfg(loom)]
            loom::thread::yield_now();
            #[cfg(not(loom))]
            thread::yield_now();
        }
    }
}

impl<T> std::fmt::Debug for AudioSlot<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioSlot").finish_non_exhaustive()
    }
}

/// Holds an [`AudioSlot`]'s value until dropped.
pub(crate) struct SlotGuard<'a, T> {
    slot: &'a AudioSlot<T>,
    /// The guard lends the value as `&mut T` does, so it is `Sync` only
    /// when `T` is.
    value: PhantomData<&'a mut T>,
}

impl<T> Deref for SlotGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: while this guard lives the slot's state is taken, by the
        // compare-and-swap that made it, so no other guard and no other
        // reference to the value exists. That swap's acquire ordered this
        // access after the last holder's release. The reference lives no
        // longer than `&self`.
        unsafe { &*self.slot.value.get() }
    }
}

impl<T> DerefMut for SlotGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: as in `deref`, and `&mut self` also rules out a shared
        // borrow through this guard, so this is the only reference.
        unsafe { &mut *self.slot.value.get() }
    }
}

impl<T> Drop for SlotGuard<'_, T> {
    fn drop(&mut self) {
        self.slot.state.store(FREE, Ordering::Release);
    }
}

#[cfg(test)]
pub(crate) mod tests;

#[cfg(all(test, loom))]
mod loom_models;
