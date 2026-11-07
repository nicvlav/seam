//! The `RestoreClock`: restore counts, the applying mark and the sequence.

use std::sync::Arc;

use seam_params::{Commit, Origin, ParamSetBuilder, RestoreClock, RestoreStamp, Value};
use seam_testkit::sync::Latch;

/// A set's clock, which outlives the set.
fn clock() -> Arc<RestoreClock> {
    Arc::clone(ParamSetBuilder::new().build().clock())
}

#[test]
fn each_set_makes_its_own_clock_and_hands_out_only_that_one() {
    let (a, b) = (
        ParamSetBuilder::new().build(),
        ParamSetBuilder::new().build(),
    );
    assert!(Arc::ptr_eq(a.clock(), a.clock()));
    assert!(!Arc::ptr_eq(a.clock(), b.clock()));
    let ui = a.take_ui().expect("the first UI handle");
    assert!(ui.clock().ptr_eq(&a.clock().reader()));
    assert!(!ui.clock().ptr_eq(&b.clock().reader()));
    assert!(a.clock().writer().on(a.clock()));
    assert!(!a.clock().writer().on(b.clock()));
}

#[test]
fn a_stored_restore_and_a_completed_pass_each_move_the_stamp() {
    let clock = clock();
    let (writer, reader) = (clock.writer(), clock.reader());
    let start = reader.stamp();
    assert_eq!(start, RestoreStamp::default());

    writer.stored();
    let stored = reader.stamp();
    assert_ne!(stored, start);
    assert_eq!((stored.stored(), stored.completed()), (1, 0));

    writer.completed();
    let completed = reader.stamp();
    assert_ne!(completed, stored);
    assert_eq!((completed.stored(), completed.completed()), (1, 1));

    // Nothing moved, so the stamp is equal and a history keeps its steps.
    assert_eq!(reader.stamp(), completed);
}

#[test]
fn the_applying_mark_nests_and_clears_with_the_last_guard() {
    let clock = clock();
    let (writer, reader) = (clock.writer(), clock.reader());
    assert!(!reader.applying());
    {
        let _outer = writer.applying();
        assert!(reader.applying());
        {
            // A late participant registering inside another's apply.
            let _inner = writer.applying();
            assert!(reader.applying());
        }
        assert!(reader.applying());
    }
    assert!(!reader.applying());
    // Applying moves neither count.
    assert_eq!(reader.stamp(), RestoreStamp::default());
}

#[test]
fn the_applying_mark_belongs_to_the_thread_that_took_it() {
    let set = ParamSetBuilder::new().build();
    let value = Value::new(0_i32, &set);
    let tracker = value.take_tracker().expect("the first tracker");
    let writer = set.clock().writer();
    let (held, written) = (Latch::new(1), Latch::new(1));

    std::thread::scope(|scope| {
        let _applying = writer.applying();
        scope.spawn(|| {
            held.wait();
            // A worker writing during the UI thread's apply is not part of it.
            assert!(!value.clock().applying());
            value.set(7);
            written.count_down();
        });
        held.count_down();
        written.wait();

        assert_eq!(value.read().origin, Origin::User);
        assert_eq!(value.get(), 7);
        let commits = tracker.take();
        let [commit] = commits.as_slice() else {
            panic!("one commit expected: {commits:?}");
        };
        assert!(matches!(
            commit.commit,
            Commit::Set {
                value: 7,
                recorded: true
            }
        ));

        // The thread that holds the mark is still part of the apply.
        value.set(8);
        assert_eq!(value.read().origin, Origin::Restore);
    });
}

#[test]
fn a_mark_on_one_clock_is_not_a_mark_on_another() {
    let (a, b) = (clock(), clock());
    let writer = a.writer();
    let _applying = writer.applying();
    assert!(a.reader().applying());
    assert!(!b.reader().applying());
}

#[test]
fn a_restore_stored_on_another_thread_reaches_the_reader() {
    let clock = clock();
    let reader = clock.reader();
    let writer = clock.writer();
    std::thread::spawn(move || writer.stored())
        .join()
        .expect("the restoring thread");
    assert_eq!(reader.stamp().stored(), 1);
}
