//! A block whose automation overflows the exporter's room for points, through
//! the VST3 exporter, under the allocation gate (debug and release). Every
//! parameter carries far more points than the eight per parameter the
//! exporter has room for on average, so the walk finds the room full, drops
//! points, and still lands each queue's last one.

seam_testkit::install_alloc_gate!();

use seam_example_eq::{ExampleEq, PARAMS};
use seam_host::vst3::TestHost;
use seam_params::host_id;
use seam_testkit::alloc::assert_no_allocations;

const FRAMES: usize = 4_096;
const POINTS: usize = 64;

#[test]
fn a_block_with_more_points_than_the_room_allocates_nothing_and_lands_each_last_point() {
    let mut host = TestHost::<ExampleEq>::new();
    assert!(host.setup(48_000.0, FRAMES));
    assert!(host.activate());
    assert!(host.start());
    let mut channels = [vec![0.1f32; FRAMES], vec![-0.1f32; FRAMES]];
    for round in 0..4usize {
        let queues: Vec<Vec<(i32, f64)>> = (0..PARAMS.len())
            .map(|parameter| {
                (0..POINTS)
                    .map(|point| {
                        let offset = i32::try_from(point * (FRAMES / POINTS)).expect("small");
                        let value = ((parameter * 7 + point * 3 + round) % 11) as f64 / 10.0;
                        (offset, value)
                    })
                    .collect()
            })
            .collect();
        let sends: Vec<(u32, &[(i32, f64)])> = PARAMS
            .iter()
            .zip(&queues)
            .map(|(decl, points)| (host_id(decl.key), points.as_slice()))
            .collect();
        let [left, right] = &mut channels;
        assert_no_allocations("a block whose points overflow the room", || {
            assert!(host.process_queues(&mut [left, right], FRAMES, &sends));
        });
        for (decl, points) in PARAMS.iter().zip(&queues) {
            let (_, last) = *points.last().expect("points");
            let id = host_id(decl.key);
            // The last point landed, snapped as a host write snaps it.
            let landed = host.normalized(id);
            let again = host.plain(id, last);
            assert!(
                (host.plain(id, landed) - again).abs() < 1e-3 * again.abs().max(1.0),
                "{}: the last point did not land",
                decl.key
            );
        }
    }
}
