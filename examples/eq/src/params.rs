//! The EQ's parameters: what every format, the saved block and the same EQ
//! written straight in JUCE share.

use seam_params::{Bool, BoolDecl, Declarer, Float, FloatDecl, Params};

use crate::dsp::Shape;

/// The parameters as the EQ reads them, in the order hosts list them.
#[derive(Debug, Clone)]
pub struct EqParams {
    /// The bands, in the order they filter.
    pub bands: [BandParams; 6],
    /// Whether the output gain applies.
    pub output_on: Bool,
    /// The output gain in dB.
    pub output_gain: Float,
}

impl Params for EqParams {
    fn declare(p: &mut Declarer) -> Self {
        EqParams {
            bands: BANDS.map(|band| BandParams::declare(p, band)),
            output_on: p.add(OUTPUT_ON),
            output_gain: p.add(OUTPUT_GAIN),
        }
    }
}

/// One band's shape and parameters; a high-pass has no gain.
#[derive(Debug, Clone)]
pub struct BandParams {
    /// What the band does.
    pub shape: Shape,
    /// Whether it filters, or passes its input through.
    pub on: Bool,
    /// Its gain in dB; `None` for the high-pass.
    pub gain: Option<Float>,
    /// Its frequency in Hz.
    pub frequency: Float,
    /// Its Q.
    pub q: Float,
}

impl BandParams {
    /// Declares `band`'s parameters through `p`, and keeps what it is
    /// handed.
    fn declare(p: &mut Declarer, band: Band) -> Self {
        BandParams {
            shape: band.shape,
            on: p.add(band.on),
            gain: band.gain.map(|gain| p.add(gain)),
            frequency: p.add(band.frequency),
            q: p.add(band.q),
        }
    }
}

/// One band as it is declared; a high-pass has no gain.
#[derive(Debug, Clone, Copy)]
struct Band {
    shape: Shape,
    on: BoolDecl,
    gain: Option<FloatDecl>,
    frequency: FloatDecl,
    q: FloatDecl,
}

/// The bands, in the order they filter.
const BANDS: [Band; 6] = [
    Band {
        shape: Shape::HighPass,
        on: on("hp.on", "High-pass on", "High-pass"),
        gain: None,
        frequency: frequency(
            "hp.freq",
            "High-pass frequency",
            "High-pass",
            (16.0, 350.0, 75.0),
            16.0,
        ),
        q: q("hp.q", "High-pass Q", "High-pass"),
    },
    Band {
        shape: Shape::LowShelf,
        on: on("low.on", "Low shelf on", "Low shelf"),
        gain: Some(decibels(
            "low.gain",
            "Low shelf gain",
            "Low shelf",
            -24.0,
            24.0,
        )),
        frequency: frequency(
            "low.freq",
            "Low shelf frequency",
            "Low shelf",
            (30.0, 450.0, 116.0),
            100.0,
        ),
        q: q("low.q", "Low shelf Q", "Low shelf"),
    },
    Band {
        shape: Shape::Peak,
        on: on("lowmid.on", "Low mid on", "Low mid"),
        gain: Some(decibels(
            "lowmid.gain",
            "Low mid gain",
            "Low mid",
            -24.0,
            24.0,
        )),
        frequency: frequency(
            "lowmid.freq",
            "Low mid frequency",
            "Low mid",
            (200.0, 2_500.0, 707.0),
            500.0,
        ),
        q: q("lowmid.q", "Low mid Q", "Low mid"),
    },
    Band {
        shape: Shape::Peak,
        on: on("mid.on", "Mid on", "Mid"),
        gain: Some(decibels("mid.gain", "Mid gain", "Mid", -24.0, 24.0)),
        frequency: frequency(
            "mid.freq",
            "Mid frequency",
            "Mid",
            (600.0, 5_000.0, 1_732.0),
            1_500.0,
        ),
        q: q("mid.q", "Mid Q", "Mid"),
    },
    Band {
        shape: Shape::Peak,
        on: on("highmid.on", "High mid on", "High mid"),
        gain: Some(decibels(
            "highmid.gain",
            "High mid gain",
            "High mid",
            -24.0,
            24.0,
        )),
        frequency: frequency(
            "highmid.freq",
            "High mid frequency",
            "High mid",
            (1_000.0, 9_000.0, 3_000.0),
            3_000.0,
        ),
        q: q("highmid.q", "High mid Q", "High mid"),
    },
    Band {
        shape: Shape::HighShelf,
        on: on("high.on", "High shelf on", "High shelf"),
        gain: Some(decibels(
            "high.gain",
            "High shelf gain",
            "High shelf",
            -24.0,
            24.0,
        )),
        frequency: frequency(
            "high.freq",
            "High shelf frequency",
            "High shelf",
            (1_500.0, 16_000.0, 4_899.0),
            8_000.0,
        ),
        q: q("high.q", "High shelf Q", "High shelf"),
    },
];

const OUTPUT_ON: BoolDecl = on("out.on", "Output on", "Output");
const OUTPUT_GAIN: FloatDecl = decibels("out.gain", "Output gain", "Output", -60.0, 12.0);

// Each maker below ends in `checked`, so a mistake in a declaration fails the
// build on the constant that holds it.

const fn on(key: &'static str, name: &'static str, group: &'static str) -> BoolDecl {
    BoolDecl::new(key, name, true).with_group(group).checked()
}

const fn decibels(
    key: &'static str,
    name: &'static str,
    group: &'static str,
    start: f32,
    end: f32,
) -> FloatDecl {
    FloatDecl::new(key, name, start..=end, 0.0)
        .with_step(0.1)
        .with_unit("dB")
        .with_group(group)
        .checked()
}

/// A frequency in hertz, skewed so `centre`, the range's geometric middle,
/// sits halfway.
const fn frequency(
    key: &'static str,
    name: &'static str,
    group: &'static str,
    (start, end, centre): (f32, f32, f32),
    default: f32,
) -> FloatDecl {
    FloatDecl::new(key, name, start..=end, default)
        .with_step(1.0)
        .with_skew_centre(centre)
        .with_unit("Hz")
        .with_group(group)
        .checked()
}

const fn q(key: &'static str, name: &'static str, group: &'static str) -> FloatDecl {
    FloatDecl::new(key, name, 0.1..=30.0, 1.0)
        .with_step(0.01)
        .with_skew_centre(1.7)
        .with_group(group)
        .checked()
}
