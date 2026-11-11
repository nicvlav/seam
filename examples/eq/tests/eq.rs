//! The example EQ, wired end to end with no host: its declarations, its
//! filters, retuning between blocks, and its saved state across threads.

use std::sync::Arc;

use seam_example_eq::EqProcessor;
use seam_host::{Block, EventList, Layout, Plugin, ProcessSetup, Processor, SavedState};
use seam_params::{Kind, ParamDecl};

mod common;
use common::{built, declarations};

const RATE: f64 = 48_000.0;
const MAX_FRAMES: usize = 512;

fn setup() -> ProcessSetup {
    ProcessSetup {
        sample_rate: RATE,
        max_frames: MAX_FRAMES,
        layout: Layout::STEREO,
        max_events: 0,
        max_out_events: 0,
    }
}

/// Runs both channels through in blocks of at most [`MAX_FRAMES`].
fn run(processor: &mut EqProcessor, left: &mut [f32], right: &mut [f32]) {
    let events = EventList::with_capacity(0);
    let mut out = EventList::with_capacity(0);
    for (left, right) in left
        .chunks_mut(MAX_FRAMES)
        .zip(right.chunks_mut(MAX_FRAMES))
    {
        let frames = left.len();
        let mut channels: [&mut [f32]; 2] = [left, right];
        processor.process(Block::new(&mut channels, frames, &events, &mut out));
    }
}

/// The last sample of `samples` of DC through the EQ.
fn dc(processor: &mut EqProcessor, samples: usize) -> f32 {
    let (mut left, mut right) = (vec![1.0f32; samples], vec![1.0f32; samples]);
    run(processor, &mut left, &mut right);
    assert_eq!(left, right, "both channels filter alike");
    *left.last().expect("samples")
}

/// Writes `value`, in real units, as the plugin's own code would.
fn set(saved: &SavedState, key: &str, value: f32) {
    let set = saved.set();
    let id = set.id(key).expect("declared");
    set.program().sync(id, value);
}

/// Every band off but those named.
fn only(saved: &SavedState, on: &[&str]) {
    for key in ["hp", "low", "lowmid", "mid", "highmid", "high"] {
        let on = on.contains(&key);
        set(saved, &format!("{key}.on"), if on { 1.0 } else { 0.0 });
    }
}

fn noise(samples: usize) -> Vec<f32> {
    let mut state = 0x2545_F491_u32;
    (0..samples)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state as f32 / u32::MAX as f32) * 2.0 - 1.0
        })
        .collect()
}

#[test]
fn every_declaration_is_accepted_with_its_own_host_id() {
    // The builder refused nothing, and no two keys share a host id.
    let (_, saved) = built();
    let set = saved.set();
    assert_eq!(set.len(), 25);
    let mut ids: Vec<u32> = set.ids().map(|id| set.host_id(id)).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 25);
}

#[test]
fn a_flat_eq_passes_audio_through() {
    let (mut plugin, saved) = built();
    set(&saved, "hp.on", 0.0);
    let mut processor = plugin.activate(&setup());
    let level = dc(&mut processor, 48_000);
    assert!((level - 1.0).abs() < 1e-4, "{level}");
}

#[test]
fn a_low_shelf_sets_the_dc_gain() {
    let (mut plugin, saved) = built();
    only(&saved, &["low"]);
    set(&saved, "low.gain", -12.0);
    set(&saved, "low.freq", 450.0);
    let mut processor = plugin.activate(&setup());
    let level = dc(&mut processor, 48_000);
    assert!((20.0 * level.log10() + 12.0).abs() < 0.01, "{level}");
}

#[test]
fn a_change_between_blocks_retunes_the_next_block() {
    let (mut plugin, saved) = built();
    only(&saved, &[]);
    let mut processor = plugin.activate(&setup());
    assert_eq!(dc(&mut processor, 64), 1.0, "every band off and 0 dB out");
    set(&saved, "out.gain", -6.0);
    let level = dc(&mut processor, 64);
    assert!((20.0 * level.log10() + 6.0).abs() < 1e-3, "{level}");
    set(&saved, "out.on", 0.0);
    assert_eq!(dc(&mut processor, 64), 1.0, "the output gain off");
}

