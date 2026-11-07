//! The loom models of the store: its count-then-value read, two writers,
//! and a write's step between store and publish.

#[cfg(loom)]
mod loom_models {
    use std::sync::Arc;

    use crate::{HostEvent, Kind, Origin, ParamDecl, ParamId, ParamSetBuilder, Reading};

    /// The count-then-value protocol: a host write racing a reader. The
    /// writer stores the value, then publishes the count; the reader loads
    /// the count, then the value. Write number `n` stores the value `n`, so
    /// whatever the interleaving, a value below the count it was read with
    /// would be older than that count.
    #[test]
    fn loom_a_read_never_has_a_count_newer_than_its_value() {
        loom::model(|| {
            let mut builder = ParamSetBuilder::new();
            builder
                .add(ParamDecl::new(
                    "p",
                    "p",
                    Kind::continuous(0.0, 2.0, 1.0, None),
                    0.0,
                ))
                .expect("valid");
            let set = builder.build();
            let audio = set.audio();
            let id = ParamId::from_index(0);
            let writer = loom::thread::spawn(move || {
                audio.host_write(id, 0.5);
                audio.host_write(id, 1.0);
            });
            for _ in 0..2 {
                let Reading { changes, value, .. } = set.read(id);
                assert!(value as u64 >= changes, "count {changes}, value {value}");
            }
            writer.join().expect("the writer");
        });
    }

    /// Two writers racing: a user's write on the UI thread and a host write
    /// on the audio thread. Whichever lands last, the store keeps that
    /// writer's value with that writer's origin, so a read names who wrote
    /// the value it returns, during the race as after it. Stored apart, or
    /// read with two loads, the value and the origin could each be a
    /// different writer's.
    #[test]
    fn loom_racing_writers_leave_one_writers_value_with_its_origin() {
        loom::model(|| {
            let mut builder = ParamSetBuilder::new();
            builder
                .add(ParamDecl::new(
                    "p",
                    "p",
                    Kind::continuous(0.0, 2.0, 1.0, None),
                    0.0,
                ))
                .expect("valid");
            let set = builder.build();
            let audio = set.audio();
            let id = ParamId::from_index(0);
            let host = loom::thread::spawn(move || audio.host_write(id, 0.5));
            let ui = set.take_ui().expect("the first UI handle");
            assert!(ui.write(id, 2.0));
            let writer = |value: f32| {
                if value == 2.0 {
                    Origin::User
                } else {
                    Origin::Host
                }
            };
            let Reading { value, origin, .. } = set.read(id);
            assert_eq!(origin, writer(value), "value {value}, read during the race");
            host.join().expect("the host writer");
            let Reading { value, origin, .. } = set.read(id);
            assert_eq!(origin, writer(value), "value {value}");
        });
    }

    /// The writes with a step between their store and their publish: a
    /// user's write, which tells the host there, and a sync, which asks the
    /// host to re-read there. A reader on another thread compares counts, as
    /// a watch does, then drains the host queue and takes the re-read
    /// request, as an exporter does. Write number `n` stores the value `n`,
    /// so a value below the count read before it is older than that count.
    /// A value the host heard that the store does not hold yet, or a request
    /// taken while the store still holds the user's value, would have the
    /// host read back an older value than it was told of.
    #[test]
    fn loom_a_user_write_and_a_sync_publish_their_count_and_request_after_their_value() {
        loom::model(|| {
            let mut builder = ParamSetBuilder::new();
            builder
                .add(ParamDecl::new(
                    "p",
                    "p",
                    Kind::continuous(0.0, 2.0, 1.0, None),
                    0.0,
                ))
                .expect("valid");
            let set = builder.build();
            let id = ParamId::from_index(0);
            let mut drain = set.take_host_event_drain().expect("the first drain");
            let reader = {
                let set = Arc::clone(&set);
                loom::thread::spawn(move || {
                    let Reading { changes, value, .. } = set.read(id);
                    assert!(value as u64 >= changes, "count {changes}, value {value}");
                    let mut heard = None;
                    drain.drain(|event| {
                        if let HostEvent::Value { normalized, .. } = event {
                            heard = Some(normalized);
                        }
                    });
                    if let Some(normalized) = heard {
                        let stored = set.normalized(id);
                        assert!(stored >= normalized, "heard {normalized}, stored {stored}");
                    }
                    if set.take_rescan() {
                        assert_eq!(set.value(id), 2.0, "a request before the sync's value");
                    }
                    drain
                })
            };
            let ui = set.take_ui().expect("the first UI handle");
            assert!(ui.write(id, 1.0));
            assert!(set.program().sync(id, 2.0));
            let mut drain = reader.join().expect("the reader");
            // A bracket the reader's drain left open ends here, once both
            // threads are done, so the drain drops with none open.
            drain.finish(|_| {});
        });
    }
}
