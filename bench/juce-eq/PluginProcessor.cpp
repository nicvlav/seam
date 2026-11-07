#include "PluginProcessor.h"

#include "EqParameters.h"

namespace bench
{

namespace
{
    template <typename Parameter>
    Parameter* parameter (juce::AudioProcessorValueTreeState& state, const char* key)
    {
        auto* found = dynamic_cast<Parameter*> (state.getParameter (key));
        jassert (found != nullptr);
        return found;
    }
} // namespace

JuceEq::JuceEq()
    : AudioProcessor (BusesProperties()
                          .withInput ("Input", juce::AudioChannelSet::stereo(), true)
                          .withOutput ("Output", juce::AudioChannelSet::stereo(), true)),
      state_ (*this, nullptr, "EqState", parameterLayout())
{
    for (size_t index = 0; index < bands_.size(); ++index)
    {
        const auto& spec = bench::bands[index];
        auto& band = bands_[index];
        band.shape = spec.shape;
        band.on = parameter<juce::AudioParameterBool> (state_, spec.on);
        band.gain = spec.gain != nullptr ? parameter<juce::AudioParameterFloat> (state_, spec.gain) : nullptr;
        band.frequency = parameter<juce::AudioParameterFloat> (state_, spec.frequency);
        band.q = parameter<juce::AudioParameterFloat> (state_, spec.q);
    }

    outputOn_ = parameter<juce::AudioParameterBool> (state_, outputOn);
    outputGainDb_ = parameter<juce::AudioParameterFloat> (state_, outputGain);
}

// Message thread: sizes every filter's state for order 2, so no retune on the
// audio thread ever reallocates it.
void JuceEq::prepareToPlay (double sampleRate, int maximumExpectedSamplesPerBlock)
{
    sampleRate_ = sampleRate;
#if BENCH_PORTED_DSP
    juce::ignoreUnused (maximumExpectedSamplesPerBlock);

    for (auto& band : bands_)
        band.isTuned = false;
#else
    const juce::dsp::ProcessSpec spec { sampleRate, static_cast<juce::uint32> (maximumExpectedSamplesPerBlock), 1 };

    for (auto& band : bands_)
    {
        band.isTuned = false;

        for (auto& filter : band.filters)
        {
            *filter.coefficients = rawCoefficients (Shape::peak, 0.0f, 1000.0f, 1.0f, sampleRate);
            filter.prepare (spec);
        }
    }
#endif

    isOutputTuned_ = false;
    retune();
    reset();
}

void JuceEq::reset()
{
#if BENCH_PORTED_DSP
    for (auto& band : bands_)
        band.filter.reset();
#else
    for (auto& band : bands_)
        for (auto& filter : band.filters)
            filter.reset();
#endif
}

bool JuceEq::isBusesLayoutSupported (const BusesLayout& layouts) const
{
    const auto output = layouts.getMainOutputChannelSet();
    return (output == juce::AudioChannelSet::mono() || output == juce::AudioChannelSet::stereo())
           && layouts.getMainInputChannelSet() == output;
}

void JuceEq::retune() noexcept
{
    for (auto& band : bands_)
    {
        const std::array<float, 4> values { band.on->get() ? 1.0f : 0.0f,
                                            band.gain != nullptr ? band.gain->get() : 0.0f,
                                            band.frequency->get(),
                                            band.q->get() };

        if (band.isTuned && values == band.tunedTo)
            continue;

        band.tunedTo = values;
        band.isTuned = true;
        band.isBypassed = ! band.on->get();
#if BENCH_PORTED_DSP
        band.filter.set (port::coefficients (band.shape, values[1], values[2], values[3], sampleRate_));
#else
        const auto coefficients = rawCoefficients (band.shape, values[1], values[2], values[3], sampleRate_);

        for (auto& filter : band.filters)
            *filter.coefficients = coefficients;
#endif
    }

    const std::array<float, 2> output { outputOn_->get() ? 1.0f : 0.0f, outputGainDb_->get() };

    if (! isOutputTuned_ || output != outputTunedTo_)
    {
        outputTunedTo_ = output;
        isOutputTuned_ = true;
#if BENCH_PORTED_DSP
        outputGain_ = outputOn_->get() ? std::optional<float> { port::decibelsToGain (output[1]) } : std::nullopt;
#else
        outputGain_ = outputOn_->get() ? std::optional<float> { juce::Decibels::decibelsToGain (output[1]) }
                                       : std::nullopt;
#endif
    }
}

// Audio thread.
void JuceEq::processBlock (juce::AudioBuffer<float>& buffer, juce::MidiBuffer&)
{
    // As JUCE's project template does, and as Seam's exporters do around
    // every process call.
    juce::ScopedNoDenormals noDenormals;
    retune();
    const auto channels = juce::jmin (2, buffer.getNumChannels());
    const auto frames = buffer.getNumSamples();
#if BENCH_PORTED_DSP
    // As the Rust EQ's process: one call per band over both channels, then
    // the gain as a plain loop.
    auto* const* samples = buffer.getArrayOfWritePointers();

    for (auto& band : bands_)
        band.filter.process (samples, static_cast<size_t> (channels), static_cast<size_t> (frames), band.isBypassed);

    if (outputGain_)
    {
        const auto gain = *outputGain_;

        for (int channel = 0; channel < channels; ++channel)
        {
            auto* const channelSamples = samples[channel];

            for (int frame = 0; frame < frames; ++frame)
                channelSamples[frame] *= gain;
        }
    }
#else
    for (auto& band : bands_)
    {
        for (int channel = 0; channel < channels; ++channel)
        {
            float* samples[] { buffer.getWritePointer (channel) };
            juce::dsp::AudioBlock<float> block (samples, 1, static_cast<size_t> (frames));
            juce::dsp::ProcessContextReplacing<float> context (block);
            context.isBypassed = band.isBypassed;
            band.filters[static_cast<size_t> (channel)].process (context);
        }
    }

    if (outputGain_)
        for (int channel = 0; channel < channels; ++channel)
            juce::FloatVectorOperations::multiply (buffer.getWritePointer (channel), *outputGain_, frames);
#endif
}

juce::AudioProcessorEditor* JuceEq::createEditor()
{
    return new juce::GenericAudioProcessorEditor (*this);
}

void JuceEq::getStateInformation (juce::MemoryBlock& destination)
{
    if (const auto xml = state_.copyState().createXml())
        copyXmlToBinary (*xml, destination);
}

void JuceEq::setStateInformation (const void* data, int sizeInBytes)
{
    if (const auto xml = getXmlFromBinary (data, sizeInBytes))
        if (xml->hasTagName (state_.state.getType()))
            state_.replaceState (juce::ValueTree::fromXml (*xml));
}

} // namespace bench

juce::AudioProcessor* JUCE_CALLTYPE createPluginFilter()
{
    return new bench::JuceEq();
}
