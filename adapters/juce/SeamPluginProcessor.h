#pragma once

// The generic JUCE adapter: a Seam plugin, through the C interface seam.h
// declares, as a juce::AudioProcessor. It holds no product's code. Every fact
// JUCE needs at build time comes from the plugin's generated settings (see
// SeamJuce.cmake), and everything else from the plugin's table at run time.
//
// The contract with the plugin, in the order it runs:
//
//  1. The constructor creates the instance, lists its parameters to JUCE by
//     their keys, and starts the tick, about 60 times a second, on JUCE's
//     message thread. Which thread may make which call is seam.h's to say
//     (SeamPlugin). A plugin that cannot move between threads keeps the
//     constructing thread. JUCE's VST3 wrapper constructs on the thread the
//     host creates the plugin on. Where that is not the message thread,
//     every tick is refused and reported as a fault.
//  2. prepareToPlay activates it for the host's rate, block size and main
//     bus layout, and reports the latency the activation set.
//  3. processBlock hands each block over in place. The host's automation
//     reaches the plugin before it, through Parameter::setValue, which JUCE's
//     wrappers call on the audio thread; Seam flushes denormals itself. A
//     host's bypass of a plugin that declares no bypass parameter reaches
//     processBlockBypassed, which delays the input by the reported latency,
//     so the host's compensation still lines it up.
//  4. The tick calls idle; edits the plugin made reach the host from inside
//     it, through JUCE's parameter notifications.
//  5. The destructor stops the tick, then destroys the instance. That ends
//     any gesture still open with JUCE; JUCE's VST3 wrapper has stopped
//     listening by then, so the ends keep JUCE's own count balanced rather
//     than reach the host.

#include <atomic>
#include <cstdint>
#include <vector>

#include <juce_audio_processors/juce_audio_processors.h>

#include "seam.h"

namespace seam
{

class PluginProcessor final : public juce::AudioProcessor,
                              private juce::Timer
{
public:
    PluginProcessor();
    ~PluginProcessor() override;

    // Message thread.
    void prepareToPlay (double sampleRate, int maximumExpectedSamplesPerBlock) override;
    void releaseResources() override;
    // Any thread: the next block resets first.
    void reset() override;
    bool isBusesLayoutSupported (const BusesLayout& layouts) const override;
    // Audio thread.
    void processBlock (juce::AudioBuffer<float>& buffer, juce::MidiBuffer& midi) override;
    using juce::AudioProcessor::processBlock;
    void processBlockBypassed (juce::AudioBuffer<float>& buffer, juce::MidiBuffer& midi) override;
    using juce::AudioProcessor::processBlockBypassed;

    juce::AudioProcessorEditor* createEditor() override { return nullptr; }
    bool hasEditor() const override { return false; }

    const juce::String getName() const override { return JucePlugin_Name; }
    bool acceptsMidi() const override { return JucePlugin_WantsMidiInput != 0; }
    bool producesMidi() const override { return JucePlugin_ProducesMidiOutput != 0; }
    bool isMidiEffect() const override { return JucePlugin_IsMidiEffect != 0; }
    double getTailLengthSeconds() const override;

    // One program, none offered to choose. It takes the plugin's name, since
    // a VST3 host lists programs by name.
    int getNumPrograms() override { return 1; }
    int getCurrentProgram() override { return 0; }
    void setCurrentProgram (int) override {}
    const juce::String getProgramName (int) override { return JucePlugin_Name; }
    void changeProgramName (int, const juce::String&) override {}

    // Any thread but audio.
    void getStateInformation (juce::MemoryBlock& destination) override;
    void setStateInformation (const void* data, int sizeInBytes) override;

    // The plugin's own bypass parameter, when it declares one; JUCE's VST3
    // wrapper adds its own otherwise, as it does for any JUCE plugin.
    juce::AudioProcessorParameter* getBypassParameter() const override;

private:
    class Parameter;

    // The main buses, from the plugin's first layout.
    static BusesProperties busesOf (const SeamPlugin& plugin);

    // Message thread: the tick.
    void timerCallback() override;

    // The plugin's callbacks, inside idle or the instance's destruction, on
    // the message thread. None may throw: they are called from Rust.
    static void onBeginEdit (void* context, std::uint32_t index) noexcept;
    static void onPerformEdit (void* context, std::uint32_t index, double normalized) noexcept;
    static void onEndEdit (void* context, std::uint32_t index) noexcept;
    static void onRestart (void* context, std::uint32_t flags) noexcept;

    Parameter* parameterAt (std::uint32_t index) const noexcept;

    // Message thread: tells the host of the plugin's latency, and the
    // bypassed path of the delay it then keeps.
    void reportLatency();

    const SeamPlugin& plugin_;
    // Null when the plugin was built against another version of seam.h, or
    // refused to start: the processor then plays silence.
    SeamInstance* instance_ = nullptr;
    // In the plugin's order; owned by the AudioProcessor.
    std::vector<Parameter*> parameters_;
    Parameter* bypass_ = nullptr;

    // The bypassed path's delay line, one channel per bus channel, as long as
    // the latency prepareToPlay last reported; allocated there, used only on
    // the audio thread. A latency that grows later is delayed by at most
    // that, until the next prepareToPlay.
    juce::AudioBuffer<float> bypassDelay_;
    int bypassPosition_ = 0;
    // Whether the last block was bypassed, so a bypass that starts clears
    // the delay line instead of replaying the end of the last one. Audio
    // thread only.
    bool bypassing_ = false;
    // The latency the host was last told, for the audio thread.
    std::atomic<int> bypassLatency_ { 0 };

    JUCE_DECLARE_NON_COPYABLE_WITH_LEAK_DETECTOR (PluginProcessor)
};

} // namespace seam
