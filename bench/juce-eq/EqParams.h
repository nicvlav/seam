#pragma once

// The EQ's 25 parameters, the same as examples/eq/src/params.rs: keys,
// names, ranges, skew centres and defaults, f32 for f32. parity.py checks
// the two tables line by line before anything else is compared.

#include <array>
#include <cmath>
#include <limits>

#include "EqDsp.h"

namespace bench
{

struct ParamSpec
{
    const char* key;
    const char* name;
    const char* unit;
    bool isBool;
    float start, end, interval;
    /** NaN for a range with no skew. */
    float skewCentre;
    float defaultValue;
};

inline constexpr auto noSkew = std::numeric_limits<float>::quiet_NaN();

constexpr ParamSpec onParam (const char* key, const char* name)
{
    return { key, name, "", true, 0.0f, 1.0f, 1.0f, noSkew, 1.0f };
}

constexpr ParamSpec decibels (const char* key, const char* name, float start, float end)
{
    return { key, name, "dB", false, start, end, 0.1f, noSkew, 0.0f };
}

constexpr ParamSpec frequency (const char* key, const char* name, float start, float end, float centre, float defaultValue)
{
    return { key, name, "Hz", false, start, end, 1.0f, centre, defaultValue };
}

constexpr ParamSpec qParam (const char* key, const char* name)
{
    return { key, name, "", false, 0.1f, 30.0f, 0.01f, 1.7f, 1.0f };
}

/** Every parameter, in the Rust EQ's declaration order. */
inline constexpr std::array<ParamSpec, 25> params {
    onParam ("hp.on", "High-pass on"),
    frequency ("hp.freq", "High-pass frequency", 16.0f, 350.0f, 75.0f, 16.0f),
    qParam ("hp.q", "High-pass Q"),
    onParam ("low.on", "Low shelf on"),
    decibels ("low.gain", "Low shelf gain", -24.0f, 24.0f),
    frequency ("low.freq", "Low shelf frequency", 30.0f, 450.0f, 116.0f, 100.0f),
    qParam ("low.q", "Low shelf Q"),
    onParam ("lowmid.on", "Low mid on"),
    decibels ("lowmid.gain", "Low mid gain", -24.0f, 24.0f),
    frequency ("lowmid.freq", "Low mid frequency", 200.0f, 2500.0f, 707.0f, 500.0f),
    qParam ("lowmid.q", "Low mid Q"),
    onParam ("mid.on", "Mid on"),
    decibels ("mid.gain", "Mid gain", -24.0f, 24.0f),
    frequency ("mid.freq", "Mid frequency", 600.0f, 5000.0f, 1732.0f, 1500.0f),
    qParam ("mid.q", "Mid Q"),
    onParam ("highmid.on", "High mid on"),
    decibels ("highmid.gain", "High mid gain", -24.0f, 24.0f),
    frequency ("highmid.freq", "High mid frequency", 1000.0f, 9000.0f, 3000.0f, 3000.0f),
    qParam ("highmid.q", "High mid Q"),
    onParam ("high.on", "High shelf on"),
    decibels ("high.gain", "High shelf gain", -24.0f, 24.0f),
    frequency ("high.freq", "High shelf frequency", 1500.0f, 16000.0f, 4899.0f, 8000.0f),
    qParam ("high.q", "High shelf Q"),
    onParam ("out.on", "Output on"),
    decibels ("out.gain", "Output gain", -60.0f, 12.0f),
};

struct BandSpec
{
    Shape shape;
    const char* on;
    /** nullptr for the high-pass. */
    const char* gain;
    const char* frequency;
    const char* q;
};

/** The bands, in the order they filter. */
inline constexpr std::array<BandSpec, 6> bands {
    BandSpec { Shape::highPass, "hp.on", nullptr, "hp.freq", "hp.q" },
    BandSpec { Shape::lowShelf, "low.on", "low.gain", "low.freq", "low.q" },
    BandSpec { Shape::peak, "lowmid.on", "lowmid.gain", "lowmid.freq", "lowmid.q" },
    BandSpec { Shape::peak, "mid.on", "mid.gain", "mid.freq", "mid.q" },
    BandSpec { Shape::peak, "highmid.on", "highmid.gain", "highmid.freq", "highmid.q" },
    BandSpec { Shape::highShelf, "high.on", "high.gain", "high.freq", "high.q" },
};

inline constexpr const char* outputOn = "out.on";
inline constexpr const char* outputGain = "out.gain";

} // namespace bench
