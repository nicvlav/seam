//! Owned lists: steps, stale steps, forgetting and booked size.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use seam_params::{Kind, List, ParamDecl, ParamSet, ParamSetBuilder, RestoreWriter, UiParams};
use seam_undo::{Bound, History, HistoryError};

/// A move-only element that counts its drops and knows its own address.
#[derive(Debug)]
struct Sample {
    id: u32,
    drops: Arc<AtomicUsize>,
    identity: Box<u8>,
}

impl seam_undo::UndoSize for Sample {}

impl Sample {
    fn new(id: u32, drops: &Arc<AtomicUsize>) -> Self {
        Sample {
            id,
            drops: Arc::clone(drops),
            identity: Box::new(0),
        }
    }

    fn address(&self) -> usize {
        &*self.identity as *const u8 as usize
    }
}

impl Drop for Sample {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
    }
}

struct Rig {
    set: Arc<ParamSet>,
    _ui: UiParams,
    writer: RestoreWriter,
    history: History,
    drops: Arc<AtomicUsize>,
}

fn rig() -> Rig {
    rig_with(Bound::default())
}

fn rig_with(bound: Bound) -> Rig {
    let mut builder = ParamSetBuilder::new();
    builder
        .add(ParamDecl::new(
            "p",
            "p",
            Kind::continuous(0.0, 1.0, 0.0, None),
            0.0,
        ))
        .expect("valid");
    let set = builder.build();
    let clock = std::sync::Arc::clone(set.clock());
    let ui = set.take_ui().expect("the first UI handle");
    let history = History::new(&ui, bound).expect("the first history");
    Rig {
        set,
        _ui: ui,
        writer: clock.writer(),
        history,
        drops: Arc::new(AtomicUsize::new(0)),
    }
}

impl Rig {
    fn list(&self, ids: &[u32]) -> List<Sample> {
        let items = ids.iter().map(|&id| Sample::new(id, &self.drops)).collect();
        List::new(items, &self.set)
    }
}

fn ids(list: &List<Sample>) -> Vec<u32> {
    list.with(|items| items.iter().map(|sample| sample.id).collect())
}

#[test]
fn each_verb_is_one_step_and_undo_restores_elements_by_identity() {
    let rig = rig();
    let list = rig.list(&[1, 2, 3]);
    rig.history.enroll_list(&list, "Samples").expect("first");
    let second = list.with(|items| items[1].address());

    assert!(list.remove(1));
    rig.history.tick();
    list.insert(0, Sample::new(4, &rig.drops))
        .expect("in range");
    rig.history.tick();
    assert!(list.move_item(0, 2));
    rig.history.tick();
    assert!(list.clear());
    rig.history.tick();
    assert_eq!(rig.history.len(), 4);
    assert!(
        rig.history
            .entries()
            .iter()
            .all(|entry| entry.name == "Samples")
    );

    rig.history.undo().expect("idle");
    assert_eq!(ids(&list), [1, 3, 4]);
    rig.history.undo().expect("idle");
    assert_eq!(ids(&list), [4, 1, 3]);
    rig.history.undo().expect("idle");
    assert_eq!(ids(&list), [1, 3]);
    rig.history.undo().expect("idle");
    assert_eq!(ids(&list), [1, 2, 3]);
    assert_eq!(
        list.with(|items| items[1].address()),
        second,
        "the same element, not a copy"
    );

    rig.history.jump(4).expect("idle");
    assert!(list.is_empty());
    rig.history.jump(0).expect("idle");
    assert_eq!(ids(&list), [1, 2, 3]);
}

#[test]
fn a_verb_that_changes_nothing_records_nothing() {
    let rig = rig();
    let list = rig.list(&[1, 2]);
    rig.history.enroll_list(&list, "Samples").expect("first");
    assert!(!list.move_item(1, 1));
    assert!(!list.move_item(0, 2));
    assert!(!list.remove(2));
    let back = list
        .insert(3, Sample::new(9, &rig.drops))
        .expect_err("past the end");
    assert_eq!(back.id, 9);
    drop(back);
    let empty = rig.list(&[]);
    rig.history.enroll_list(&empty, "Empty").expect("first");
    assert!(!empty.clear());
    rig.history.tick();
    assert!(rig.history.is_empty());
}

