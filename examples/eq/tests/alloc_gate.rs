//! The example EQ's audio thread never allocates, from the first block
//! after activation on, retuning included, driven directly, through the
//! VST3 exporter and through the C interface.

seam_testkit::install_alloc_gate!();

use seam_example_eq::ExampleEq;
use seam_host::vst3::TestHost;
use seam_host::{Block, EventList, Layout, Plugin, ProcessSetup, Processor};
use seam_params::host_id;
use seam_testkit::alloc::assert_no_allocations;

#[test]
fn processing_and_retuning_never_allocate() {
    let mut plugin = ExampleEq::new();
    let saved = plugin.saved();
    let set = saved.set();
    let program = set.program();
    let gain = set.id("mid.gain").expect("declared");
    let on = set.id("high.on").expect("declared");
    let setup = ProcessSetup {
        sample_rate: 48_000.0,
        max_frames: 256,
        layout: Layout::STEREO,
        max_events: 0,
        max_out_events: 0,
    };
    let mut processor = plugin.activate(&setup);
    let events = EventList::with_capacity(0);
    let mut out = EventList::with_capacity(0);
    let (mut left, mut right) = (vec![0.25f32; 256], vec![-0.25f32; 256]);
    for round in 0..64u8 {
        program.sync(gain, f32::from(round % 24));
        program.sync(on, f32::from(round % 2));
        assert_no_allocations("a block, retuning when a parameter moved", || {
            let mut channels: [&mut [f32]; 2] = [&mut left, &mut right];
            processor.process(Block::new(&mut channels, 256, &events, &mut out));
        });
    }
}

#[test]
fn the_first_block_through_the_vst3_exporter_allocates_nothing() {
    let mut host = TestHost::<ExampleEq>::new();
    assert!(host.setup(48_000.0, 256));
    assert!(host.activate());
    assert!(host.start());
    let (gain, on) = (host_id("mid.gain"), host_id("high.on"));
    let (mut left, mut right) = (vec![0.25f32; 600], vec![-0.25f32; 600]);
    let (input_left, input_right) = (vec![0.5f32; 600], vec![-0.5f32; 600]);
    for round in 0..64u8 {
        let changes = [
            (gain, f64::from(round % 24) / 24.0),
            (on, f64::from(round % 2)),
        ];
        assert_no_allocations("a block through VST3, in place, retuning", || {
            assert!(host.process_in_place(&mut [&mut left, &mut right], 256, &changes));
        });
        let ramp = [
            (0, f64::from(round % 24) / 24.0),
            (128, 0.5),
            (255, f64::from(23 - round % 24) / 24.0),
        ];
        assert_no_allocations("a block through VST3 with points at offsets", || {
            assert!(host.process_queues(
                &mut [&mut left, &mut right],
                256,
                &[
                    (gain, &ramp),
                    (on, &[(0, f64::from(round % 2)), (100, 1.0)])
                ],
            ));
        });
        assert_no_allocations(
            "a block through VST3, separate and longer than the setup",
            || {
                assert!(host.process(
                    &[&input_left, &input_right],
                    &mut [&mut left, &mut right],
                    600,
                    &changes,
                ));
            },
        );
    }
}

#[test]
fn the_first_block_through_the_c_interface_allocates_nothing() {
    let host = seam_host::cabi::TestHost::<ExampleEq>::new();
    assert!(host.activate(48_000.0, 256, Layout::STEREO));
    let index = |key: &str| {
        (0..host.parameter_count() as u32)
            .find(|&index| host.parameter(index).expect("listed").key == key)
            .expect("declared")
    };
    let (gain, on) = (index("mid.gain"), index("high.on"));
    let (mut left, mut right) = (vec![0.25f32; 600], vec![-0.25f32; 600]);
    let (input_left, input_right) = (vec![0.5f32; 600], vec![-0.5f32; 600]);
    for round in 0..64u8 {
        assert_no_allocations("host writes, then a block in place, retuning", || {
            host.set(gain, f64::from(round % 24) / 24.0);
            host.set(on, f64::from(round % 2));
            host.process_in_place(&mut [&mut left, &mut right], 256);
        });
        assert_no_allocations("a block, separate and longer than the activation", || {
            host.process(
                &[&input_left, &input_right],
                &mut [&mut left, &mut right],
                600,
            );
        });
    }
}
