#pragma once

// The Rust EQ's filters (examples/eq/src/dsp.rs) written line for line in
// C++, for A-port (bench/README.md, "The ladder"): the same coefficient
// formulas, computed once per band; one section over both channels, one call
// per band per block; the bypass a flag inside the loop; the state snapped at
// the end of each block. The arithmetic is JUCE's, as dsp.rs's is, so A-port
// plays what A-native plays, bit for bit.

#include <algorithm>
#include <array>
#include <cmath>
#include <cstddef>
#include <limits>
#include <numbers>

#include "EqDsp.h"

namespace bench::port
{

/** std::pow from the C runtime, never folded at compile time, as dsp.rs's
    powf keeps it. */
inline float powf (float base, float exponent) noexcept
{
    volatile float opaque = base;
    return std::pow (static_cast<float> (opaque), exponent);
}

/** Decibels::decibelsToGain with its default floor of -100 dB. */
inline float decibelsToGain (float decibels) noexcept
{
    return decibels > -100.0f ? powf (10.0f, decibels * 0.05f) : 0.0f;
}

/** Decibels::gainWithLowerBound (gain, -300). */
inline float gainWithLowerBound (float gain) noexcept
{
    return std::max (gain, powf (10.0f, -300.0f * 0.05f));
}

/** JUCE_SNAP_TO_ZERO. */
inline float snapToZero (float value) noexcept
{
    return ! (value < -1.0e-8f || value > 1.0e-8f) ? 0.0f : value;
}

/** A band's coefficients, b0, b1, b2, a1 and a2, normalized by a0. `gainDb`
    is unused by the high-pass. */
inline std::array<float, 5> coefficients (Shape shape, float gainDb, float frequencyHz, float q, double sampleRate) noexcept
{
    constexpr auto pi = std::numbers::pi_v<float>;
    // f32::clamp.
    auto frequency = frequencyHz;
    if (frequency < 1.0f)
        frequency = 1.0f;
    if (frequency > static_cast<float> (sampleRate / 2.0))
        frequency = static_cast<float> (sampleRate / 2.0);
    const auto rate = static_cast<float> (sampleRate);
    std::array<float, 6> raw {};

    switch (shape)
    {
        case Shape::highPass:
        {
            const auto n = std::tan (pi * frequency / rate);
            const auto nSquared = n * n;
            const auto invQ = 1.0f / q;
            const auto c1 = 1.0f / (1.0f + invQ * n + nSquared);
            raw = { c1, c1 * -2.0f, c1, 1.0f, c1 * 2.0f * (nSquared - 1.0f), c1 * (1.0f - invQ * n + nSquared) };
            break;
        }
        case Shape::peak:
        {
            const auto a = std::sqrt (gainWithLowerBound (decibelsToGain (gainDb)));
            const auto omega = (2.0f * pi * std::max (frequency, 2.0f)) / rate;
            const auto alpha = std::sin (omega) / (q * 2.0f);
            const auto c2 = -2.0f * std::cos (omega);
            const auto alphaTimesA = alpha * a;
            const auto alphaOverA = alpha / a;
            raw = { 1.0f + alphaTimesA, c2, 1.0f - alphaTimesA, 1.0f + alphaOverA, c2, 1.0f - alphaOverA };
            break;
        }
        case Shape::lowShelf:
        case Shape::highShelf:
        {
            const auto a = std::sqrt (gainWithLowerBound (decibelsToGain (gainDb)));
            const auto aMinus1 = a - 1.0f;
            const auto aPlus1 = a + 1.0f;
            const auto omega = (2.0f * pi * std::max (frequency, 2.0f)) / rate;
            const auto cos = std::cos (omega);
            const auto beta = std::sin (omega) * std::sqrt (a) / q;
            const auto aMinus1TimesCos = aMinus1 * cos;

            if (shape == Shape::lowShelf)
                raw = { a * (aPlus1 - aMinus1TimesCos + beta),
                        a * 2.0f * (aMinus1 - aPlus1 * cos),
                        a * (aPlus1 - aMinus1TimesCos - beta),
                        aPlus1 + aMinus1TimesCos + beta,
                        -2.0f * (aMinus1 + aPlus1 * cos),
                        aPlus1 + aMinus1TimesCos - beta };
            else
                raw = { a * (aPlus1 + aMinus1TimesCos + beta),
                        a * -2.0f * (aMinus1 + aPlus1 * cos),
                        a * (aPlus1 + aMinus1TimesCos - beta),
                        aPlus1 - aMinus1TimesCos + beta,
                        2.0f * (aMinus1 - aPlus1 * cos),
                        aPlus1 - aMinus1TimesCos - beta };
            break;
        }
    }

    // Multiplied by 1/a0, or by 0 when a0 is about 0.
    const auto a0 = raw[3];
    const auto inverse = std::abs (a0) > std::numeric_limits<float>::min() ? 1.0f / a0 : 0.0f;
    return { raw[0] * inverse, raw[1] * inverse, raw[2] * inverse, raw[4] * inverse, raw[5] * inverse };
}

/** One second-order section over up to two channels. */
struct Biquad
{
    std::array<float, 5> coefficients {};
    /** Per channel, the two state values of the transposed direct form. */
    std::array<std::array<float, 2>, 2> state {};

    /** New coefficients; the state carries on. */
    void set (const std::array<float, 5>& next) noexcept { coefficients = next; }

    /** Starts from silence. */
    void reset() noexcept { state = {}; }

    /** Filters the first `frames` samples of up to two channels in place.
        `bypassed`, the filter still runs, so its state follows the signal,
        but the samples keep their input. */
    void process (float* const* channels, std::size_t count, std::size_t frames, bool bypassed) noexcept
    {
        const auto [b0, b1, b2, a1, a2] = coefficients;

        for (std::size_t channel = 0; channel < std::min (count, state.size()); ++channel)
        {
            auto* const samples = channels[channel];
            auto [lv1, lv2] = state[channel];

            for (std::size_t frame = 0; frame < frames; ++frame)
            {
                const auto input = samples[frame];
                const auto output = (input * b0) + lv1;
                if (! bypassed)
                    samples[frame] = output;
                lv1 = (input * b1) - (output * a1) + lv2;
                lv2 = (input * b2) - (output * a2);
            }

            state[channel] = { snapToZero (lv1), snapToZero (lv2) };
        }
    }
};

} // namespace bench::port
