// bench-hosted: the comparison's console host. It loads one .vst3 through
// JUCE's VST3 hosting and reports what a host sees, so a build is checked and
// measured as a host loads it, not driven directly.
//
//   bench-hosted <bundle> info          the plugin's description, layouts and
//                                       parameters, one per line:
//                                       id|name|unit|text|steps|default
//   bench-hosted <bundle> state <id>    sets parameter <id> to 0.75, plays a
//                                       block, saves, loads the block into a
//                                       second instance and reports whether
//                                       it saves the same
//   bench-hosted <bundle> audio         reads parity.py's audio script from
//                                       stdin (rate, set <id> <f32 bits>,
//                                       reset, block <frames>) and writes each
//                                       output frame as two f32 bit patterns
//   bench-hosted <bundle> describe      the plugin's description as XML, for
//                                       measure
//   bench-hosted <bundle> measure <description.xml> <rate> <frames>
//                <warm-up blocks> <blocks> <audio out, or -> [idle | spin]
//                                       loads the plugin the description names,
//                                       without scanning, and plays it in real
//                                       time as a host does: see measure()
//
// Parameters are named by the VST3 id the build gives them, which scripts/
// hosted.py works out per build. The audio is the same xorshift noise as
// bench-parity's, left and right inverted.

#include <algorithm>
#include <atomic>
#include <bit>
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <fstream>
#include <iostream>
#include <map>
#include <numeric>
#include <string>
#include <thread>
#include <vector>

#include <juce_audio_processors/juce_audio_processors.h>

#ifndef NOMINMAX
 #define NOMINMAX
#endif
#include <windows.h>
#include <avrt.h>
#include <intrin.h>
#include <psapi.h>

#ifndef CREATE_WAITABLE_TIMER_HIGH_RESOLUTION
 #define CREATE_WAITABLE_TIMER_HIGH_RESOLUTION 0x00000002
#endif

namespace
{

void printBits (float value)
{
    std::printf ("%08x", std::bit_cast<std::uint32_t> (value));
}

// xorshift32, as a signed 32-bit integer over 2^31.
struct Noise
{
    std::uint32_t state = 0x12345678u;

