#include "SeamPluginProcessor.h"

#include <array>
#include <cstring>
#include <limits>
#include <memory>

namespace seam
{

namespace
{

// The plugin's table, checked against the header this adapter was built with.
const SeamPlugin& table() noexcept
{
    return *seam_plugin();
}

bool isCurrent (const SeamPlugin& plugin) noexcept
{
    return plugin.abi_version == SEAM_ABI_VERSION;
}

juce::String textOf (SeamStr text)
{
    return juce::String::fromUTF8 (text.data, static_cast<int> (text.length));
}

// save's writer: the block replaces what the MemoryBlock held. Called from
// Rust, so it may not throw; an allocation failure ends the process instead.
void writeBlock (void* context, const std::uint8_t* data, std::size_t length) noexcept
{
    static_cast<juce::MemoryBlock*> (context)->replaceAll (data, length);
}

} // namespace

//==============================================================================
// A Seam parameter, as JUCE sees it: named by its key, with every value read
// from and written to the plugin's store, so there is no second copy to keep
// in step.
class PluginProcessor::Parameter final : public juce::HostedAudioProcessorParameter
{
public:
    Parameter (const SeamPlugin& plugin, SeamInstance* instance, std::uint32_t index, const SeamParamInfo& info)
        : juce::HostedAudioProcessorParameter (1),
          plugin_ (plugin),
          instance_ (instance),
          index_ (index),
          key_ (textOf (info.key)),
          name_ (textOf (info.name)),
          unit_ (textOf (info.unit)),
          kind_ (info.kind),
          steps_ (static_cast<int> (info.steps)),
          default_ (static_cast<float> (info.default_normalized))
    {
        told_.store (getValue());
    }

    // Any thread.
    float getValue() const override { return static_cast<float> (plugin_.get (instance_, index_)); }

    // Any thread, the audio thread included: a host write, which the
    // plugin snaps and stores. The host knows the stored value from then on.
    void setValue (float newValue) override
    {
        plugin_.set (instance_, index_, newValue);
        told_.store (getValue());
    }

    float getDefaultValue() const override { return default_; }
    juce::String getName (int maximumStringLength) const override { return name_.substring (0, maximumStringLength); }
    juce::String getLabel() const override { return unit_; }
    juce::String getParameterID() const override { return key_; }

    // A bool or choice is stepped for the host; a stepped range stays
    // continuous.
    int getNumSteps() const override { return steps_ > 0 ? steps_ : AudioProcessor::getDefaultNumParameterSteps(); }
    bool isDiscrete() const override { return steps_ > 0; }
    bool isBoolean() const override { return kind_ == SEAM_KIND_BOOL; }

    // Message thread, or any but audio.
    juce::String getText (float normalized, int maximumStringLength) const override
    {
        std::array<char, 256> text {};
        plugin_.text (instance_, index_, normalized, text.data(), text.size());
        return juce::String::fromUTF8 (text.data()).substring (0, maximumStringLength);
    }

    // JUCE has no way to say text means nothing: a failed parse answers the
    // current value, so the host's write changes nothing.
    float getValueForText (const juce::String& text) const override
    {
        // Counted to the NUL: getNumBytesAsUTF8() counts re-encoded characters,
        // which can run past the buffer of a string holding invalid UTF-8.
        const auto* utf8 = text.toRawUTF8();
        double normalized = 0.0;
        if (plugin_.parse (instance_, index_, SeamStr { utf8, std::strlen (utf8) }, &normalized))
            return static_cast<float> (normalized);
        return getValue();
    }

    // Message thread: tells JUCE, and through it the host, of a value the
    // plugin already stored.
    void tell (float normalized)
    {
        told_.store (normalized);
        sendValueChangedMessageToListeners (normalized);
    }

    // Message thread, after a sync: tells the host the stored value if it
    // is not the last one the host was told or wrote.
    void tellIfMoved()
    {
        const auto value = getValue();
        if (value != told_.load())
            tell (value);
    }

