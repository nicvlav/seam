use super::*;
use crate::faults::thread_faults;
use seam_testkit::sync::{PATIENCE, wait_until};
use std::thread;

/// A patience that counts the calls that began to wait, for a test that
/// must meet one while it waits. Its count is std's atomic, even where loom
/// swaps the boundary's own.
#[derive(Debug)]
pub(crate) struct Watched {
    patience: Duration,
    waits: std::sync::atomic::AtomicUsize,
}

impl Watched {
    pub(crate) const fn new(patience: Duration) -> Self {
        Watched {
            patience,
            waits: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// How many calls were refused and began to wait.
    pub(crate) fn waits(&self) -> usize {
        self.waits.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl Patience for Watched {
    fn patience(&self) -> Duration {
        self.patience
    }

    fn waiting(&self) {
        self.waits
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

#[test]
fn a_panic_is_answered_and_recorded() {
    let before = thread_faults();
    assert_eq!(guard(-1, || -> i32 { panic!("inside an entry point") }), -1);
    assert_eq!(thread_faults(), before + 1);
    assert_eq!(guard(-1, || 7), 7);
    assert_eq!(thread_faults(), before + 1);
}

#[test]
fn main_thread_state_is_refused_on_another_thread() {
    let main = MainThread::bound(std::rc::Rc::new(5));
    assert_eq!(main.lend().map(|value| **value), Ok(5));
    assert!(main.is_owner() && main.owned().is_some());
    thread::scope(|scope| {
        scope.spawn(|| {
            assert_eq!(main.lend().err(), Some(Refused::Elsewhere));
            assert!(!main.is_owner() && main.owned().is_none());
        });
    });
}

#[test]
fn the_audio_side_never_waits_for_a_holder() {
    let slot = AudioSlot::new(1);
    let held = slot.hold();
    assert!(slot.try_hold().is_none());
    drop(held);
    *slot.try_hold().expect("free") += 1;
    assert_eq!(*slot.hold(), 2);
}

#[test]
fn denormals_flush_inside_and_not_after() {
    let tiny = std::hint::black_box(f32::MIN_POSITIVE);
    let half = std::hint::black_box(0.5f32);
    assert!(
        (tiny * half).is_subnormal(),
        "the host's setting keeps them"
    );
    {
        let _flush = FlushDenormals::new();
        let flushed = std::hint::black_box(tiny) * std::hint::black_box(half);
        if cfg!(all(
            not(miri),
            any(target_arch = "x86_64", target_arch = "aarch64")
        )) {
            assert_eq!(flushed, 0.0);
        }
    }
    assert!((tiny * half).is_subnormal(), "restored after");
}

#[test]
fn a_denormal_input_reads_as_zero_inside() {
    let tiny = std::hint::black_box(1.0e-40f32);
    assert!(tiny.is_subnormal());
    let _flush = FlushDenormals::new();
    let product = std::hint::black_box(tiny) * std::hint::black_box(1.0e20f32);
    if cfg!(all(not(miri), target_arch = "x86_64")) {
        assert_eq!(product, 0.0, "DAZ, not only FTZ");
    }
}

#[test]
fn unbound_state_is_lent_to_any_thread_for_good() {
    // SAFETY: an `i32` moves between threads freely.
    let main = unsafe { MainThread::unbound(5, &PATIENCE) };
    for _ in 0..3 {
        assert_eq!(main.lend().as_deref(), Ok(&5), "here");
        thread::scope(|scope| {
            scope.spawn(|| assert_eq!(main.lend().as_deref(), Ok(&5), "and there"));
        });
    }
    assert!(!main.is_owner() && main.owned().is_none(), "owned by none");
}

#[test]
fn unbound_state_lent_out_is_refused_elsewhere_and_lent_again_on_its_thread() {
    // SAFETY: an `i32` moves between threads freely.
    let main = unsafe { MainThread::unbound(5, &PATIENCE) };
    let outer = main.lend().expect("lent here");
    let inner = main.lend().expect("lent again here, as a nested call");
    // The loan lasts until the last one drops, whatever the order.
    drop(outer);
    thread::scope(|scope| {
        scope.spawn(|| assert_eq!(main.lend().err(), Some(Refused::Lent)));
    });
    drop(inner);
    thread::scope(|scope| {
        scope.spawn(|| assert_eq!(main.lend().as_deref(), Ok(&5), "free once both drop"));
    });
}

#[test]
fn unbound_state_drops_on_whichever_thread_drops_it() {
    let value = std::sync::Arc::new(5);
    // SAFETY: an `Arc` moves between threads freely.
    let main = unsafe { MainThread::unbound(std::sync::Arc::clone(&value), &PATIENCE) };
    thread::scope(|scope| {
        scope.spawn(move || drop(main));
    });
    assert_eq!(std::sync::Arc::strong_count(&value), 1, "dropped there");
}

#[test]
fn a_waiting_loan_takes_unbound_state_once_its_holder_lets_go() {
    static WATCHED: Watched = Watched::new(PATIENCE);
    // SAFETY: an `i32` moves between threads freely.
    let main = unsafe { MainThread::unbound(5, &WATCHED) };
    let held = main.lend().expect("held here");
    thread::scope(|scope| {
        let waiter = scope.spawn(|| main.lend_or_wait().map(|state| *state));
        wait_until("the other thread to wait", || WATCHED.waits() == 1);
        drop(held);
        assert_eq!(waiter.join().expect("the waiter"), Ok(5));
    });
}

#[test]
fn a_waiting_loan_is_refused_once_its_patience_runs_out() {
    static WATCHED: Watched = Watched::new(Duration::from_millis(5));
    // SAFETY: an `i32` moves between threads freely.
    let main = unsafe { MainThread::unbound(5, &WATCHED) };
    let _held = main.lend().expect("held here, throughout");
    thread::scope(|scope| {
        scope.spawn(|| {
            let refused = main.lend_or_wait().err();
            assert_eq!(refused, Some(Refused::Lent));
        });
    });
    assert_eq!(WATCHED.waits(), 1);
}

#[test]
fn a_loan_taken_at_once_begins_no_wait() {
    static WATCHED: Watched = Watched::new(PATIENCE);
    // SAFETY: an `i32` moves between threads freely.
    let main = unsafe { MainThread::unbound(5, &WATCHED) };
    let outer = main.lend_or_wait().expect("free");
    let inner = main
        .lend_or_wait()
        .expect("lent again here, as a nested call");
    assert_eq!((*outer, *inner), (5, 5));
    assert_eq!(WATCHED.waits(), 0, "and so read no clock");
}

#[test]
fn bound_state_on_another_thread_is_refused_without_a_wait() {
    let main = MainThread::bound(std::rc::Rc::new(5));
    thread::scope(|scope| {
        scope.spawn(|| {
            let refused = main.lend_or_wait().err();
            assert_eq!(refused, Some(Refused::Elsewhere), "it never comes back");
        });
    });
}

#[test]
fn main_thread_state_dropped_on_another_thread_is_leaked() {
    let value = std::rc::Rc::new(5);
    let main = MainThread::bound(std::rc::Rc::clone(&value));
    thread::scope(|scope| {
        scope.spawn(move || drop(main));
    });
    assert_eq!(std::rc::Rc::strong_count(&value), 2, "never dropped there");
}