#[test]
fn verbs_join_an_open_transaction() {
    let rig = rig();
    let list = rig.list(&[1]);
    rig.history.enroll_list(&list, "Samples").expect("first");
    {
        let _transaction = rig.history.transaction("Load kit");
        list.push(Sample::new(2, &rig.drops));
        list.push(Sample::new(3, &rig.drops));
        assert!(list.move_item(2, 0));
    }
    assert_eq!(rig.history.len(), 1);
    rig.history.undo().expect("idle");
    assert_eq!(ids(&list), [1]);
    rig.history.redo().expect("idle");
    assert_eq!(ids(&list), [3, 1, 2]);
}

#[test]
fn a_reset_records_nothing_and_makes_earlier_steps_replay_as_nothing() {
    // A wholesale reset (loading) records nothing and makes every
    // earlier step inert; a stale index-based step is a no-op, not
    // undefined behaviour.
    let rig = rig();
    let list = rig.list(&[1, 2, 3]);
    rig.history.enroll_list(&list, "Samples").expect("first");
    assert!(list.remove(2));
    rig.history.tick();
    assert!(list.clear());
    rig.history.tick();

    list.reset(vec![Sample::new(7, &rig.drops)]);
    rig.history.tick();
    assert_eq!(rig.history.len(), 2, "the reset recorded nothing");
    rig.history.jump(0).expect("idle");
    assert_eq!(ids(&list), [7], "both steps were stale");
    rig.history.jump(2).expect("idle");
    assert_eq!(ids(&list), [7]);

    // Where the index still fits, only the reset's generation keeps the
    // step from putting an old element into the new contents.
    let kit = rig.list(&[1, 2]);
    rig.history.enroll_list(&kit, "Kit").expect("first");
    assert!(kit.remove(0));
    rig.history.tick();
    kit.reset(vec![Sample::new(7, &rig.drops), Sample::new(8, &rig.drops)]);
    rig.history.undo().expect("idle");
    assert_eq!(ids(&kit), [7, 8]);

    // The clear's guard on its own: every out-of-band change
    // goes stale through the generation first, so the guard is reached only
    // through the step's handle. It puts elements back only into a list
    // that is still empty.
    let other = rig.list(&[5]);
    let tracker = other.take_tracker().expect("untracked");
    assert!(other.clear());
    let (mut commits, target) = tracker.take();
    let Some(seam_params::Stamped {
        commit: seam_params::ListCommit::Clear { elements },
        ..
    }) = commits.pop()
    else {
        panic!("a clear commit");
    };
    other.push(Sample::new(6, &rig.drops));
    // Taken, so the step is current again and meets the guard itself.
    drop(tracker.take());
    let elements = target.restore_all(elements).expect_err("not empty");
    assert_eq!(ids(&other), [6]);
    assert!(target.take_all().is_some());
    assert!(target.restore_all(elements).is_ok());
    assert_eq!(ids(&other), [5]);
}

#[test]
fn a_verb_made_while_a_restore_applies_records_nothing() {
    // A participant's apply needs no special verbs.
    let rig = rig();
    let list = rig.list(&[1, 2]);
    rig.history.enroll_list(&list, "Samples").expect("first");
    assert!(list.remove(0));
    rig.history.tick();
    {
        let _applying = rig.writer.applying();
        list.push(Sample::new(3, &rig.drops));
    }
    rig.history.tick();
    assert_eq!(rig.history.len(), 1);
    rig.history.undo().expect("idle");
    assert_eq!(
        ids(&list),
        [2, 3],
        "the step from before the apply is stale"
    );
}

#[test]
fn every_element_drops_exactly_once_whatever_drops_first() {
    // Lists and the history drop in any order, and the history never keeps
    // a list's elements alive.
    for history_first in [false, true] {
        let rig = rig();
        let list = rig.list(&[1, 2, 3, 4]);
        rig.history.enroll_list(&list, "Samples").expect("first");
        assert!(list.remove(0));
        rig.history.tick();
        list.push(Sample::new(5, &rig.drops));
        rig.history.tick();
        assert_eq!(rig.drops.load(Ordering::Relaxed), 0);
        let drops = Arc::clone(&rig.drops);
        let Rig { history, .. } = rig;
        if history_first {
            drop(history);
            assert_eq!(drops.load(Ordering::Relaxed), 1, "the removed element");
            drop(list);
        } else {
            drop(list);
            assert_eq!(
                drops.load(Ordering::Relaxed),
                4,
                "the list's elements, at once"
            );
            // Replays against the dead list keep what the steps hold.
            history.tick();
            assert_eq!(history.undo(), Ok(true));
            assert_eq!(history.undo(), Ok(true));
            assert_eq!(drops.load(Ordering::Relaxed), 4);
            drop(history);
        }
        assert_eq!(
            drops.load(Ordering::Relaxed),
            5,
            "history first: {history_first}"
        );
    }
}

