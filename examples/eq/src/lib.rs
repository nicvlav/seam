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
    AuCodes, Block, Category, Feature, Layout, Plugin, PluginInfo, ProcessSetup, Processor,
    SavedState, Status,
};
use seam_params::{BindError, Bool, Changes, Float, ParamDecl, ParamSet};
use seam_session::Session;

use crate::dsp::{Biquad, Shape, coefficients, decibels_to_gain};
pub use crate::params::PARAMS;
use crate::params::{BANDS, Band, OUTPUT_GAIN, OUTPUT_ON};

/// The plugin: its parameters and its saved state, on the main thread.
#[derive(Debug)]
pub struct ExampleEq {
    saved: Arc<SavedState>,
    params: EqParams,
}

/// The parameters as the EQ reads them, each bound to its declaration.
#[derive(Debug, Clone)]
struct EqParams {
    bands: [BandParams; 6],
    output_on: Bool,
    output_gain: Float,
}

/// One band's shape and parameters; a high-pass has no gain.
#[derive(Debug, Clone)]
struct BandParams {
    shape: Shape,
    on: Bool,
    gain: Option<Float>,
    frequency: Float,
    q: Float,
}

impl EqParams {
    fn bind(set: &ParamSet) -> Result<Self, BindError> {
        let [hp, low, lowmid, mid, highmid, high] = BANDS.map(|band| BandParams::bind(set, band));
        Ok(EqParams {
            bands: [hp?, low?, lowmid?, mid?, highmid?, high?],
            output_on: set.bind(OUTPUT_ON)?,
            output_gain: set.bind(OUTPUT_GAIN)?,
        })
    }
}

impl BandParams {
    fn bind(set: &ParamSet, band: Band) -> Result<Self, BindError> {
        Ok(BandParams {
            shape: band.shape,
            on: set.bind(band.on)?,
            gain: band.gain.map(|key| set.bind(key)).transpose()?,
            frequency: set.bind(band.frequency)?,
            q: set.bind(band.q)?,
        })
    }
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
        au_codes: AuCodes {
            subtype: *b"SmEq",
            manufacturer: *b"Seam",
        },
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
        let params = EqParams::bind(&set).expect("each parameter matches its declaration");
        let session = Session::new(&set);
        ExampleEq {
            saved: Arc::new(SavedState::new(&set, &session)),
            params,
        }
    }

    fn saved(&self) -> Arc<SavedState> {
        Arc::clone(&self.saved)
    }

    fn activate(&mut self, setup: &ProcessSetup) -> EqProcessor {
        EqProcessor::new(self.params.clone(), setup.sample_rate)
    }
}

/// The audio-thread half: six filters and a gain over the first two
/// channels; any further channels pass through.
#[derive(Debug)]
pub struct EqProcessor {
    sample_rate: f64,
    bands: [BandState; 6],
    output: OutputState,
}

#[derive(Debug)]
struct BandState {
    params: BandParams,
    /// What the filter's coefficients were last made from.
    changes: Changes,
    bypassed: bool,
    filter: Biquad,
}

#[derive(Debug)]
struct OutputState {
    on: Bool,
    gain_db: Float,
    changes: Changes,
    /// `None` while the output gain is off.
    gain: Option<f32>,
}

impl EqProcessor {
    fn new(params: EqParams, sample_rate: f64) -> Self {
        let bands = params.bands.map(|band| BandState {
            params: band,
            changes: Changes::new(),
            bypassed: false,
            filter: Biquad::default(),
        });
        let mut processor = EqProcessor {
            sample_rate,
            bands,
            output: OutputState {
                on: params.output_on,
                gain_db: params.output_gain,
                changes: Changes::new(),
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
        for band in &mut self.bands {
            let params = &band.params;
            let (on, frequency, q) = (&params.on, &params.frequency, &params.q);
            let moved = match &params.gain {
                Some(gain) => band.changes.moved([on, frequency, q, gain]),
                None => band.changes.moved([on, frequency, q]),
            };
            if !moved {
                continue;
            }
            band.bypassed = !on.get();
            let gain_db = params.gain.as_ref().map_or(0.0, Float::get);
            band.filter.coefficients = coefficients(
                params.shape,
                gain_db,
                frequency.get(),
                q.get(),
                self.sample_rate,
            );
        }
        let output = &mut self.output;
        if output.changes.moved([&output.on, &output.gain_db]) {
            output.gain = output
                .on
                .get()
                .then(|| decibels_to_gain(output.gain_db.get()));
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
