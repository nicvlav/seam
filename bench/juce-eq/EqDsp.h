#pragma once

// The EQ's coefficients, through JUCE's own makers: float frequency, Q and
// gain factor, a double sample rate, each frequency clamped to the Nyquist
// frequency first as the Rust EQ does (JUCE's makers only assert it).

#include <array>

#include <juce_dsp/juce_dsp.h>

namespace bench
{

enum class Shape
{
    highPass,
    lowShelf,
    peak,
    highShelf
};

/** b0, b1, b2, a0, a1, a2, before normalizing by a0. `gainDb` is unused by
    the high-pass. */
inline std::array<float, 6> rawCoefficients (Shape shape, float gainDb, float frequency, float q, double sampleRate) noexcept
{
    using Make = juce::dsp::IIR::ArrayCoefficients<float>;
    const auto clamped = juce::jlimit (1.0f, static_cast<float> (sampleRate / 2.0), frequency);
    const auto gain = juce::Decibels::decibelsToGain (gainDb);

    switch (shape)
    {
        case Shape::highPass:  return Make::makeHighPass (sampleRate, clamped, q);
        case Shape::lowShelf:  return Make::makeLowShelf (sampleRate, clamped, q, gain);
        case Shape::peak:      return Make::makePeakFilter (sampleRate, clamped, q, gain);
        case Shape::highShelf: return Make::makeHighShelf (sampleRate, clamped, q, gain);
    }

    return {};
}

} // namespace bench