#[test]
fn forgetting_one_list_leaves_another_of_its_type_enrolled() {
    let rig = rig();
    let (a, b, never) = (rig.list(&[1]), rig.list(&[1]), rig.list(&[1]));
    rig.history.enroll_list(&a, "a").expect("first");
    rig.history.enroll_list(&b, "b").expect("first");
    assert!(rig.history.forget_list(&b));
    assert!(!rig.history.forget_list(&never), "never enrolled");
    a.push(Sample::new(2, &rig.drops));
    b.push(Sample::new(2, &rig.drops));
    rig.history.tick();
    assert_eq!(rig.history.len(), 1, "a still records, b no more");
    assert!(rig.history.forget_list(&a));
}

#[test]
fn a_forgotten_list_records_nothing_more_and_its_steps_replay_as_nothing() {
    let rig = rig();
    let list = rig.list(&[1, 2]);
    rig.history.enroll_list(&list, "Samples").expect("first");
    assert_eq!(
        rig.history.enroll_list(&list, "Again"),
        Err(HistoryError::AlreadyEnrolled)
    );
    assert!(list.remove(0));
    rig.history.tick();

    assert!(rig.history.forget_list(&list));
    assert!(!rig.history.forget_list(&list));
    list.push(Sample::new(3, &rig.drops));
    rig.history.tick();
    assert_eq!(rig.history.len(), 1, "nothing recorded once forgotten");
    rig.history.undo().expect("idle");
    assert_eq!(ids(&list), [2, 3], "the step replays as nothing");
    assert_eq!(
        rig.drops.load(Ordering::Relaxed),
        0,
        "the step keeps its element"
    );
    rig.history.clear().expect("idle");
    assert_eq!(
        rig.drops.load(Ordering::Relaxed),
        1,
        "and frees it as it drops"
    );

    rig.history.enroll_list(&list, "Samples").expect("again");
    assert!(list.remove(0));
    rig.history.tick();
    rig.history.undo().expect("idle");
    assert_eq!(ids(&list), [2, 3], "enrolled again, it records anew");
}

#[test]
fn a_clear_books_every_element_it_holds() {
    // A step books its stored values by size, once.
    let size = size_of::<Sample>();
    let bound = Bound::default()
        .with_max_units(3 * size + 64)
        .with_min_entries(1);
    let rig = rig_with(bound);
    let list = rig.list(&[1, 2, 3, 4, 5]);
    rig.history.enroll_list(&list, "Samples").expect("first");
    assert!(list.remove(0));
    rig.history.tick();
    assert!(list.clear());
    rig.history.tick();
    assert_eq!(
        rig.history.len(),
        1,
        "the clear's four elements pruned the remove"
    );
}

#[test]
fn every_guard_on_a_steps_handle_makes_it_a_no_op() {
    // Past the end, a verb not yet taken (another thread's, landing
    // between the tick and the replay), and a reset.
    let rig = rig();
    let list = rig.list(&[1, 2]);
    let tracker = list.take_tracker().expect("untracked");
    let (_, target) = tracker.take();
    assert!(target.remove(2).is_none());
    let back = target
        .insert(3, Sample::new(9, &rig.drops))
        .expect_err("past the end");
    assert_eq!(back.id, 9);
    drop(back);
    assert!(!target.move_item(0, 2));
    assert!(!target.move_item(2, 0));
    assert_eq!(ids(&list), [1, 2]);

    // A verb waiting for the history: the step drops it and goes stale for
    // good, with every other step of the list.
    list.push(Sample::new(3, &rig.drops));
    assert!(target.remove(0).is_none());
    assert!(tracker.take().0.is_empty(), "the waiting verb was dropped");
    assert!(!target.move_item(0, 1));
    assert!(target.take_all().is_none());
    assert_eq!(ids(&list), [1, 2, 3]);

    // A reset, for good too.
    let (_, fresh) = tracker.take();
    assert!(fresh.move_item(0, 1), "a handle taken since is current");
    assert_eq!(ids(&list), [2, 1, 3]);
    list.reset(vec![Sample::new(7, &rig.drops)]);
    assert!(fresh.remove(0).is_none());
    assert_eq!(ids(&list), [7]);
}

#[test]
fn an_untracked_list_drops_what_it_removes_at_once() {
    let rig = rig();
    let list = rig.list(&[1, 2, 3]);
    assert!(list.remove(0));
    assert_eq!(rig.drops.load(Ordering::Relaxed), 1);
    assert!(list.clear());
    assert_eq!(rig.drops.load(Ordering::Relaxed), 3);
}
