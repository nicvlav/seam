//! Restores from a host thread race saves from another and registrations,
//! releases, changes and ticks on the UI thread. No save ever loses a section
//! or shows an older one after a newer, and the last restore wins.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use seam_params::ParamSetBuilder;
use seam_session::{ApplyError, Participant, Registration, Section, Session};
use seam_testkit::sync::Latch;
use serde_json::json;

const RESTORES: u64 = 300;

/// A participant whose state is the number of the restore it last applied.
#[derive(Default)]
struct Counter {
    n: u64,
    /// Every value it applied, so the test can check none went back.
    applied: Vec<u64>,
}

impl Participant for Counter {
    fn capture(&self) -> Section {
        json!(self.n)
    }

    fn apply(&mut self, section: Option<&Section>) -> Result<(), ApplyError> {
        self.n = section.and_then(Section::as_u64).unwrap_or(0);
        self.applied.push(self.n);
        Ok(())
    }
}

/// A small deterministic generator of the UI thread's operations.
///
/// Its seed fixes only the UI thread's sequence: how the operations interleave
/// with the restoring and saving threads is the scheduler's, so a failing
/// round is not replayable. The round number and the failed assertion say
/// what broke.
struct Steps(u64);

impl Steps {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn restores_and_registrations_racing_saves_never_lose_or_regress_a_section() {
    for round in 1..=20_u64 {
        let set = ParamSetBuilder::new().build();
        let session = Session::new(&set);
        let ui = session.take_ui().expect("the first UI handle");
        let stop = Arc::new(AtomicBool::new(false));
        // Opens when the saver has saved a restored state, so the round
        // cannot end before a save has happened, however short it ran.
        let saved_once = Arc::new(Latch::new(1));

        let restoring = session.clone();
        let restorer = std::thread::spawn(move || {
            for n in 1..=RESTORES {
                restoring.restore(Some(&json!({"k": n, "other": n})));
                if n % 8 == 0 {
                    std::thread::yield_now();
                }
            }
        });
        let saving = session.clone();
        let stopped = Arc::clone(&stop);
        let first_save = Arc::clone(&saved_once);
        let saver = std::thread::spawn(move || {
            let (mut k, mut other, mut saves) = (0, 0, 0_u64);
            while !stopped.load(Ordering::Acquire) {
                let Some(extra) = saving.save() else {
                    assert_eq!(other, 0, "a save lost every section");
                    continue;
                };
                saves += 1;
                first_save.count_down();
                let now_other = extra.get("other").and_then(Section::as_u64);
                let now_k = extra.get("k").and_then(Section::as_u64);
                if other > 0 {
                    assert!(now_other.is_some(), "a save lost the unheld section");
                }
                if let Some(now) = now_other {
                    assert!(now >= other, "other went back from {other} to {now}");
                    other = now;
                    let now_k = now_k.expect("a save lost the participant's section");
                    assert!(now_k >= k, "k went back from {k} to {now_k}");
                    // Every restore carries both keys and stores them at
                    // once, and an apply re-captures what it applied.
                    assert_eq!(now_k, now, "a save saw two restores at once");
                    k = now_k;
                }
            }
            saves
        });

        let mut steps = Steps(round.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut current: Option<(Rc<RefCell<Counter>>, Registration)> = None;
        let mut gone = Vec::new();
        while !restorer.is_finished() {
            match steps.next() % 4 {
                0 if current.is_none() => {
                    let counter = Rc::new(RefCell::new(Counter::default()));
                    let registration = ui.register("k", &counter).expect("free");
                    current = Some((counter, registration));
                }
                1 => {
                    if let Some((counter, registration)) = current.take() {
                        drop(registration);
                        gone.push(counter);
                    }
                }
                2 => {
                    if let Some((_, registration)) = &current {
                        registration.changed();
                    }
                }
                _ => ui.tick(),
            }
        }
        restorer.join().expect("the restoring thread");
        ui.tick();
        let (counter, _registration) = current.get_or_insert_with(|| {
            let counter = Rc::new(RefCell::new(Counter::default()));
            let registration = ui.register("k", &counter).expect("free");
            (counter, registration)
        });
        ui.tick();
        assert_eq!(
            counter.borrow().n,
            RESTORES,
            "round {round}: the last restore wins"
        );
        assert_eq!(
            session.save(),
            Some(json!({"k": RESTORES, "other": RESTORES})),
            "round {round}"
        );
        saved_once.wait();
        stop.store(true, Ordering::Release);
        let saves = saver
            .join()
            .unwrap_or_else(|_| panic!("round {round}: the saving thread panicked"));
        assert!(saves > 0, "round {round}: the saver never saved");
        gone.push(Rc::clone(counter));
        for counter in &gone {
            let applied = &counter.borrow().applied;
            assert!(
                applied.windows(2).all(|pair| pair[0] <= pair[1]),
                "round {round}: a participant applied an older section after a newer: {applied:?}"
            );
        }
    }
}
