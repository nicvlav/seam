//! The EQ's parameters: what every format, the saved block and the same EQ
//! written straight in JUCE share.

use seam_params::{Kind, ParamDecl};

use crate::dsp::Shape;

/// One band's parameter keys; a high-pass has no gain.
#[derive(Debug, Clone, Copy)]
pub struct Band {
    /// What the band does.
    pub shape: Shape,
    /// Whether it filters, or passes its input through.
    pub on: &'static str,
    /// Its gain in dB; `None` for the high-pass.
    pub gain: Option<&'static str>,
    /// Its frequency in Hz.
    pub frequency: &'static str,
    /// Its Q.
    pub q: &'static str,
}

/// The bands, in the order they filter.
pub const BANDS: [Band; 6] = [
    Band {
        shape: Shape::HighPass,
        on: "hp.on",
        gain: None,
        frequency: "hp.freq",
        q: "hp.q",
    },
    Band {
        shape: Shape::LowShelf,
        on: "low.on",
        gain: Some("low.gain"),
        frequency: "low.freq",
        q: "low.q",
    },
    Band {
        shape: Shape::Peak,
        on: "lowmid.on",
        gain: Some("lowmid.gain"),
        frequency: "lowmid.freq",
        q: "lowmid.q",
    },
    Band {
        shape: Shape::Peak,
        on: "mid.on",
        gain: Some("mid.gain"),
        frequency: "mid.freq",
        q: "mid.q",
    },
    Band {
        shape: Shape::Peak,
        on: "highmid.on",
        gain: Some("highmid.gain"),
        frequency: "highmid.freq",
        q: "highmid.q",
    },
    Band {
        shape: Shape::HighShelf,
        on: "high.on",
        gain: Some("high.gain"),
        frequency: "high.freq",
        q: "high.q",
    },
];

/// Whether the output gain applies.
pub const OUTPUT_ON: &str = "out.on";
/// The output gain.
pub const OUTPUT_GAIN: &str = "out.gain";

const fn on(key: &'static str, name: &'static str, group: &'static str) -> ParamDecl {
    ParamDecl::new(key, name, Kind::Bool, 1.0).with_group(group)
}

const fn decibels(
    key: &'static str,
    name: &'static str,
    group: &'static str,
    start: f32,
    end: f32,
) -> ParamDecl {
    ParamDecl::new(key, name, Kind::continuous(start, end, 0.1, None), 0.0)
        .with_unit("dB")
        .with_group(group)
}

/// A frequency in hertz, skewed so `centre`, the range's geometric middle,
/// sits halfway.
const fn frequency(
    key: &'static str,
    name: &'static str,
    group: &'static str,
    (start, end, centre): (f32, f32, f32),
    default: f32,
) -> ParamDecl {
    ParamDecl::new(
        key,
        name,
        Kind::continuous(start, end, 1.0, Some(centre)),
        default,
    )
    .with_unit("Hz")
    .with_group(group)
}

const fn q(key: &'static str, name: &'static str, group: &'static str) -> ParamDecl {
    ParamDecl::new(key, name, Kind::continuous(0.1, 30.0, 0.01, Some(1.7)), 1.0).with_group(group)
}

/// Every parameter, in declaration order.
pub const PARAMS: &[ParamDecl] = &[
    on("hp.on", "High-pass on", "High-pass"),
    frequency(
        "hp.freq",
        "High-pass frequency",
        "High-pass",
        (16.0, 350.0, 75.0),
        16.0,
    ),
    q("hp.q", "High-pass Q", "High-pass"),
    on("low.on", "Low shelf on", "Low shelf"),
    decibels("low.gain", "Low shelf gain", "Low shelf", -24.0, 24.0),
    frequency(
        "low.freq",
        "Low shelf frequency",
        "Low shelf",
        (30.0, 450.0, 116.0),
        100.0,
    ),
    q("low.q", "Low shelf Q", "Low shelf"),
    on("lowmid.on", "Low mid on", "Low mid"),
    decibels("lowmid.gain", "Low mid gain", "Low mid", -24.0, 24.0),
    frequency(
        "lowmid.freq",
        "Low mid frequency",
        "Low mid",
        (200.0, 2_500.0, 707.0),
        500.0,
    ),
    q("lowmid.q", "Low mid Q", "Low mid"),
    on("mid.on", "Mid on", "Mid"),
    decibels("mid.gain", "Mid gain", "Mid", -24.0, 24.0),
    frequency(
        "mid.freq",
        "Mid frequency",
        "Mid",
        (600.0, 5_000.0, 1_732.0),
        1_500.0,
    ),
    q("mid.q", "Mid Q", "Mid"),
    on("highmid.on", "High mid on", "High mid"),
    decibels("highmid.gain", "High mid gain", "High mid", -24.0, 24.0),
    frequency(
        "highmid.freq",
        "High mid frequency",
        "High mid",
        (1_000.0, 9_000.0, 3_000.0),
        3_000.0,
    ),
    q("highmid.q", "High mid Q", "High mid"),
    on("high.on", "High shelf on", "High shelf"),
    decibels("high.gain", "High shelf gain", "High shelf", -24.0, 24.0),
    frequency(
        "high.freq",
        "High shelf frequency",
        "High shelf",
        (1_500.0, 16_000.0, 4_899.0),
        8_000.0,
    ),
    q("high.q", "High shelf Q", "High shelf"),
    on(OUTPUT_ON, "Output on", "Output"),
    decibels(OUTPUT_GAIN, "Output gain", "Output", -60.0, 12.0),
];
