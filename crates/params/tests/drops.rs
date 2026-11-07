//! What a list or a plain value lets go of drops after its lock is
//! released, so an element's `Drop` may call the list or value it sat in.

use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use seam_params::{List, Origin, ParamSet, Value};
use seam_testkit::sync::PATIENCE;

/// What an element's `Drop` calls.
type Call = Arc<dyn Fn() + Send + Sync>;

/// An element whose `Drop` calls the list or value it sits in, through the
/// call its clones share.
#[derive(Clone, Default)]
struct Reaches(Arc<Mutex<Option<Call>>>);

impl Reaches {
    fn reach(&self, call: impl Fn() + Send + Sync + 'static) {
        *self.slot() = Some(Arc::new(call));
    }

    /// Stops calling, so the list or value the call holds can drop.
    fn stop(&self) {
        // Taken out first, so the handle it holds drops with the slot free.
        let call = self.slot().take();
        drop(call);
    }

    fn slot(&self) -> MutexGuard<'_, Option<Call>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for Reaches {
    fn drop(&mut self) {
        let call = self.slot().clone();
        if let Some(call) = call {
            call();
        }
    }
}

/// A [`Reaches`] whose clones can be set to panic.
struct Brittle {
    reaches: Reaches,
    /// Clones left before one panics.
    clones: Arc<AtomicI32>,
}

impl Brittle {
    fn new() -> Self {
        Brittle {
            reaches: Reaches::default(),
            clones: Arc::new(AtomicI32::new(i32::MAX)),
        }
    }

    /// The clone after the next `clones` panics.
    fn panic_after(&self, clones: i32) {
        self.clones.store(clones, Ordering::SeqCst);
    }
}

impl Clone for Brittle {
    fn clone(&self) -> Self {
        assert_ne!(
            self.clones.fetch_sub(1, Ordering::SeqCst),
            0,
            "a clone that panics"
        );
        Brittle {
            reaches: self.reaches.clone(),
            clones: Arc::clone(&self.clones),
        }
    }
}

/// Runs `steps` on a thread of its own and fails unless it finishes. An
/// element dropped under its list's or value's lock calls back into that
/// lock and deadlocks there; that thread is then left behind.
fn finishes(what: &str, steps: impl FnOnce() + Send + 'static) {
    let (done, finished) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        steps();
        let _ = done.send(());
    });
    match finished.recv_timeout(PATIENCE) {
        Ok(()) | Err(RecvTimeoutError::Disconnected) => {
            if let Err(panic) = worker.join() {
                std::panic::resume_unwind(panic);
            }
        }
        Err(RecvTimeoutError::Timeout) => {
            panic!("{what} deadlocked: an element dropped under the lock")
        }
    }
}

#[test]
fn every_element_a_list_lets_go_of_drops_after_the_unlock() {
    finishes("a list", || {
        let set = ParamSet::from_decls([]).expect("valid");
        let reaches = Reaches::default();
        let list = List::new(vec![reaches.clone(); 8], &set);
        let handle = list.clone();
        reaches.reach(move || {
            let _ = handle.len();
        });

        // Untracked, a verb's element drops at once, derived or not.
        assert!(list.remove(0));
        assert!(list.derived(Origin::Host).remove(0));
        // Tracked, a removed element waits in the queue until a verb that
        // records nothing, a reset, a stale replay or the tracker's end
        // drops it.
        let tracker = list.take_tracker().expect("untracked");
        assert!(list.remove(0));
        list.derived(Origin::Host).push(reaches.clone());
        assert!(list.remove(0));
        list.reset(vec![reaches.clone(); 4]);
        let (_, replay) = tracker.take();
        assert!(list.remove(0));
        assert!(replay.remove(0).is_none(), "the waiting verb made it stale");
        assert!(list.remove(0));
        drop(tracker);
        // A clear's elements, untracked.
        assert!(list.clear());
        reaches.stop();
    });
}

#[test]
fn every_value_a_plain_value_lets_go_of_drops_after_the_unlock() {
    finishes("a plain value", || {
        let set = ParamSet::from_decls([]).expect("valid");
        let writer = set.clock().writer();
        let reaches = Reaches::default();
        let value = Value::new(reaches.clone(), &set);
        let handle = value.clone();
        reaches.reach(move || {
            let _ = handle.gesture_open();
        });

        // The value a write replaces, and an untracked gesture's after-values
        // and, at its close, its before and after.
        value.set(reaches.clone());
        let drag = value.begin();
        drag.set(reaches.clone());
        drag.set(reaches.clone());
        value.set_from(reaches.clone(), Origin::Program);
        drop(drag);
        // A restore during a gesture replaces its before and after.
        let drag = value.begin();
        drag.set(reaches.clone());
        {
            let _applying = writer.applying();
            value.set(reaches.clone());
        }
        drop(drag);
        // Tracked: a coalesced write, the commits a replay drops, and the
        // queue the tracker's end drops.
        let tracker = value.take_tracker().expect("untracked");
        value.set(reaches.clone());
        value.set(reaches.clone());
        assert!(tracker.downgrade().replay(reaches.clone()));
        value.set(reaches.clone());
        drop(tracker);
        reaches.stop();
    });
}

#[test]
fn what_a_write_replaced_drops_after_the_unlock_when_a_clone_panics() {
    finishes("a plain value", || {
        let set = ParamSet::from_decls([]).expect("valid");
        let writer = set.clock().writer();
        let element = Brittle::new();
        let value = Value::new(element.clone(), &set);
        let handle = value.clone();
        element.reaches.reach(move || {
            let _ = handle.gesture_open();
        });

        // A restore during a gesture clones twice: the stored value, then
        // the gesture's new before. The second panics with the replaced
        // value already taken out.
        let drag = value.begin();
        let next = element.clone();
        element.panic_after(1);
        let unwound = {
            let _applying = writer.applying();
            panic::catch_unwind(AssertUnwindSafe(|| value.set(next))).is_err()
        };
        element.panic_after(i32::MAX);
        assert!(unwound, "the second clone panicked");
        drop(drag);
        element.reaches.stop();
    });
}
