//! What a list or a plain value lets go of drops after its lock is
//! released, so an element's `Drop` may call the list or value it sat in;
//! and a typed parameter's store goes with its last holder, on any thread.

use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use seam_params::{Float, Kind, List, Origin, ParamDecl, ParamSet, Value};
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
    /// Tells one write's element from another.
    tag: u32,
}

impl Brittle {
    fn new() -> Self {
        Brittle {
            reaches: Reaches::default(),
            clones: Arc::new(AtomicI32::new(i32::MAX)),
            tag: 0,
        }
    }

    /// Another element sharing this one's call and clone count.
    fn tagged(&self, tag: u32) -> Self {
        Brittle {
            reaches: self.reaches.clone(),
            clones: Arc::clone(&self.clones),
            tag,
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
            tag: self.tag,
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
fn a_write_whose_clone_panics_changes_nothing() {
    finishes("a plain value", || {
        let set = ParamSet::from_decls([]).expect("valid");
        let writer = set.clock().writer();
        let element = Brittle::new();
        let value = Value::new(element.clone(), &set);
        let handle = value.clone();
        element.reaches.reach(move || {
            let _ = handle.gesture_open();
        });

        // A restore during a gesture clones twice: the value it stores, and
        // the gesture's new before. Either may panic.
        let drag = value.begin();
        for clones in [0, 1] {
            let next = element.tagged(1);
            element.panic_after(clones);
            let unwound = {
                let _applying = writer.applying();
                panic::catch_unwind(AssertUnwindSafe(|| value.set(next))).is_err()
            };
            element.panic_after(i32::MAX);
            assert!(unwound, "clone {clones} panicked");
            assert_eq!(value.get().tag, 0, "clone {clones}: the value stands");
            assert_eq!(value.changes(), 0, "clone {clones}: no change");
        }
        drop(drag);
        element.reaches.stop();
    });
}

#[test]
fn the_last_typed_parameter_drops_on_a_worker_after_its_set() {
    let set = ParamSet::from_decls([ParamDecl::new(
        "gain",
        "Gain",
        Kind::continuous(-60.0, 12.0, 0.1, None),
        0.0,
    )])
    .expect("valid");
    let gain: Float = set.bind("gain").expect("continuous");
    set.audio().host_write(gain.id(), 1.0);
    drop(set);
    finishes("a typed parameter", move || {
        assert_eq!(gain.get(), 12.0, "the store outlived its set");
        drop(gain);
    });
}
