use loom::cell::UnsafeCell;
use loom::sync::Arc;
use loom::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::{AudioSlot, MainThread, Refused};

/// A processor whose state loom tracks.
struct Held(UnsafeCell<u32>);

/// Unbound main-thread state as two threads' calls take it. A tick tries
/// once. Another call waits for it by trying again, as `lend_or_wait`
/// does. That call then borrows it again inside its own loan and drops
/// the outer loan first, as calls nested out of order would. Whatever
/// the interleaving, the two never touch it at once. The tick is refused
/// only because the other call has it, and the waiting call always gets
/// it. Once both are done, it is free.
#[test]
fn loom_unbound_state_is_lent_to_one_thread_at_a_time() {
    loom::model(|| {
        // SAFETY: a `u32` moves between threads freely.
        let main = Arc::new(unsafe { MainThread::unbound(UnsafeCell::new(0u32), &Duration::ZERO) });
        let other = Arc::clone(&main);
        let tick = loom::thread::spawn(move || match other.lend() {
            Ok(state) => {
                // SAFETY: the loan is the only holder.
                state.with_mut(|value| unsafe { *value += 1 });
                true
            }
            Err(refused) => {
                assert_eq!(refused, Refused::Lent, "never bound");
                false
            }
        });
        let outer = loop {
            match main.lend() {
                Ok(outer) => break outer,
                Err(refused) => {
                    assert_eq!(refused, Refused::Lent, "never bound");
                    loom::thread::yield_now();
                }
            }
        };
        let inner = main.lend().expect("lent again on its own thread");
        drop(outer);
        // SAFETY: the inner loan still holds it.
        inner.with_mut(|value| unsafe { *value += 10 });
        drop(inner);
        let ticked = tick.join().expect("the tick's thread");
        let state = main.lend().expect("free once both are done");
        // SAFETY: the loan is the only holder.
        let value = state.with(|value| unsafe { *value });
        assert_eq!(value, 10 + u32::from(ticked));
    });
}

/// State whose drop reads it, so loom sees a drop that races a loan, and
/// counts the drops.
struct Noted(UnsafeCell<u32>, Arc<AtomicUsize>);

impl Drop for Noted {
    fn drop(&mut self) {
        // SAFETY: `&mut self`, so no loan is left.
        let value = self.0.with(|value| unsafe { *value });
        assert!(value <= 11, "only the two loans wrote it");
        self.1.fetch_add(1, Ordering::Relaxed);
    }
}

/// Unbound main-thread state that two threads share, each taking a loan
/// and then letting go of its share. Whichever thread lets go last drops
/// the state there, the spawned one included. Whatever the interleaving,
/// the drop comes after both loans, never during one, and once.
#[test]
fn loom_unbound_state_drops_on_the_thread_that_lets_it_go_last() {
    loom::model(|| {
        let drops = Arc::new(AtomicUsize::new(0));
        let noted = Noted(UnsafeCell::new(0), Arc::clone(&drops));
        // SAFETY: a `u32` and an `Arc` move between threads freely.
        let main = Arc::new(unsafe { MainThread::unbound(noted, &Duration::ZERO) });
        let other = Arc::clone(&main);
        let thread = loom::thread::spawn(move || {
            if let Ok(state) = other.lend() {
                // SAFETY: the loan is the only holder.
                state.0.with_mut(|value| unsafe { *value += 1 });
            }
            // The last share, in some interleavings: the state drops here.
            drop(other);
        });
        if let Ok(state) = main.lend() {
            // SAFETY: the loan is the only holder.
            state.0.with_mut(|value| unsafe { *value += 10 });
        }
        drop(main);
        thread.join().expect("the other thread");
        assert_eq!(drops.load(Ordering::Relaxed), 1, "dropped once");
    });
}

/// The audio slot: a block's `try_hold` on the audio thread races an
/// activation's `hold` and replace on the main thread. Whatever the
/// interleaving, the two never hold it at once. The block's work lands
/// whole in the old processor, which the main thread gets back, or in
/// the new one. Or it lands nowhere, when the block found the slot held.
#[test]
fn loom_a_block_racing_an_activation_sees_one_whole_processor() {
    loom::model(|| {
        let slot = Arc::new(AudioSlot::new(Some(Held(UnsafeCell::new(1)))));
        let audio = Arc::clone(&slot);
        let block = loom::thread::spawn(move || {
            if let Some(mut held) = audio.try_hold()
                && let Some(processor) = held.as_mut()
            {
                // SAFETY: the guard is the only holder.
                processor.0.with_mut(|state| unsafe { *state += 10 });
            }
        });
        let old = slot.hold().replace(Held(UnsafeCell::new(2)));
        block.join().expect("the audio thread");

        let old = old.expect("the first processor");
        // SAFETY: the block let go before the main thread held it.
        let old = old.0.with(|state| unsafe { *state });
        let mut next = slot.try_hold().expect("free once both let go");
        let next = next.as_mut().expect("the new processor");
        // SAFETY: the guard is the only holder.
        let new = next.0.with(|state| unsafe { *state });
        assert!(
            matches!((old, new), (1, 2) | (11, 2) | (1, 12)),
            "old {old}, new {new}"
        );
    });
}
