//! The plugin through the VST3 test host where a host can reach it (audio,
//! parameters, the saved block), and driven directly where only the plugin's
//! own code can (the label, undo, MIDI learn).

use std::sync::Arc;

use seam_example_gain::Gain;
use seam_host::vst3::TestHost;
use seam_host::{Block, EventList, Layout, Parts, Plugin, ProcessSetup, Processor, SavedState};
use seam_params::host_id;
use seam_ump::{ChannelVoice, Voice};

const GAIN: &str = "gain";
const MODE: &str = "mode";
const BYPASS: &str = "bypass";

/// The plugin, built as an exporter builds it, and the saved state the
/// host's save and load would go through.
fn built() -> (Gain, Arc<SavedState>) {
    let (params, parts) = Parts::for_plugin::<Gain>().expect("valid declarations");
    let saved = Arc::clone(parts.saved());
    (Gain::new(params, parts), saved)
}

fn running() -> TestHost<Gain> {
    let host = TestHost::<Gain>::new();
    assert_eq!(host.setup(48_000.0, 64), Ok(()));
    assert_eq!(host.activate(), Ok(()));
    assert_eq!(host.start(), Ok(()));
    host
}

#[test]
fn the_gain_applies_through_the_exporter() {
    let mut host = running();
    let gain = host_id(GAIN);
    let plus_six = host.normalize(gain, 6.0);
    let (mut left, mut right) = ([0.5_f32; 8], [-0.25_f32; 8]);
    assert_eq!(
        host.process_in_place(&mut [&mut left, &mut right], 8, &[(gain, plus_six)]),
        Ok(())
    );
    let factor = 10.0_f32.powf(6.0 / 20.0);
    assert!((left[7] - 0.5 * factor).abs() < 1e-6, "{}", left[7]);
    assert!((right[0] + 0.25 * factor).abs() < 1e-6, "{}", right[0]);
}

#[test]
fn bypass_passes_the_input_through() {
    let mut host = running();
    let changes = [
        (host_id(GAIN), host.normalize(host_id(GAIN), 12.0)),
        (host_id(MODE), 1.0),
        (host_id(BYPASS), 1.0),
    ];
    let (mut left, mut right) = ([0.5_f32; 8], [-0.25_f32; 8]);
    assert_eq!(
        host.process_in_place(&mut [&mut left, &mut right], 8, &changes),
        Ok(())
    );
    assert_eq!((left, right), ([0.5; 8], [-0.25; 8]));
}

#[test]
fn the_host_sees_three_named_modes_and_one_bypass() {
    let host = TestHost::<Gain>::new();
    let mode = host.parameter(1).expect("listed");
    assert_eq!(mode.id, host_id(MODE));
    assert!(mode.is_list);
    assert_eq!(mode.step_count, 2);
    let names: Vec<_> = [0.0, 0.5, 1.0]
        .iter()
        .map(|&normalized| host.text(mode.id, normalized).expect("a name"))
        .collect();
    assert_eq!(names, ["Clean", "Warm", "Hot"]);
    let bypass = host.parameter(2).expect("listed");
    assert!(bypass.is_bypass && !mode.is_bypass);
}

#[test]
fn the_label_saves_and_loads_back() {
    let (mut plugin, saved) = built();
    plugin.set_label("Drum bus");
    // The label is captured for the save at the next tick.
    plugin.idle();
    let block = saved.save();

    // Through the exporter, as a host reopening the project.
    let host = TestHost::<Gain>::new();
    assert_eq!(host.load(&block), Ok(()));
    assert_eq!(host.save().as_ref(), Ok(&block));

    // Into a new instance: the label applies on its next tick.
    let (mut reopened, saved) = built();
    saved.load(&block).expect("a saved block");
    assert_eq!(reopened.label(), "Untitled");
    reopened.idle();
    assert_eq!(reopened.label(), "Drum bus");
    assert_eq!(
        reopened.history().undo(),
        Ok(false),
        "a load leaves nothing to undo"
    );
}

#[test]
fn undo_and_redo_restore_the_label_and_its_saved_state() {
    let (mut plugin, saved) = built();
    plugin.set_label("Vocals");
    plugin.idle();
    plugin.set_label("Lead vocals");

    assert_eq!(plugin.history().undo(), Ok(true));
    assert_eq!(plugin.label(), "Vocals");
    plugin.idle();
    let saved = String::from_utf8(saved.save()).expect("UTF-8");
    assert!(saved.contains(r#""label":"Vocals""#), "{saved}");

    assert_eq!(plugin.history().undo(), Ok(true));
    assert_eq!(plugin.label(), "Untitled");
    assert_eq!(plugin.history().redo(), Ok(true));
    assert_eq!(plugin.label(), "Vocals");
}

#[test]
fn two_lone_writes_are_two_steps_and_the_labels_sets_between_ticks_are_one() {
    let (plugin, _) = built();
    let gain = plugin.ui().set().id(GAIN).expect("declared");

    // Each lone write is a gesture of its own, so each is a step. Undo
    // records what changed since the last tick before it undoes.
    assert!(plugin.ui().write(gain, -6.0));
    assert!(plugin.ui().write(gain, -12.0));
    assert_eq!(plugin.history().undo(), Ok(true));
    assert_eq!(plugin.ui().value(gain), -6.0);

    // The label's sets between two ticks are one step, the latest winning.
    plugin.set_label("Vocals");
    plugin.set_label("Lead vocals");
    assert_eq!(plugin.history().undo(), Ok(true));
    assert_eq!(plugin.label(), "Untitled");
    assert_eq!(plugin.history().undo(), Ok(true));
    assert_eq!(plugin.ui().value(gain), 0.0);
}

#[test]
fn a_learned_controller_drives_the_gain() {
    let (mut plugin, saved) = built();
    let mut processor = plugin.activate(&ProcessSetup {
        sample_rate: 48_000.0,
        max_frames: 8,
        layout: Layout::STEREO,
        max_events: 4,
        max_out_events: 0,
    });
    let cc = |controller, value| {
        let mut events = EventList::with_capacity(1);
        let voice = Voice::Midi1ControlChange { controller, value };
        events.push(
            ChannelVoice {
                group: 0,
                channel: 0,
                voice,
            }
            .event(0),
        );
        events
    };
    let run = |processor: &mut _, events: &EventList| {
        let (mut left, mut right) = ([0.5_f32; 8], [0.5_f32; 8]);
        let mut out = EventList::with_capacity(0);
        let mut channels = [&mut left[..], &mut right[..]];
        Processor::process(processor, Block::new(&mut channels, 8, events, &mut out));
        left[0]
    };

    plugin.learn_gain().expect("armed");
    run(&mut processor, &cc(7, 64));
    plugin.idle(); // The tick commits what the block staged.
    assert!(plugin.gain_is_learned());

    // Controller 7 at 0 is the bottom of the range, -60 dB, in its own block.
    let level = run(&mut processor, &cc(7, 0));
    assert!((level - 0.0005).abs() < 1e-6, "{level}");

    // The mapping is saved with the plugin.
    let block = saved.save();
    let (mut reopened, saved) = built();
    saved.load(&block).expect("a saved block");
    reopened.idle();
    assert!(reopened.gain_is_learned());
    plugin.deactivate(processor);
}