    float next() noexcept
    {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        return static_cast<float> (static_cast<std::int32_t> (state)) / 2147483648.0f;
    }
};

juce::String idOf (juce::AudioProcessorParameter* parameter)
{
    auto* hosted = dynamic_cast<juce::HostedAudioProcessorParameter*> (parameter);
    return hosted != nullptr ? hosted->getParameterID() : juce::String ("?");
}

int describe (juce::AudioPluginInstance& plugin, const juce::PluginDescription& description)
{
    std::printf ("name=%s vendor=%s version=%s category=%s uid=%s instrument=%d\n",
                 description.name.toRawUTF8(),
                 description.manufacturerName.toRawUTF8(),
                 description.version.toRawUTF8(),
                 description.category.toRawUTF8(),
                 description.createIdentifierString().toRawUTF8(),
                 (int) description.isInstrument);
    std::printf ("inputs=%d outputs=%d latency=%d tail=%f\n",
                 plugin.getTotalNumInputChannels(),
                 plugin.getTotalNumOutputChannels(),
                 plugin.getLatencySamples(),
                 plugin.getTailLengthSeconds());

    auto layout = plugin.getBusesLayout();
    if (! layout.inputBuses.isEmpty() && ! layout.outputBuses.isEmpty())
    {
        layout.inputBuses.getReference (0) = juce::AudioChannelSet::mono();
        layout.outputBuses.getReference (0) = juce::AudioChannelSet::mono();
        std::printf ("mono=%d ", (int) plugin.checkBusesLayoutSupported (layout));
        layout.outputBuses.getReference (0) = juce::AudioChannelSet::stereo();
        std::printf ("monoToStereo=%d ", (int) plugin.checkBusesLayoutSupported (layout));
        layout.inputBuses.getReference (0) = juce::AudioChannelSet::create5point1();
        layout.outputBuses.getReference (0) = juce::AudioChannelSet::create5point1();
        std::printf ("surround=%d\n", (int) plugin.checkBusesLayoutSupported (layout));
    }

    for (auto* parameter : plugin.getParameters())
        std::printf ("%s|%s|%s|%s|%d|%f\n",
                     idOf (parameter).toRawUTF8(),
                     parameter->getName (64).toRawUTF8(),
                     parameter->getLabel().toRawUTF8(),
                     parameter->getText (parameter->getValue(), 32).toRawUTF8(),
                     parameter->getNumSteps(),
                     parameter->getDefaultValue());
    return 0;
}

std::map<std::string, juce::AudioProcessorParameter*> parametersById (juce::AudioPluginInstance& plugin)
{
    std::map<std::string, juce::AudioProcessorParameter*> byId;
    for (auto* parameter : plugin.getParameters())
        byId[idOf (parameter).toStdString()] = parameter;
    return byId;
}

int roundTrip (juce::VST3PluginFormat& format,
               const juce::PluginDescription& description,
               juce::AudioPluginInstance& plugin,
               const std::string& id)
{
    const auto byId = parametersById (plugin);
    const auto found = byId.find (id);
    if (found == byId.end())
    {
        std::fprintf (stderr, "unknown id %s\n", id.c_str());
        return 1;
    }

    plugin.prepareToPlay (48000.0, 512);
    found->second->setValue (0.75f);
    juce::AudioBuffer<float> buffer (2, 64);
    juce::MidiBuffer midi;
    buffer.clear();
    plugin.processBlock (buffer, midi);
    juce::MemoryBlock saved;
    plugin.getStateInformation (saved);

    juce::String error;
    auto other = format.createInstanceFromDescription (description, 48000.0, 512, error);
    if (other == nullptr)
    {
        std::fprintf (stderr, "second instance failed: %s\n", error.toRawUTF8());
        return 1;
    }
    other->setStateInformation (saved.getData(), (int) saved.getSize());
    juce::MemoryBlock again;
    other->getStateInformation (again);
    const auto reloaded = parametersById (*other).at (id)->getValue();
    std::printf ("reloaded the same: %d; parameter after reload: %f\n", (int) (again == saved), reloaded);
    return again == saved && reloaded == 0.75f ? 0 : 1;
}

int play (juce::AudioPluginInstance& plugin)
{
    const auto byId = parametersById (plugin);
    juce::AudioBuffer<float> buffer (2, 4096);
    juce::MidiBuffer midi;
    Noise noise;
    std::string word;
    while (std::cin >> word)
    {
        if (word == "rate")
        {
            double rate = 0.0;
            std::cin >> rate;
            plugin.releaseResources();
            plugin.setRateAndBufferSizeDetails (rate, 4096);
            plugin.prepareToPlay (rate, 4096);
        }
        else if (word == "set")
        {
            std::string id, bits;
            std::cin >> id >> bits;
            const auto found = byId.find (id);
            if (found == byId.end())
            {
                std::fprintf (stderr, "unknown id %s\n", id.c_str());
                return 1;
            }
            found->second->setValue (std::bit_cast<float> ((std::uint32_t) std::stoul (bits, nullptr, 16)));
        }
        else if (word == "reset")
        {
            plugin.reset();
        }
        else if (word == "block")
        {
            int frames = 0;
            std::cin >> frames;
            buffer.setSize (2, frames, false, false, true);
            for (int frame = 0; frame < frames; ++frame)
            {
                const auto sample = noise.next();
                buffer.setSample (0, frame, sample);
                buffer.setSample (1, frame, -sample);
            }
            plugin.processBlock (buffer, midi);
            for (int frame = 0; frame < frames; ++frame)
            {
                printBits (buffer.getSample (0, frame));
                std::printf (" ");
                printBits (buffer.getSample (1, frame));
                std::printf ("\n");
            }
        }
    }
    plugin.releaseResources();
    return 0;
}

//==============================================================================
// measure

using Clock = std::chrono::steady_clock;

std::int64_t nanosecondsBetween (Clock::time_point from, Clock::time_point to)
{
    return std::chrono::duration_cast<std::chrono::nanoseconds> (to - from).count();
}

// A transport that is playing, at 120 bpm in 4/4, so every build gets a full
// process context and JUCE's wrapper (A and C) takes parameter changes from
// process only (README, console host rules 2 and 4). Read on the audio
// thread, inside processBlock, and moved on by it after each block.
struct Playing final : juce::AudioPlayHead
{
    double rate = 48000.0;
    juce::int64 samples = 0;