#[test]
fn a_saved_state_loads_into_a_fresh_instance_on_another_thread() {
    // The host saves and loads on threads of its own.
    let (mut first, saved) = built();
    set(&saved, "mid.gain", 7.5);
    set(&saved, "mid.freq", 2_200.0);
    set(&saved, "high.on", 0.0);
    set(&saved, "out.gain", -3.0);
    let bytes = std::thread::scope(|scope| {
        scope
            .spawn(|| saved.save())
            .join()
            .expect("the host's thread")
    });

    let (mut second, loaded) = built();
    std::thread::scope(|scope| {
        scope
            .spawn(|| assert!(loaded.load(&bytes).is_ok()))
            .join()
            .expect("the host's thread");
    });
    for id in saved.set().ids() {
        assert_eq!(saved.set().value(id), loaded.set().value(id));
    }

    let input = noise(4_800);
    let mut outputs = Vec::new();
    for plugin in [&mut first, &mut second] {
        let mut processor = plugin.activate(&setup());
        let (mut left, mut right) = (input.clone(), input.clone());
        run(&mut processor, &mut left, &mut right);
        outputs.push(left);
    }
    assert_eq!(outputs[0], outputs[1], "the same state, the same output");
}

#[test]
fn loads_racing_the_audio_thread_end_on_the_last_one() {
    // A load on a host thread while blocks run never leaves the
    // processor tuned to anything but a state that was loaded.
    let (mut plugin, saved) = built();
    let states: Vec<Vec<u8>> = [-12.0f32, 6.0, 0.5]
        .into_iter()
        .map(|gain| {
            let (_, other) = built();
            set(&other, "lowmid.gain", gain);
            set(&other, "out.gain", gain / 2.0);
            other.save()
        })
        .collect();
    let mut processor = plugin.activate(&setup());
    let loader = Arc::clone(&saved);
    std::thread::scope(|scope| {
        let states = &states;
        let handle = scope.spawn(move || {
            for round in 0..300 {
                assert!(loader.load(&states[round % 2]).is_ok());
            }
            assert!(loader.load(&states[2]).is_ok());
        });
        let input = noise(MAX_FRAMES);
        while !handle.is_finished() {
            let (mut left, mut right) = (input.clone(), input.clone());
            run(&mut processor, &mut left, &mut right);
        }
    });

    let (mut fresh, loaded) = built();
    assert!(loaded.load(&states[2]).is_ok());
    let mut expected = fresh.activate(&setup());
    let mut outputs = Vec::new();
    for processor in [&mut processor, &mut expected] {
        dc(processor, 1);
        processor.reset();
        let (mut left, mut right) = (noise(2_400), noise(2_400));
        run(processor, &mut left, &mut right);
        outputs.push(left);
    }
    assert_eq!(outputs[0], outputs[1]);
}

/// A value of `decl` other than its default.
fn another(decl: &ParamDecl) -> f32 {
    match decl.kind {
        Kind::Continuous(range) => [0.37, 0.71]
            .into_iter()
            .filter_map(|at| {
                decl.kind
                    .snap(range.start() + (range.end() - range.start()) * at)
            })
            .find(|&value| value != decl.default)
            .expect("a second value"),
        _ => 1.0 - decl.default,
    }
}

/// Every band boosted and the output turned down, so every parameter of
/// every band changes the sound.
fn audible(saved: &SavedState) {
    for key in ["low", "lowmid", "mid", "highmid", "high"] {
        set(saved, &format!("{key}.gain"), 6.0);
    }
    set(saved, "out.gain", -3.0);
}

