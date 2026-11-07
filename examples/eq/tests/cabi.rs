//! The example EQ as the JUCE adapter sees it, through the C interface: its
//! parameter list, its saved state, and audio that matches the processor
//! driven directly, sample for sample.

use seam_example_eq::{ExampleEq, PARAMS};
use seam_host::cabi::{KIND_BOOL, KIND_CONTINUOUS, TestHost};
use seam_host::{Block, EventList, Layout, Plugin, ProcessSetup, Processor};
use seam_params::{Kind, host_id};

mod common;
use common::{Noise, bits};

const RATE: f64 = 48_000.0;
const MAX_FRAMES: u32 = 512;

#[test]
fn the_adapter_lists_every_parameter_with_its_key() {
    let host = TestHost::<ExampleEq>::new();
    assert_eq!(host.parameter_count(), PARAMS.len());
    assert_eq!(host.layouts(), [Layout::STEREO, Layout::MONO]);
    for index in 0..PARAMS.len() as u32 {
        let info = host.parameter(index).expect("listed");
        let decl = PARAMS
            .iter()
            .find(|decl| decl.key == info.key)
            .expect("a declared key");
        assert_eq!(info.host_id, host_id(decl.key), "{}", decl.key);
        assert_eq!(
            (info.name.as_str(), info.unit.as_str()),
            (decl.name, decl.unit)
        );
        let (kind, steps) = match decl.kind {
            Kind::Bool => (KIND_BOOL, 2),
            _ => (KIND_CONTINUOUS, 0),
        };
        assert_eq!((info.kind, info.steps), (kind, steps), "{}", decl.key);
        assert_eq!(
            info.default_normalized,
            decl.kind.normalize(decl.default_value())
        );
        assert_eq!(info.flags, 0, "no bypass parameter");
    }
}

#[test]
fn audio_through_the_c_interface_matches_the_processor_driven_directly() {
    let host = TestHost::<ExampleEq>::new();
    assert!(host.activate(RATE, MAX_FRAMES, Layout::STEREO));
    let mut plugin = ExampleEq::new();
    let saved = plugin.saved();
    let mut processor = plugin.activate(&ProcessSetup {
        sample_rate: RATE,
        max_frames: MAX_FRAMES as usize,
        layout: Layout::STEREO,
        max_events: 0,
        max_out_events: 0,
    });
    let writes = saved.set().audio();
    let (events, mut out_events) = (EventList::with_capacity(0), EventList::with_capacity(0));
    let keys: Vec<String> = (0..PARAMS.len() as u32)
        .map(|index| host.parameter(index).expect("listed").key)
        .collect();

    let mut noise = Noise(0x1234_5678);
    let mut chooser = Noise(0x9E37_79B9);
    for block in 0..400 {
        let frames = [1, 16, 37, 64, 128, 255, 512][chooser.below(7)];
        for _ in 0..chooser.below(4) {
            let index = chooser.below(keys.len());
            let normalized = chooser.unit();
            host.set(index as u32, normalized);
            let id = saved.set().id(&keys[index]).expect("declared");
            writes.host_write(id, normalized);
        }
        let left: Vec<f32> = (0..frames).map(|_| noise.sample()).collect();
        let right: Vec<f32> = left.iter().map(|sample| -sample).collect();

        let (mut expected_left, mut expected_right) = (left.clone(), right.clone());
        let mut channels: [&mut [f32]; 2] = [&mut expected_left, &mut expected_right];
        processor.process(Block::new(&mut channels, frames, &events, &mut out_events));

        let (mut got_left, mut got_right) = (vec![0.0f32; frames], vec![0.0f32; frames]);
        if block % 2 == 0 {
            host.process(
                &[&left, &right],
                &mut [&mut got_left, &mut got_right],
                frames,
            );
        } else {
            got_left.copy_from_slice(&left);
            got_right.copy_from_slice(&right);
            host.process_in_place(&mut [&mut got_left, &mut got_right], frames);
        }
        assert_eq!(bits(&got_left), bits(&expected_left), "block {block}, left");
        assert_eq!(
            bits(&got_right),
            bits(&expected_right),
            "block {block}, right"
        );
    }
    plugin.deactivate(processor);
    assert_eq!(host.faults(), 0);
}

#[test]
fn a_saved_block_through_the_c_interface_loads_through_vst3() {
    let adapter = TestHost::<ExampleEq>::new();
    let index = (0..PARAMS.len() as u32)
        .find(|&index| adapter.parameter(index).expect("listed").key == "mid.gain")
        .expect("declared");
    adapter.set(index, 0.75);
    let saved = adapter.save().expect("a block");
    let native = seam_host::vst3::TestHost::<ExampleEq>::new();
    assert!(native.load(&saved));
    assert_eq!(native.normalized(host_id("mid.gain")), adapter.get(index));
}
