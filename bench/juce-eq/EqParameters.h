#pragma once

// JUCE parameters made from the spec table, each float one snapping and
// mapping 0..1 exactly as Seam does (SeamRange.h), so a host's 0..1, a state
// restore or the plugin's own write lands on the same f32 as in the Rust EQ.

#include <memory>
#include <optional>

#include <juce_audio_processors/juce_audio_processors.h>

#include "EqParams.h"
#include "SeamRange.h"

namespace bench
{

inline SeamRange seamRangeOf (const ParamSpec& spec) noexcept
{
    return { spec.start,
             spec.end,
             spec.interval,
             std::isnan (spec.skewCentre) ? std::optional<float> {} : std::optional<float> { spec.skewCentre } };
}

/** JUCE clamps the 0..1 before calling the first function and calls the last
    on every value it stores.

    With BENCH_JUCE_NATIVE_RANGES, JUCE's own range instead, as a JUCE EQ
    would declare it: the comparison's timing-only build (README, "A with
    JUCE's ranges"), which stores values a little apart from Seam's. */
inline juce::NormalisableRange<float> juceRangeOf (const ParamSpec& spec)
{
#if BENCH_JUCE_NATIVE_RANGES
    juce::NormalisableRange<float> range { spec.start, spec.end, spec.interval };
    if (! std::isnan (spec.skewCentre))
        range.setSkewForCentre (spec.skewCentre);
    return range;
#else
    const auto seam = seamRangeOf (spec);
    return { spec.start,
             spec.end,
             [seam] (float, float, float proportion) { return seam.denormalize (static_cast<double> (proportion)); },
             [seam] (float, float, float value) { return static_cast<float> (seam.normalize (value)); },
             [seam] (float, float, float value) { return seam.snap (value); } };
#endif
}

inline std::unique_ptr<juce::RangedAudioParameter> makeParameter (const ParamSpec& spec)
{
    const juce::ParameterID id { spec.key, 1 };

    if (spec.isBool)
        return std::make_unique<juce::AudioParameterBool> (id, spec.name, spec.defaultValue >= 0.5f);

    return std::make_unique<juce::AudioParameterFloat> (id,
                                                        spec.name,
                                                        juceRangeOf (spec),
                                                        spec.defaultValue,
                                                        juce::AudioParameterFloatAttributes().withLabel (spec.unit));
}

inline juce::AudioProcessorValueTreeState::ParameterLayout parameterLayout()
{
    juce::AudioProcessorValueTreeState::ParameterLayout layout;

    for (const auto& spec : params)
        layout.add (makeParameter (spec));

    return layout;
}

} // namespace bench
