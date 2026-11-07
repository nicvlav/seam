//! The example EQ: six biquad bands (a high-pass, a low shelf, three peaks
//! and a high shelf) and an output gain, on Seam.
//!
//! It has parameters and saved state only, with no editor, undo or MIDI
//! learn, so it can be measured against the same EQ written straight in
//! JUCE. Its `cdylib` is a VST3 module through Seam's exporter (the `vst3`
//! feature, on by default); with only `cabi` on, it exports the C interface
//! instead, for the JUCE adapter.
//!
//! A band retunes at the start of the first block after one of its
//! parameters moves. A band that is off keeps running and passes its input
//! through, as a bypassed filter in a JUCE `ProcessorChain` does, so it comes
//! back on warm.

pub mod dsp;
pub mod params;

#[cfg(feature = "vst3")]
seam_host::export_vst3!(ExampleEq);
#[cfg(feature = "cabi")]
seam_host::export_cabi!(ExampleEq);

use std::sync::Arc;

use seam_host::{
    Block, Category, Feature, Layout, Plugin, PluginInfo, ProcessSetup, Processor, SavedState,
    Status,
};
use seam_params::{AudioParams, ParamDecl, ParamId, ParamSet};
use seam_session::Session;

use crate::dsp::{Biquad, Shape, coefficients, decibels_to_gain};
pub use crate::params::PARAMS;
use crate::params::{BANDS, OUTPUT_GAIN, OUTPUT_ON};

/// The plugin: its parameters and its saved state, on the main thread.
#[derive(Debug)]
pub struct ExampleEq {
    saved: Arc<SavedState>,
}

impl Plugin for ExampleEq {
    const INFO: PluginInfo = PluginInfo {
        name: "Seam Example EQ",
        vendor: "Seam",
        url: "",
        email: "",
        description: "Six biquad bands and an output gain",
        version: "0.1.0",
        id: "dev.seam.example-eq",
        vst3_class_id: 0x44f0_fc1b_fd79_434d_a987_84b8_01dc_348b_u128.to_be_bytes(),
        au_codes: (*b"SmEq", *b"Seam"),
        category: Category::Effect,
        features: &[Feature::Equalizer],
        layouts: &[Layout::STEREO, Layout::MONO],
        midi_input: false,
        midi_output: false,
    };

    const PARAMS: &'static [ParamDecl] = PARAMS;

    type Processor = EqProcessor;

    fn new() -> Self {
        let set =
            ParamSet::from_decls(PARAMS.iter().copied()).expect("the EQ's declarations are valid");
        let session = Session::new(&set);
        ExampleEq {
            saved: Arc::new(SavedState::new(&set, &session)),
        }
    }

    fn saved(&self) -> Arc<SavedState> {
        Arc::clone(&self.saved)
    }

    fn activate(&mut self, setup: &ProcessSetup) -> EqProcessor {
        EqProcessor::new(self.saved.set(), setup.sample_rate)
    }
}

/// The audio-thread half: six filters and a gain over the first two
/// channels; any further channels pass through.
#[derive(Debug)]
pub struct EqProcessor {
    params: AudioParams,
    sample_rate: f64,
    bands: [BandState; 6],
    output: OutputState,
}

#[derive(Debug)]
struct BandState {
    shape: Shape,
    on: ParamId,
    gain: Option<ParamId>,
    frequency: ParamId,
    q: ParamId,
    /// The sum of its parameters' change counters when it last tuned.
    seen: Option<u64>,
    bypassed: bool,
    filter: Biquad,
}

#[derive(Debug)]
struct OutputState {
    on: ParamId,
    gain_db: ParamId,
    seen: Option<u64>,
    /// `None` while the output gain is off.
    gain: Option<f32>,
}

fn declared(set: &ParamSet, key: &str) -> ParamId {
    set.id(key).expect("declared in PARAMS")
}

impl EqProcessor {
    fn new(set: &Arc<ParamSet>, sample_rate: f64) -> Self {
        let bands = BANDS.map(|band| BandState {
            shape: band.shape,
            on: declared(set, band.on),
            gain: band.gain.map(|key| declared(set, key)),
            frequency: declared(set, band.frequency),
            q: declared(set, band.q),
            seen: None,
            bypassed: false,
            filter: Biquad::default(),
        });
        let mut processor = EqProcessor {
            params: set.audio(),
            sample_rate,
            bands,
            output: OutputState {
                on: declared(set, OUTPUT_ON),
                gain_db: declared(set, OUTPUT_GAIN),
                seen: None,
                gain: None,
            },
        };
        processor.retune();
        processor
    }

    /// Retunes what moved since it last tuned. Reads atomics only.
    #[cfg_attr(
        not(test),
        deny(
            clippy::indexing_slicing,
            clippy::unwrap_used,
            clippy::expect_used,
            clippy::panic,
            clippy::arithmetic_side_effects
        )
    )]
    fn retune(&mut self) {
        let set = self.params.set();
        for band in &mut self.bands {
            let seen = set
                .changes(band.on)
                .wrapping_add(band.gain.map_or(0, |id| set.changes(id)))
                .wrapping_add(set.changes(band.frequency))
                .wrapping_add(set.changes(band.q));
            if band.seen == Some(seen) {
                continue;
            }
            band.seen = Some(seen);
            band.bypassed = set.value(band.on) < 0.5;
            let gain_db = band.gain.map_or(0.0, |id| set.value(id));
            band.filter.coefficients = coefficients(
                band.shape,
                gain_db,
                set.value(band.frequency),
                set.value(band.q),
                self.sample_rate,
            );
        }
        let output = &mut self.output;
        let seen = set
            .changes(output.on)
            .wrapping_add(set.changes(output.gain_db));
        if output.seen != Some(seen) {
            output.seen = Some(seen);
            output.gain =
                (set.value(output.on) >= 0.5).then(|| decibels_to_gain(set.value(output.gain_db)));
        }
    }
}

#[cfg_attr(
    not(test),
    deny(
        clippy::indexing_slicing,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::arithmetic_side_effects
    )
)]
impl Processor for EqProcessor {
    fn process(&mut self, block: Block<'_, '_>) -> Status {
        self.retune();
        let frames = block.frames;
        let count = block.channels.len().min(2);
        let Some((channels, _)) = block.channels.split_at_mut_checked(count) else {
            return Status::Continue;
        };
        for band in &mut self.bands {
            band.filter.process(channels, frames, band.bypassed);
        }
        if let Some(gain) = self.output.gain {
            for samples in channels.iter_mut() {
                for sample in samples.iter_mut().take(frames) {
                    *sample *= gain;
                }
            }
        }
        Status::Continue
    }

    fn reset(&mut self) {
        for band in &mut self.bands {
            band.filter.reset();
        }
    }
}
