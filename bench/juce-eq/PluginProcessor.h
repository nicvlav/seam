#pragma once

// The comparison's baseline: the example EQ written straight in JUCE, with no
// Seam. Six bands, then an output gain, over the first two channels; any
// further channels pass through. It matches the Rust EQ (examples/eq) sample
// for sample under the conditions in bench/README.md:
//
//  1. Parameters snap and map 0..1 as Seam's do (EqParameters.h).
//  2. A band retunes at the start of a block when one of its values moved,
//     from JUCE's coefficient makers (EqDsp.h).
//  3. A band that is off still runs bypassed, as in a ProcessorChain, so it
//     comes back on warm.
//  4. The output gain is a plain multiply, with no ramp.
//
// processBlock flushes denormals with ScopedNoDenormals, as JUCE's project
// template does and as Seam's VST3 exporter does around the Rust EQ.
//
// With BENCH_PORTED_DSP, the Rust EQ's own filters ported to C++ (EqPort.h)
// replace JUCE's dsp::IIR, for A-port (bench/README.md, "The ladder").

#include <array>
#include <optional>

#include <juce_audio_processors/juce_audio_processors.h>
#include <juce_dsp/juce_dsp.h>

#include "EqDsp.h"
#if BENCH_PORTED_DSP
#include "EqPort.h"
#endif

namespace bench
{

class JuceEq final : public juce::AudioProcessor
{
public:
    JuceEq();

    void prepareToPlay (double sampleRate, int maximumExpectedSamplesPerBlock) override;
    void releaseResources() override {}
    void reset() override;
    bool isBusesLayoutSupported (const BusesLayout& layouts) const override;
    void processBlock (juce::AudioBuffer<float>& buffer, juce::MidiBuffer& midi) override;

    juce::AudioProcessorEditor* createEditor() override;
    bool hasEditor() const override { return true; }

    const juce::String getName() const override { return JucePlugin_Name; }
    bool acceptsMidi() const override { return false; }
    bool producesMidi() const override { return false; }
    bool isMidiEffect() const override { return false; }
    double getTailLengthSeconds() const override { return 0.0; }

    int getNumPrograms() override { return 1; }
    int getCurrentProgram() override { return 0; }
    void setCurrentProgram (int) override {}
    const juce::String getProgramName (int) override { return {}; }
    void changeProgramName (int, const juce::String&) override {}

    void getStateInformation (juce::MemoryBlock& destination) override;
    void setStateInformation (const void* data, int sizeInBytes) override;

private:
    struct Band
    {
        Shape shape {};
        juce::AudioParameterBool* on = nullptr;
        /** nullptr for the high-pass. */
        juce::AudioParameterFloat* gain = nullptr;
        juce::AudioParameterFloat* frequency = nullptr;
        juce::AudioParameterFloat* q = nullptr;
        // Audio thread only: the values it last tuned to, and whether it has tuned.
        std::array<float, 4> tunedTo {};
        bool isTuned = false;
        bool isBypassed = false;
#if BENCH_PORTED_DSP
        port::Biquad filter;
#else
        std::array<juce::dsp::IIR::Filter<float>, 2> filters;
#endif
    };

    // Audio thread only: retunes each band, and the output, whose values moved.
    void retune() noexcept;

    juce::AudioProcessorValueTreeState state_;
    std::array<Band, 6> bands_;
    juce::AudioParameterBool* outputOn_ = nullptr;
    juce::AudioParameterFloat* outputGainDb_ = nullptr;
    // Audio thread only.
    std::array<float, 2> outputTunedTo_ {};
    bool isOutputTuned_ = false;
    std::optional<float> outputGain_;
    double sampleRate_ = 48000.0;

    JUCE_DECLARE_NON_COPYABLE_WITH_LEAK_DETECTOR (JuceEq)
};

} // namespace bench