    juce::Optional<PositionInfo> getPosition() const override
    {
        PositionInfo info;
        const auto seconds = (double) samples / rate;
        const auto quarters = seconds * 2.0;
        info.setIsPlaying (true);
        info.setTimeInSamples (samples);
        info.setTimeInSeconds (seconds);
        info.setBpm (120.0);
        info.setTimeSignature (TimeSignature {});
        info.setPpqPosition (quarters);
        info.setPpqPositionOfLastBarStart (std::floor (quarters / 4.0) * 4.0);
        return info;
    }
};

struct Write
{
    int block = 0;
    juce::AudioProcessorParameter* parameter = nullptr;
    float value = 0.0f;
};

struct Memory
{
    std::size_t workingSet = 0, peakWorkingSet = 0, privateBytes = 0, peakPrivateBytes = 0;
};

Memory memory()
{
    PROCESS_MEMORY_COUNTERS_EX counters {};
    counters.cb = sizeof (counters);
    GetProcessMemoryInfo (GetCurrentProcess(), reinterpret_cast<PROCESS_MEMORY_COUNTERS*> (&counters), sizeof (counters));
    return { counters.WorkingSetSize, counters.PeakWorkingSetSize, counters.PrivateUsage, counters.PeakPagefileUsage };
}

// The processor's time stamp counter, which runs at a fixed rate whatever the
// core's clock does. The fences keep the reads from moving across the timed
// call, so a block's time holds the call and a few cycles of the reads.
std::uint64_t stampBefore() noexcept
{
    _mm_lfence();
    const auto stamp = __rdtsc();
    _mm_lfence();
    return stamp;
}

std::uint64_t stampAfter() noexcept
{
    unsigned int processor = 0;
    const auto stamp = __rdtscp (&processor);
    _mm_lfence();
    return stamp;
}

// The cycles a thread or the whole process has run, as Windows counts them:
// in units of the time stamp counter, updated coarsely, so good for totals
// over a run, not for one block.
std::uint64_t threadCycles (HANDLE thread)
{
    ULONG64 cycles = 0;
    QueryThreadCycleTime (thread, &cycles);
    return cycles;
}

std::uint64_t processCycles()
{
    ULONG64 cycles = 0;
    QueryProcessCycleTime (GetCurrentProcess(), &cycles);
    return cycles;
}

// What the audio thread records. Every buffer is sized before the plugin is
// created, so the host allocates nothing while it plays.
struct Run
{
    double rate = 48000.0;
    int frames = 0, warmUp = 0, blocks = 0;
    HANDLE mainThread = nullptr;
    std::vector<Write> writes;
    std::vector<std::uint64_t> ticks; // time stamp counter ticks per timed block
    std::vector<float> captured;
    std::uint64_t firstTicks = 0, wallTicks = 0;
    std::int64_t wallNanoseconds = 0;
    std::uint64_t wholeProcessCycles = 0, mainThreadCycles = 0, audioThreadCycles = 0;
    bool spin = false; // spin between blocks rather than sleep
};

// Plays every block on its own thread, as an audio device's callback thread:
// MMCSS "Pro Audio" at time-critical priority, each block started when its
// frames would be due at the run's rate, so blocks are paced as in playback
// rather than run back to back. Only processBlock is timed. The warm-up blocks
// play first, untimed, so a build that warms up on its first blocks shows it
// in the first block's time rather than in the medians.
//
// Between blocks the thread either sleeps on a high-resolution timer
// ("idle"), so its core may drop into a sleep state and wake with cold caches
// and a low clock, as in a light session; or spins until the block is due
// ("spin"), so the core stays awake and a block's time is the code's alone.
void playPaced (juce::AudioPluginInstance& plugin, Playing& head, juce::AudioBuffer<float>& buffer, Run& run)
{
    DWORD task = 0;
    const auto mmcss = AvSetMmThreadCharacteristicsW (L"Pro Audio", &task);
    SetThreadPriority (GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
    const auto timer = CreateWaitableTimerExW (nullptr, nullptr, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS);

    juce::MidiBuffer midi;
    Noise noise;
    std::size_t next = 0;
    const auto period = std::chrono::duration<double> ((double) run.frames / run.rate);
    const auto total = run.warmUp + run.blocks;
    const auto start = Clock::now() + std::chrono::milliseconds (5);
    Clock::time_point timedFrom;
    std::uint64_t ticksFrom = 0, processFrom = 0, mainFrom = 0, audioFrom = 0;

    for (int block = 0; block < total; ++block)
    {
        const auto due = start + std::chrono::duration_cast<Clock::duration> (period * block);
        const auto wait = nanosecondsBetween (Clock::now(), due);
        if (run.spin)
        {
            while (Clock::now() < due)
                _mm_pause();
        }
        else if (wait > 0 && timer != nullptr)
        {
            LARGE_INTEGER relative;
            relative.QuadPart = -(wait / 100); // in 100 ns units, negative for relative
            SetWaitableTimer (timer, &relative, 0, nullptr, nullptr, FALSE);
            WaitForSingleObject (timer, INFINITE);
        }

        for (int frame = 0; frame < run.frames; ++frame)
        {
            const auto sample = noise.next();
            buffer.setSample (0, frame, sample);
            buffer.setSample (1, frame, -sample);
        }
        for (; next < run.writes.size() && run.writes[next].block == block; ++next)
            run.writes[next].parameter->setValue (run.writes[next].value);

        if (block == run.warmUp)
        {
            timedFrom = Clock::now();
            ticksFrom = __rdtsc();
            processFrom = processCycles();
            mainFrom = threadCycles (run.mainThread);
            audioFrom = threadCycles (GetCurrentThread());
        }

        const auto before = stampBefore();
        plugin.processBlock (buffer, midi);
        const auto after = stampAfter();
        head.samples += run.frames;

        if (block == 0)
            run.firstTicks = after - before;
        if (block >= run.warmUp)
            run.ticks[(std::size_t) (block - run.warmUp)] = after - before;
        if (! run.captured.empty())
        {
            auto* out = run.captured.data() + (std::size_t) block * (std::size_t) run.frames * 2;
            for (int frame = 0; frame < run.frames; ++frame)
            {
                out[frame * 2] = buffer.getSample (0, frame);
                out[frame * 2 + 1] = buffer.getSample (1, frame);
            }
        }
    }

    run.wallNanoseconds = nanosecondsBetween (timedFrom, Clock::now());
    run.wallTicks = __rdtsc() - ticksFrom;
    run.wholeProcessCycles = processCycles() - processFrom;
    run.mainThreadCycles = threadCycles (run.mainThread) - mainFrom;
    run.audioThreadCycles = threadCycles (GetCurrentThread()) - audioFrom;
    if (timer != nullptr)
        CloseHandle (timer);
    if (mmcss != nullptr)
        AvRevertMmThreadCharacteristics (mmcss);
}

// Runs the message loop on this thread, the message thread, until `done` is
// set. It delivers what the JUCE timer threads post (the hosting's own; in A
// and C the plugin's own copy of JUCE, for C's tick and A's state) and the
// window timer B's exporter ticks on, as a host's message loop does
// while it plays (README, console host rule 7).
void pumpMessagesUntil (HANDLE done)
{
    for (;;)
    {
        if (MsgWaitForMultipleObjects (1, &done, FALSE, INFINITE, QS_ALLINPUT) == WAIT_OBJECT_0)
            return;
        MSG message;
        while (PeekMessageW (&message, nullptr, 0, 0, PM_REMOVE))
        {
            TranslateMessage (&message);
            DispatchMessageW (&message);
        }
    }
}

template <typename T>
T percentile (std::vector<T> values, double fraction)
{
    if (values.empty())
        return T {};
    std::sort (values.begin(), values.end());
    const auto rank = (std::size_t) std::ceil (fraction * (double) values.size());
    return values[std::clamp<std::size_t> (rank, 1, values.size()) - 1];
}

// Loads the plugin cold, in this fresh process, from a description (no scan,
// as a host opening a project does), prepares it, and plays `blocks` timed
// blocks after `warm-up` untimed ones. Host writes come on stdin, one per
// line, `<block> <id> <f32 bits>`, in block order; each is set just before
// its block, and none may be in block 0, so the wrapper has seen the
// transport playing first. Prints one line of `name=value` pairs: the times
// to create and prepare; each timed block's time in process, from the time
// stamp counter; the cycles the whole process, the main thread and the audio
// thread ran while timed; and memory before the plugin and at its peak.
int measure (int argc, char* argv[])
{
    if (argc < 9)
    {
        std::fprintf (stderr, "usage: bench-hosted <bundle> measure <description.xml> <rate> <frames> <warm-up blocks> <blocks> <audio out | -> [idle | spin]\n");
        return 2;
    }
    const auto bundle = juce::File (juce::String::fromUTF8 (argv[1]));
    const auto xml = juce::parseXML (juce::File (juce::String::fromUTF8 (argv[3])));
    juce::PluginDescription description;
    if (xml == nullptr || ! description.loadFromXml (*xml))
    {
        std::fprintf (stderr, "no description in %s\n", argv[3]);
        return 1;
    }
    // A scan names the bundle, or, with no moduleinfo.json in it, the module
    // inside it.
    const auto named = juce::File (description.fileOrIdentifier);
    if (named != bundle && ! named.isAChildOf (bundle))
    {
        std::fprintf (stderr, "the description names %s, not %s\n",
                      description.fileOrIdentifier.toRawUTF8(), bundle.getFullPathName().toRawUTF8());
        return 1;
    }

    Run run;
    run.rate = std::stod (argv[4]);
    run.frames = std::stoi (argv[5]);
    run.warmUp = std::stoi (argv[6]);
    run.blocks = std::stoi (argv[7]);
    const std::string audioOut = argv[8];
    const std::string pace = argc > 9 ? argv[9] : "idle";
    run.spin = pace == "spin";
    if (run.frames < 1 || run.warmUp < 1 || run.blocks < 1 || (pace != "idle" && pace != "spin"))
    {
        std::fprintf (stderr, "frames, warm-up and blocks must each be at least 1, and the pace idle or spin\n");
        return 2;
    }

    struct Pending
    {
        int block;
        std::string id;
        float value;
    };
    std::vector<Pending> pending;
    for (std::string block, id, bits; std::cin >> block >> id >> bits;)
        pending.push_back ({ std::stoi (block), id, std::bit_cast<float> ((std::uint32_t) std::stoul (bits, nullptr, 16)) });
    for (std::size_t i = 0; i < pending.size(); ++i)
        if (pending[i].block < 1 || pending[i].block >= run.warmUp + run.blocks
            || (i > 0 && pending[i].block < pending[i - 1].block))
        {
            std::fprintf (stderr, "a write's block is out of order or out of range: %d\n", pending[i].block);
            return 2;
        }

    run.ticks.resize ((std::size_t) run.blocks);
    run.mainThread = OpenThread (THREAD_QUERY_LIMITED_INFORMATION, FALSE, GetCurrentThreadId());
    if (audioOut != "-")
        run.captured.resize ((std::size_t) (run.warmUp + run.blocks) * (std::size_t) run.frames * 2);
    run.writes.reserve (pending.size());
    juce::AudioBuffer<float> buffer (2, run.frames);
    juce::VST3PluginFormat format;
    const auto baseline = memory();

    juce::String error;
    const auto createFrom = Clock::now();
    auto plugin = format.createInstanceFromDescription (description, run.rate, run.frames, error);
    const auto created = Clock::now();
    if (plugin == nullptr)
    {
        std::fprintf (stderr, "create failed: %s\n", error.toRawUTF8());
        return 1;
    }
    if (plugin->getTotalNumInputChannels() != 2 || plugin->getTotalNumOutputChannels() != 2)
    {
        std::fprintf (stderr, "not stereo in and out\n");
        return 1;
    }

    Playing head;
    head.rate = run.rate;
    plugin->setPlayHead (&head);
    const auto prepareFrom = Clock::now();
    plugin->prepareToPlay (run.rate, run.frames);
    const auto prepared = Clock::now();

    const auto byId = parametersById (*plugin);
    for (const auto& write : pending)
    {
        const auto found = byId.find (write.id);
        if (found == byId.end())
        {
            std::fprintf (stderr, "unknown id %s\n", write.id.c_str());
            return 1;
        }
        run.writes.push_back ({ write.block, found->second, write.value });
    }

    const auto done = CreateEventW (nullptr, TRUE, FALSE, nullptr);
    std::thread audio ([&]
    {
        playPaced (*plugin, head, buffer, run);
        SetEvent (done);
    });
    pumpMessagesUntil (done);
    audio.join();
    CloseHandle (done);
    plugin->releaseResources();
    const auto peak = memory();
    CloseHandle (run.mainThread);

    // The counter's rate, from its ticks over the timed span's wall time,
    // turns ticks into nanoseconds.
    const auto ticksPerNanosecond = (double) run.wallTicks / (double) std::max<std::int64_t> (run.wallNanoseconds, 1);
    const auto nanoseconds = [&] (std::uint64_t ticks) { return (double) ticks / ticksPerNanosecond; };
    const auto periodTicks = (double) run.frames / run.rate * 1e9 * ticksPerNanosecond;
    const auto overruns = std::count_if (run.ticks.begin(), run.ticks.end(),
                                         [&] (std::uint64_t ticks) { return (double) ticks > periodTicks; });

    std::printf ("create_ns=%lld prepare_ns=%lld tsc_hz=%.0f blocks=%d first_ns=%.1f median_ns=%.1f p90_ns=%.1f "
                 "p99_ns=%.1f p999_ns=%.1f max_ns=%.1f median_ticks=%llu total_ticks=%llu process_cycles=%llu "
                 "main_cycles=%llu audio_cycles=%llu wall_ns=%lld overruns=%lld "
                 "base_working_set=%zu peak_working_set=%zu base_private=%zu peak_private=%zu\n",
                 (long long) nanosecondsBetween (createFrom, created),
                 (long long) nanosecondsBetween (prepareFrom, prepared),
                 ticksPerNanosecond * 1e9,
                 run.blocks,
                 nanoseconds (run.firstTicks),
                 nanoseconds (percentile (run.ticks, 0.5)),
                 nanoseconds (percentile (run.ticks, 0.9)),
                 nanoseconds (percentile (run.ticks, 0.99)),
                 nanoseconds (percentile (run.ticks, 0.999)),
                 nanoseconds (percentile (run.ticks, 1.0)),
                 (unsigned long long) percentile (run.ticks, 0.5),
                 (unsigned long long) std::accumulate (run.ticks.begin(), run.ticks.end(), std::uint64_t {}),
                 (unsigned long long) run.wholeProcessCycles,
                 (unsigned long long) run.mainThreadCycles,
                 (unsigned long long) run.audioThreadCycles,
                 (long long) run.wallNanoseconds,
                 (long long) overruns,
                 baseline.workingSet, peak.peakWorkingSet, baseline.privateBytes, peak.peakPrivateBytes);

    if (! run.captured.empty())
    {
        std::ofstream out (audioOut, std::ios::binary);
        out.write (reinterpret_cast<const char*> (run.captured.data()),
                   (std::streamsize) (run.captured.size() * sizeof (float)));
        if (! out)
        {
            std::fprintf (stderr, "could not write %s\n", audioOut.c_str());
            return 1;
        }
    }
    return 0;
}

} // namespace

int main (int argc, char* argv[])
{
    juce::ScopedJuceInitialiser_GUI juce;
    if (argc < 3)
    {
        std::fprintf (stderr, "usage: bench-hosted <bundle> info | state <id> | audio | describe | measure ...\n");
        return 2;
    }

    const std::string mode = argv[2];
    if (mode == "measure")
        return measure (argc, argv);

    juce::VST3PluginFormat format;
    juce::OwnedArray<juce::PluginDescription> types;
    format.findAllTypesForFile (types, juce::String::fromUTF8 (argv[1]));
    if (types.isEmpty())
    {
        std::fprintf (stderr, "no plugin found in %s\n", argv[1]);
        return 1;
    }
    const auto& description = *types[0];
    if (mode == "describe")
    {
        std::printf ("%s\n", description.createXml()->toString().toRawUTF8());
        return 0;
    }

    juce::String error;
    auto plugin = format.createInstanceFromDescription (description, 48000.0, 512, error);
    if (plugin == nullptr)
    {
        std::fprintf (stderr, "create failed: %s\n", error.toRawUTF8());
        return 1;
    }

    if (mode == "info")
        return describe (*plugin, description);
    if (mode == "state" && argc >= 4)
        return roundTrip (format, description, *plugin, argv[3]);
    if (mode == "audio")
        return play (*plugin);
    std::fprintf (stderr, "unknown mode %s\n", mode.c_str());
    return 2;
}
