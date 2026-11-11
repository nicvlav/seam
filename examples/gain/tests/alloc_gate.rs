//! The example gain's audio thread never allocates, from the first block
//! after activation on: driven directly, as learn captures a controller and the
//! learned controller then drives the gain, and through the VST3 exporter
//! with every parameter moving.

seam_testkit::install_alloc_gate!();

use std::sync::Arc;

use seam_example_gain::Gain;
use seam_host::vst3::TestHost;
use seam_host::{Block, EventList, Layout, Parts, Plugin, ProcessSetup, Processor};
use seam_params::host_id;
use seam_testkit::alloc::assert_no_allocations;
use seam_ump::{ChannelVoice, Voice};

const GAIN: &str = "gain";
const MODE: &str = "mode";
const BYPASS: &str = "bypass";

/// Makes `events` hold one control change, at the block's first frame.
fn control_change(events: &mut EventList, controller: u8, value: u8) {
    events.clear();
    let voice = Voice::Midi1ControlChange { controller, value };
    events.push(
        ChannelVoice {
            group: 0,
            channel: 0,
            voice,
        }
        .event(0),
    );
}

#[test]
fn learning_and_a_learned_controller_never_allocate() {
    let (params, parts) = Parts::for_plugin::<Gain>().expect("valid declarations");
    let set = Arc::clone(parts.set());
    let mut plugin = Gain::new(params, parts);
    let program = set.program();
    let [gain, mode, bypass] = [GAIN, MODE, BYPASS].map(|key| set.id(key).expect("declared"));
    let mut processor = plugin.activate(&ProcessSetup {
        sample_rate: 48_000.0,
        max_frames: 256,
        layout: Layout::STEREO,
        max_events: 4,
        max_out_events: 0,
    });
    let mut events = EventList::with_capacity(4);
    let mut out = EventList::with_capacity(0);
    let (mut left, mut right) = (vec![0.25f32; 256], vec![-0.25f32; 256]);

    plugin.learn_gain().expect("armed");
    control_change(&mut events, 7, 64);
    assert_no_allocations("the first block, capturing a controller to learn", || {
        let mut channels: [&mut [f32]; 2] = [&mut left, &mut right];
        processor.process(Block::new(&mut channels, 256, &events, &mut out));
    });
    plugin.idle();
    assert!(
        plugin.gain_is_learned(),
        "the tick committed what the block captured"
    );

    for round in 0..64u8 {
        control_change(&mut events, 7, round * 2);
        program.sync(mode, f32::from(round % 3));
        program.sync(bypass, f32::from(u8::from(round % 8 == 7)));
        assert_no_allocations("a block, the learned controller moving the gain", || {
            let mut channels: [&mut [f32]; 2] = [&mut left, &mut right];
            processor.process(Block::new(&mut channels, 256, &events, &mut out));
        });
    }
    // The last block's controller, 126 of 127, set the gain near the top of
    // its range, 12 dB.
    assert!(set.value(gain) > 11.0, "{}", set.value(gain));
    plugin.deactivate(processor);
}

#[test]
fn the_first_block_through_the_vst3_exporter_allocates_nothing() {
    let mut host = TestHost::<Gain>::new();
    assert_eq!(host.setup(48_000.0, 256), Ok(()));
    assert_eq!(host.activate(), Ok(()));
    assert_eq!(host.start(), Ok(()));
    let (gain, mode, bypass) = (host_id(GAIN), host_id(MODE), host_id(BYPASS));
    let (mut left, mut right) = (vec![0.25f32; 600], vec![-0.25f32; 600]);
    let (input_left, input_right) = (vec![0.5f32; 600], vec![-0.5f32; 600]);
    for round in 0..64u8 {
        let changes = [
            (gain, f64::from(round % 24) / 24.0),
            (mode, f64::from(round % 3) / 2.0),
            (bypass, f64::from(u8::from(round % 8 == 7))),
        ];
        assert_no_allocations("a block through VST3, in place, retuning", || {
            assert_eq!(
                host.process_in_place(&mut [&mut left, &mut right], 256, &changes),
                Ok(())
            );
        });
        let ramp = [
            (0, f64::from(round % 24) / 24.0),
            (128, 0.5),
            (255, f64::from(23 - round % 24) / 24.0),
        ];
        assert_no_allocations("a block through VST3 with points at offsets", || {
            assert_eq!(
                host.process_queues(
                    &mut [&mut left, &mut right],
                    256,
                    &[(gain, &ramp), (mode, &[(0, 0.0), (100, 1.0)])],
                ),
                Ok(())
            );
        });
        assert_no_allocations(
            "a block through VST3, separate and longer than the setup",
            || {
                assert_eq!(
                    host.process(
                        &[&input_left, &input_right],
                        &mut [&mut left, &mut right],
                        600,
                        &changes,
                    ),
                    Ok(())
                );
            },
        );
    }
}
