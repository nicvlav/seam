//! The bound counts what real gestures keep alive. Each test records real
//! edits and compares the entries kept with a count worked out here from the
//! documented numbers: the stored values' bytes plus 32 units per step.

use std::sync::Arc;

use seam_params::{Kind, List, ParamDecl, ParamSet, ParamSetBuilder, UiParams, Value};
use seam_undo::{Bound, History};

/// What every step books on top of its values, as `Bound` documents it.
const OVERHEAD: usize = 32;

const EDITS: usize = 800;

fn setup(bound: Bound) -> (Arc<ParamSet>, UiParams, History) {
    let mut builder = ParamSetBuilder::new();
    builder
        .add(ParamDecl::new(
            "gain",
            "Gain",
            Kind::continuous(0.0, 10.0, 1.0, None),
            5.0,
        ))
        .expect("valid");
    let set = builder.build();
    let ui = set.take_ui().expect("the first UI handle");
    let history = History::new(&ui, bound).expect("the first history");
    (set, ui, history)
}

/// How many entries of `units` each fit under `bound`, which prunes the
/// oldest while the total is over the cap.
fn fit(bound: Bound, units: usize) -> usize {
    (bound.max_units / units).max(bound.min_entries)
}

#[test]
fn eight_hundred_parameter_drags_keep_what_the_default_bound_implies() {
    let bound = Bound::default();
    let (set, ui, history) = setup(bound);
    let gain = set.id("gain").expect("declared");
    history.enroll_param(gain).expect("first");
    for edit in 0..EDITS {
        // Consecutive values differ, so every drag changes the value.
        assert!(ui.write(gain, (edit % 9 + 1) as f32));
        history.tick();
    }
    // A step stores the value before and after the drag.
    let units = 2 * size_of::<f32>() + OVERHEAD;
    assert_eq!(units, 40);
    assert_eq!(history.len(), fit(bound, units));
    assert_eq!(history.len(), 750);
}

#[test]
fn eight_hundred_edits_of_a_small_value_keep_what_the_default_bound_implies() {
    let bound = Bound::default();
    let (set, _ui, history) = setup(bound);
    let value = Value::new(0_u32, &set);
    history.enroll(&value, "count").expect("first");
    for edit in 1..=EDITS {
        value.set(edit as u32);
        history.tick();
    }
    let units = 2 * size_of::<u32>() + OVERHEAD;
    assert_eq!(history.len(), fit(bound, units));
    assert_eq!(history.len(), 750);
}

#[test]
fn eight_hundred_edits_of_a_64_kib_value_stay_under_the_bound() {
    let bound = Bound::default()
        .with_max_units(1_000_000)
        .with_min_entries(1);
    let (set, _ui, history) = setup(bound);
    let value = Value::new(vec![0_u8; 64 * 1024], &set);
    history.enroll(&value, "wave").expect("first");
    for edit in 1..=EDITS {
        value.set(vec![(edit % 256) as u8; 64 * 1024]);
        history.tick();
    }
    // A step stores two vectors: each its handle and its 64 KiB.
    let units = 2 * (size_of::<Vec<u8>>() + 64 * 1024) + OVERHEAD;
    assert_eq!(history.len(), fit(bound, units));
    assert!(
        history.len() <= 7,
        "the bound holds a few megabytes at most"
    );
}

#[test]
fn eight_hundred_removals_of_64_kib_elements_stay_under_the_bound() {
    let bound = Bound::default()
        .with_max_units(1_000_000)
        .with_min_entries(1);
    let (set, _ui, history) = setup(bound);
    let list = List::new(vec![vec![0_u8; 64 * 1024]; EDITS], &set);
    history.enroll_list(&list, "waves").expect("first");
    for _ in 0..EDITS {
        assert!(list.remove(0));
        history.tick();
    }
    // A removal's step owns the element it took out.
    let units = size_of::<Vec<u8>>() + 64 * 1024 + OVERHEAD;
    assert_eq!(history.len(), fit(bound, units));
}

#[test]
fn a_clear_books_every_element_it_took() {
    // The step owns 100 vectors of 1 KiB; the cap is exactly what they book.
    let units = 100 * (size_of::<Vec<u8>>() + 1024) + OVERHEAD;
    for (max_units, kept) in [(units, 1), (units - 1, 0)] {
        let bound = Bound::default()
            .with_max_units(max_units)
            .with_min_entries(0);
        let (set, _ui, history) = setup(bound);
        let list = List::new(vec![vec![0_u8; 1024]; 100], &set);
        history.enroll_list(&list, "waves").expect("first");
        assert!(list.clear());
        history.tick();
        assert_eq!(history.len(), kept, "cap {max_units}");
    }
}
