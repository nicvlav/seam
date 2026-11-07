//! The audio-thread path of seam-params does not allocate: the host write,
//! the learned write and the reads around them.

seam_testkit::install_alloc_gate!();

use seam_params::{HostEvent, Kind, ParamDecl, ParamSetBuilder, Range};
use seam_testkit::alloc::assert_no_allocations;

const LABELS: [&str; 4] = ["a", "b", "c", "d"];

#[test]
fn host_and_learned_writes_do_not_allocate() {
    let mut builder = ParamSetBuilder::new();
    let kinds = [
        Kind::continuous(20.0, 20_000.0, 0.0, Some(1_000.0)),
        Kind::continuous(0.0, 10.0, 3.0, None),
        Kind::Bool,
        Kind::Choice(&LABELS),
    ];
    let keys = ["freq", "steps", "on", "mode"];
    for (key, kind) in keys.into_iter().zip(kinds) {
        builder
            .add(ParamDecl::new(key, key, kind, 0.0))
            .expect("valid");
    }
    let set = builder.build();
    let audio = set.audio();
    let ids: Vec<_> = set.ids().collect();
    // 0.95 on the 0..10 range in steps of 3 lies past its last lattice
    // point, 9, so it snaps to the end.
    let inputs = [
        0.0,
        0.25,
        0.5,
        0.95,
        1.0,
        -3.0,
        7.0,
        f64::NAN,
        f64::INFINITY,
    ];

    assert_no_allocations("host and learned writes over every kind", || {
        let mut sum = 0.0_f64;
        for _ in 0..64 {
            for &id in &ids {
                for &normalized in &inputs {
                    audio.host_write(id, normalized);
                    audio.learned_write(id, normalized);
                    sum += f64::from(audio.value(id)) + set.normalized(id);
                    sum += set.changes(id) as f64;
                }
            }
        }
        std::hint::black_box(sum);
    });
}

#[test]
fn draining_host_events_does_not_allocate() {
    // CLAP drains on the audio thread. The UI writes happen outside the
    // gate, since the UI thread may allocate; the drains happen inside.
    let mut builder = ParamSetBuilder::new();
    for key in ["a", "b", "c", "d"] {
        builder
            .add(ParamDecl::new(
                key,
                key,
                Kind::Continuous(
                    Range::new(20.0, 20_000.0, 0.0, Some(1_000.0)).expect("a valid range"),
                ),
                1_000.0,
            ))
            .expect("valid");
    }
    let set = builder.build();
    let ui = set.take_ui().expect("UI");
    let mut host = set.take_host_event_drain().expect("drain");
    let ids: Vec<_> = set.ids().collect();
    let mut count = 0_u32;

    for round in 0..32_u16 {
        let gestures: Vec<_> = ids.iter().map(|&id| ui.begin(id)).collect();
        for gesture in &gestures {
            gesture.write(100.0 + f32::from(round));
        }
        assert_no_allocations("a drain with open gestures", || {
            host.drain(|event| count += u32::from(matches!(event, HostEvent::Value { .. })));
        });
        drop(gestures);
        assert_no_allocations("a drain that closes them, then an idle drain", || {
            host.drain(|_| count += 1);
            host.drain(|_| count += 1);
        });
    }
    std::hint::black_box(count);
}
