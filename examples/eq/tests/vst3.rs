//! The example EQ as a VST3 host sees it, through Seam's exporter: its
//! parameter list, its saved state, and audio that matches the processor
//! driven directly, sample for sample.

use seam_example_eq::{ExampleEq, PARAMS};
use seam_host::vst3::TestHost;
use seam_host::{Block, EventList, Layout, Plugin, ProcessSetup, Processor};
use seam_params::{Kind, host_id};

mod common;
use common::{Noise, bits};

const RATE: f64 = 48_000.0;
const MAX_FRAMES: usize = 512;

fn running() -> TestHost<ExampleEq> {
    let host = TestHost::<ExampleEq>::new();
    assert_eq!(host.setup(RATE, MAX_FRAMES), Ok(()));
    assert_eq!(host.activate(), Ok(()));
    assert_eq!(host.start(), Ok(()));
    host
}

#[test]
fn the_host_lists_every_parameter_by_its_key_hash() {
    let host = TestHost::<ExampleEq>::new();
    assert_eq!(host.parameter_count(), PARAMS.len());
    for (index, decl) in PARAMS.iter().enumerate() {
        let info = host.parameter(index).expect("listed");
        assert_eq!(info.id, host_id(decl.key), "{}", decl.key);
        assert_eq!(
            (info.title.as_str(), info.units.as_str()),
            (decl.name, decl.unit)
        );
        let steps = if matches!(decl.kind, Kind::Bool) {
            1
        } else {
            0
        };
        assert_eq!(info.step_count, steps, "{}", decl.key);
        assert_eq!(
            info.default_normalized,
            decl.kind.normalize(decl.default_value())
        );
        assert!(info.can_automate && !info.is_bypass, "{}", decl.key);
    }
    assert_eq!(host.layout(), Layout::STEREO);
    assert_eq!(host.arrange(Layout::MONO), Ok(()), "mono too");
    assert!(host.arrange(Layout::new(1, 2)).is_err());
}

#[test]
fn audio_through_the_exporter_matches_the_processor_driven_directly() {
    let mut host = running();
    let mut plugin = ExampleEq::new();
    let saved = plugin.saved();
    let mut processor = plugin.activate(&ProcessSetup {
        sample_rate: RATE,
        max_frames: MAX_FRAMES,
        layout: Layout::STEREO,
        max_events: 0,
        max_out_events: 0,
    });
    let writes = saved.set().audio();
    let (events, mut out_events) = (EventList::with_capacity(0), EventList::with_capacity(0));

    let mut noise = Noise(0x1234_5678);
    let mut chooser = Noise(0x9E37_79B9);
    for block in 0..400 {
        let frames = [1, 16, 37, 64, 128, 255, 512][chooser.below(7)];
        let changes: Vec<(u32, f64)> = (0..chooser.below(4))
            .map(|_| {
                let decl = &PARAMS[chooser.below(PARAMS.len())];
                (host_id(decl.key), chooser.unit())
            })
            .collect();
        // Each change is a queue of its own, and of two queues for one
        // parameter the exporter takes the first, so the direct side does too.
        let mut named = Vec::new();
        for &(id, normalized) in &changes {
            if named.contains(&id) {
                continue;
            }
            named.push(id);
            let id = saved.set().by_host_id(id).expect("declared");
            writes.host_write(id, normalized);
        }
        let left: Vec<f32> = (0..frames).map(|_| noise.sample()).collect();
        let right: Vec<f32> = left.iter().map(|sample| -sample).collect();

        let (mut expected_left, mut expected_right) = (left.clone(), right.clone());
        let mut channels: [&mut [f32]; 2] = [&mut expected_left, &mut expected_right];
        processor.process(Block::new(&mut channels, frames, &events, &mut out_events));

        let (mut got_left, mut got_right) = (vec![0.0f32; frames], vec![0.0f32; frames]);
        if block % 2 == 0 {
            assert_eq!(
                host.process(
                    &[&left, &right],
                    &mut [&mut got_left, &mut got_right],
                    frames,
                    &changes
                ),
                Ok(())
            );
        } else {
            got_left.copy_from_slice(&left);
            got_right.copy_from_slice(&right);
            assert_eq!(
                host.process_in_place(&mut [&mut got_left, &mut got_right], frames, &changes),
                Ok(())
            );
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
fn a_project_saved_through_the_exporter_reopens_the_same() {
    let mut host = running();
    let changes: Vec<(u32, f64)> = PARAMS
        .iter()
        .enumerate()
        .map(|(index, decl)| (host_id(decl.key), (index as f64 * 0.37) % 1.0))
        .collect();
    let mut left = [0.0f32; 4];
    assert_eq!(host.process_in_place(&mut [&mut left], 4, &changes), Ok(()));
    let block = host.save().expect("a saved block");

    let reopened = TestHost::<ExampleEq>::new();
    assert_eq!(reopened.load(&block), Ok(()));
    for decl in PARAMS {
        let id = host_id(decl.key);
        assert_eq!(reopened.normalized(id), host.normalized(id), "{}", decl.key);
    }
    assert_eq!(
        reopened.save().as_deref(),
        Some(block.as_slice()),
        "byte for byte"
    );
}
