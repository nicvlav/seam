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

use seam_host::{
    AuCodes, Block, Category, Feature, Layout, Parts, Plugin, PluginInfo, ProcessSetup, Processor,
    Status,
};
use seam_params::{Bool, Changes, Float};

use crate::dsp::{Biquad, coefficients, decibels_to_gain};
use crate::params::{BandParams, EqParams};

/// The plugin: its parameters, on the main thread. It drops the rest of its
/// parts, so it stays `Send`.
#[derive(Debug)]
pub struct ExampleEq {
    params: EqParams,
}

// The C interface binds a plugin that is `Send` to no thread. One that
// stopped being `Send` would be bound to its creating thread instead, with no
// error, so this fails the build first.
const _: () = {
    const fn send<T: Send>() {}
    send::<ExampleEq>();
};

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

    type Params = EqParams;

    type Processor = EqProcessor;

    fn new(params: EqParams, _parts: Parts) -> Self {
        ExampleEq { params }
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