#[test]
fn each_parameter_retunes_on_its_own() {
    // Moving one parameter alone retunes its band at the next block: what
    // the processor then plays equals a processor tuned to the new value
    // from the start, and differs from the old value.
    let input = noise(2_048);
    let from_silence = |processor: &mut EqProcessor| {
        processor.reset();
        let (mut left, mut right) = (input.clone(), input.clone());
        run(processor, &mut left, &mut right);
        left
    };
    for decl in &declarations() {
        let new = another(decl);
        let (mut changed, saved) = built();
        audible(&saved);
        let mut processor = changed.activate(&setup());
        dc(&mut processor, 64);
        set(&saved, decl.key, new);
        dc(&mut processor, 1);
        let after = from_silence(&mut processor);

        let (mut tuned, saved) = built();
        audible(&saved);
        set(&saved, decl.key, new);
        let expected = from_silence(&mut tuned.activate(&setup()));
        let (mut old, saved) = built();
        audible(&saved);
        let before = from_silence(&mut old.activate(&setup()));
        assert_eq!(after, expected, "{} retuned its band", decl.key);
        assert_ne!(after, before, "{} changes the sound", decl.key);
    }
}

#[test]
fn a_band_switched_back_on_resumes_warm() {
    // An off band keeps running and passes its input through, as a JUCE
    // ProcessorChain bypass does, so it comes back with the state it kept.
    let input = noise(MAX_FRAMES * 8);
    let outputs = [true, false].map(|on_from_start| {
        let (mut plugin, saved) = built();
        only(&saved, &["mid"]);
        set(&saved, "mid.gain", 12.0);
        set(&saved, "mid.freq", 200.0);
        if !on_from_start {
            set(&saved, "mid.on", 0.0);
        }
        let mut processor = plugin.activate(&setup());
        let mut out = Vec::new();
        for (block, chunk) in input.chunks(MAX_FRAMES).enumerate() {
            if block == 4 {
                set(&saved, "mid.on", 1.0);
            }
            let (mut left, mut right) = (chunk.to_vec(), chunk.to_vec());
            run(&mut processor, &mut left, &mut right);
            out.extend(left);
        }
        out
    });
    let on = MAX_FRAMES * 4;
    assert_ne!(outputs[0][..on], outputs[1][..on], "off before");
    assert_eq!(outputs[0][on..], outputs[1][on..], "warm after");
}

#[test]
fn a_retune_keeps_the_filter_state() {
    // A band that restarted from silence at every parameter move would put a
    // click on each.
    let input = noise(MAX_FRAMES * 4);
    let played = |restart: bool| {
        let (mut plugin, saved) = built();
        only(&saved, &["mid"]);
        set(&saved, "mid.gain", 6.0);
        let mut processor = plugin.activate(&setup());
        let mut out = Vec::new();
        for (block, chunk) in input.chunks(MAX_FRAMES).enumerate() {
            if block == 2 {
                set(&saved, "mid.gain", -6.0);
                if restart {
                    processor.reset();
                }
            }
            let (mut left, mut right) = (chunk.to_vec(), chunk.to_vec());
            run(&mut processor, &mut left, &mut right);
            out.extend(left);
        }
        out
    };
    let change = MAX_FRAMES * 2;
    assert_ne!(played(false)[change..], played(true)[change..]);
}

#[test]
fn a_short_block_leaves_the_rest_of_the_buffer_alone() {
    let (mut plugin, saved) = built();
    set(&saved, "mid.gain", 12.0);
    set(&saved, "out.gain", -6.0);
    let mut processor = plugin.activate(&setup());
    let (mut left, mut right) = (vec![0.5f32; 64], vec![0.5f32; 64]);
    let events = EventList::with_capacity(0);
    let mut out = EventList::with_capacity(0);
    let mut channels: [&mut [f32]; 2] = [&mut left, &mut right];
    processor.process(Block::new(&mut channels, 32, &events, &mut out));
    for samples in [&left, &right] {
        assert!(samples[32..].iter().all(|&sample| sample == 0.5));
        assert!(samples[..32].iter().all(|&sample| sample != 0.5));
    }
}