    // Any thread but audio, after a load: the host re-reads every value
    // itself (JUCE's controller does on setComponentState).
    void forgetTold() { told_.store (getValue()); }

private:
    const SeamPlugin& plugin_;
    SeamInstance* const instance_;
    const std::uint32_t index_;
    const juce::String key_, name_, unit_;
    const std::uint32_t kind_;
    const int steps_;
    const float default_;
    // What the host was last told or wrote, so a sync tells it only the
    // values that moved. Written on the audio thread too, by setValue.
    std::atomic<float> told_ { 0.0f };
};

//==============================================================================
PluginProcessor::BusesProperties PluginProcessor::busesOf (const SeamPlugin& plugin)
{
    SeamLayout layout { 0, 2 };
    if (isCurrent (plugin))
        plugin.layout (0, &layout);

    BusesProperties buses;
    if (layout.inputs > 0)
        buses = buses.withInput ("Input", juce::AudioChannelSet::canonicalChannelSet (static_cast<int> (layout.inputs)), true);
    return buses.withOutput ("Output", juce::AudioChannelSet::canonicalChannelSet (static_cast<int> (layout.outputs)), true);
}

PluginProcessor::PluginProcessor()
    : juce::AudioProcessor (busesOf (table())),
      plugin_ (table())
{
    // A plugin built against another seam.h would be misread: refuse it.
    jassert (isCurrent (plugin_));
    if (! isCurrent (plugin_))
        return;

    const SeamHost host { this, &onBeginEdit, &onPerformEdit, &onEndEdit, &onRestart };
    instance_ = plugin_.create (&host);
    if (instance_ == nullptr)
        return;

    const auto count = plugin_.param_count (instance_);
    parameters_.reserve (count);
    for (std::uint32_t index = 0; index < count; ++index)
    {
        // parameters_ is indexed as the plugin's list is, so a gap would
        // misname every later one: stop at the first that cannot be read.
        SeamParamInfo info {};
        if (! plugin_.param_info (instance_, index, &info))
            break;
        auto parameter = std::make_unique<Parameter> (plugin_, instance_, index, info);
        parameters_.push_back (parameter.get());
        if ((info.flags & SEAM_PARAM_BYPASS) != 0)
            bypass_ = parameter.get();
        addParameter (parameter.release());
    }

    startTimerHz (60);
}

PluginProcessor::~PluginProcessor()
{
    // The tick first: no idle may run once the instance is gone. stopTimer
    // does not wait for a tick running on another thread, so, as for any
    // JUCE plugin, the processor must be destroyed on the message thread.
    stopTimer();
    if (instance_ != nullptr)
        plugin_.destroy (instance_);
}

void PluginProcessor::prepareToPlay (double sampleRate, int maximumExpectedSamplesPerBlock)
{
    if (instance_ == nullptr)
        return;

    const SeamLayout layout { static_cast<std::uint32_t> (getMainBusNumInputChannels()),
                              static_cast<std::uint32_t> (getMainBusNumOutputChannels()) };
    const auto frames = static_cast<std::uint32_t> (juce::jmax (1, maximumExpectedSamplesPerBlock));
    // Refused where seam.h's SeamPlugin says, or for a layout the plugin does
    // not run with. A plugin that cannot move refuses it when JUCE's AAX
    // wrapper prepares again on the render thread for a longer block. After a
    // refusal, the processor already built plays on, and so does the bypassed
    // path. Its delay line is left alone: blocks may be running.
    const auto activated = plugin_.activate (instance_, sampleRate, frames, layout);
    jassert (activated);
    if (! activated)
        return;
    // The activation's latency, which the host reads when this returns. No
    // block runs meanwhile, so the bypassed path's delay line may be resized.
    reportLatency();
    bypassDelay_.setSize (juce::jmax (1, getTotalNumInputChannels(), getTotalNumOutputChannels()),
                          juce::jmax (1, getLatencySamples()));
    bypassDelay_.clear();
    bypassPosition_ = 0;
}

void PluginProcessor::releaseResources()
{
    if (instance_ != nullptr)
        plugin_.deactivate (instance_);
}

void PluginProcessor::reset()
{
    if (instance_ != nullptr)
        plugin_.reset (instance_);
}

bool PluginProcessor::isBusesLayoutSupported (const BusesLayout& layouts) const
{
    if (! isCurrent (plugin_))
        return false;

    const auto inputs = static_cast<std::uint32_t> (layouts.getMainInputChannels());
    const auto outputs = static_cast<std::uint32_t> (layouts.getMainOutputChannels());
    const auto count = plugin_.layout_count();
    for (std::uint32_t index = 0; index < count; ++index)
    {
        SeamLayout layout {};
        if (plugin_.layout (index, &layout) && layout.inputs == inputs && layout.outputs == outputs)
            return true;
    }
    return false;
}

void PluginProcessor::processBlock (juce::AudioBuffer<float>& buffer, juce::MidiBuffer& midi)
{
    // The buffer carries the block's MIDI in and takes its MIDI out. The C
    // interface carries none yet, so what came in must not go back out.
    // Clearing keeps the buffer's storage, so it does not allocate.
    midi.clear();
    bypassing_ = false;
    if (instance_ == nullptr)
    {
        buffer.clear();
        return;
    }

    // In place: JUCE's buffer holds the input in its first channels and
    // takes the output in the same ones.
    const auto channels = buffer.getNumChannels();
    const SeamProcess block { buffer.getArrayOfReadPointers(),
                              buffer.getArrayOfWritePointers(),
                              static_cast<std::uint32_t> (juce::jmin (getMainBusNumInputChannels(), channels)),
                              static_cast<std::uint32_t> (juce::jmin (getMainBusNumOutputChannels(), channels)),
                              static_cast<std::uint32_t> (buffer.getNumSamples()) };
    plugin_.process (instance_, &block);
}

void PluginProcessor::processBlockBypassed (juce::AudioBuffer<float>& buffer, juce::MidiBuffer&)
{
    // As JUCE's own does, outputs with no input start silent.
    const auto inputs = juce::jmin (getMainBusNumInputChannels(), buffer.getNumChannels());
    for (auto channel = inputs; channel < buffer.getNumChannels(); ++channel)
        buffer.clear (channel, 0, buffer.getNumSamples());

    // JUCE's own then passes the input through undelayed, which a host that
    // compensates for the latency would play that much early.
    const auto latency = juce::jmin (bypassLatency_.load (std::memory_order_relaxed),
                                     bypassDelay_.getNumSamples());
    if (latency <= 0)
        return;
    // A bypass that starts begins from silence, not from what the last one
    // left in the line; clearing the line allocates nothing.
    if (! bypassing_)
    {
        bypassDelay_.clear();
        bypassPosition_ = 0;
        bypassing_ = true;
    }
    if (bypassPosition_ >= latency)
        bypassPosition_ = 0;

    auto position = bypassPosition_;
    for (auto channel = 0; channel < juce::jmin (inputs, bypassDelay_.getNumChannels()); ++channel)
    {
        auto* samples = buffer.getWritePointer (channel);
        auto* line = bypassDelay_.getWritePointer (channel);
        position = bypassPosition_;
        for (auto index = 0; index < buffer.getNumSamples(); ++index)
        {
            const auto input = samples[index];
            samples[index] = line[position];
            line[position] = input;
            if (++position == latency)
                position = 0;
        }
    }
    bypassPosition_ = position;
}

double PluginProcessor::getTailLengthSeconds() const
{
    if (instance_ == nullptr)
        return 0.0;

    const auto tail = plugin_.tail (instance_);
    if (tail == SEAM_TAIL_INFINITE)
        return std::numeric_limits<double>::infinity();

    // The rate JUCE's wrapper set at initialisation or setup, as it reads it
    // back to turn these seconds into frames, so a host may ask before the
    // first activation.
    const auto sampleRate = getSampleRate();
    return (tail == SEAM_TAIL_NONE || sampleRate <= 0.0) ? 0.0 : tail / sampleRate;
}

void PluginProcessor::getStateInformation (juce::MemoryBlock& destination)
{
    if (instance_ != nullptr)
        plugin_.save (instance_, &destination, &writeBlock);
}

void PluginProcessor::setStateInformation (const void* data, int sizeInBytes)
{
    // A block that is not the plugin's changes nothing.
    if (instance_ == nullptr || data == nullptr || sizeInBytes <= 0)
        return;

    if (plugin_.load (instance_, static_cast<const std::uint8_t*> (data), static_cast<std::size_t> (sizeInBytes)))
        for (auto* parameter : parameters_)
            parameter->forgetTold();
}

juce::AudioProcessorParameter* PluginProcessor::getBypassParameter() const
{
    return bypass_;
}

void PluginProcessor::timerCallback()
{
    plugin_.idle (instance_);
}

void PluginProcessor::reportLatency()
{
    const auto latency = static_cast<int> (plugin_.latency (instance_));
    setLatencySamples (latency);
    bypassLatency_.store (latency, std::memory_order_relaxed);
}

PluginProcessor::Parameter* PluginProcessor::parameterAt (std::uint32_t index) const noexcept
{
    return index < parameters_.size() ? parameters_[index] : nullptr;
}

void PluginProcessor::onBeginEdit (void* context, std::uint32_t index) noexcept
{
    if (auto* parameter = static_cast<PluginProcessor*> (context)->parameterAt (index))
        parameter->beginChangeGesture();
}

void PluginProcessor::onPerformEdit (void* context, std::uint32_t index, double normalized) noexcept
{
    // The plugin already stored the value, so this notifies rather than
    // writes, which setValueNotifyingHost would. JUCE's VST3 wrapper sets its
    // own copy and, while the transport is stopped, writes it back through
    // setValue, which stores what the store holds unless a host write
    // landed since.
    if (auto* parameter = static_cast<PluginProcessor*> (context)->parameterAt (index))
        parameter->tell (static_cast<float> (normalized));
}

void PluginProcessor::onEndEdit (void* context, std::uint32_t index) noexcept
{
    if (auto* parameter = static_cast<PluginProcessor*> (context)->parameterAt (index))
        parameter->endChangeGesture();
}

void PluginProcessor::onRestart (void* context, std::uint32_t flags) noexcept
{
    auto& self = *static_cast<PluginProcessor*> (context);
    if ((flags & SEAM_RESTART_LATENCY) != 0)
        self.reportLatency();

    // JUCE offers no "re-read every value" for a plugin with one program:
    // each parameter that moved tells the host instead, which JUCE's
    // wrappers send on as an edit outside any gesture. A host in latch mode
    // may record those; a sync is rare, and a stale value in the host
    // would be worse. Open: whether Logic and Pro Tools record them through
    // AU is not checked yet.
    if ((flags & SEAM_RESTART_VALUES) != 0)
        for (auto* parameter : self.parameters_)
            parameter->tellIfMoved();
}

} // namespace seam

//==============================================================================
juce::AudioProcessor* JUCE_CALLTYPE createPluginFilter()
{
    return new seam::PluginProcessor();
}
