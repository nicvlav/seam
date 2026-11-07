// bench-parity: what the JUCE EQ stores and computes, for parity.py to set
// against the Rust EQ's answers to the same cases (examples/eq/examples/
// parity.rs). Every number is its bits in hex.
//
//   bench-parity ranges                 the parameter table, as the Rust one prints it
//   bench-parity snap < key input       each value through the range's own snap
//   bench-parity restore < key input    each value set as a JUCE plugin's own code or a
//                                       state restore sets it: to 0..1 and back, snapped
//   bench-parity normalized < key n     each 0..1 through AudioParameterFloat::setValue,
//                                       as a host's automation reaches it
//   bench-parity coefficients < shape gain frequency q rate
//                                       JUCE's coefficients, normalized by a0 (with
//                                       BENCH_PORTED_DSP, EqPort.h's)
//   bench-parity audio < script         the JUCE EQ's processor playing a script: `rate r`,
//                                       `set key n` (a host's 0..1), `block frames`, `reset`;
//                                       prints `left right` for every sample out

#include <bit>
#include <cstdint>
#include <cstdio>
#include <iostream>
#include <map>
#include <memory>
#include <string>

#include <juce_audio_processors/juce_audio_processors.h>
#include <juce_dsp/juce_dsp.h>

#include "../juce-eq/EqParameters.h"
#include "../juce-eq/PluginProcessor.h"

namespace
{
    float floatOf (const std::string& hex)
    {
        return std::bit_cast<float> (static_cast<std::uint32_t> (std::stoul (hex, nullptr, 16)));
    }

    double doubleOf (const std::string& hex)
    {
        return std::bit_cast<double> (static_cast<std::uint64_t> (std::stoull (hex, nullptr, 16)));
    }

    void printBits (float value)
    {
        std::printf ("%08x", std::bit_cast<std::uint32_t> (value));
    }

    /** Every parameter, made as the plugin makes it. */
    std::map<std::string, std::unique_ptr<juce::RangedAudioParameter>> parameters()
    {
        std::map<std::string, std::unique_ptr<juce::RangedAudioParameter>> made;

        for (const auto& spec : bench::params)
            made.emplace (spec.key, bench::makeParameter (spec));

        return made;
    }

    /** The value a parameter holds, in real units. */
    float valueOf (juce::RangedAudioParameter& parameter)
    {
        if (auto* flag = dynamic_cast<juce::AudioParameterBool*> (&parameter))
            return flag->get() ? 1.0f : 0.0f;

        return dynamic_cast<juce::AudioParameterFloat&> (parameter).get();
    }

    int ranges()
    {
        for (const auto& spec : bench::params)
        {
            std::printf ("%s %s ", spec.key, spec.isBool ? "bool" : "continuous");
            printBits (spec.start);
            std::printf (" ");
            printBits (spec.end);
            std::printf (" ");
            printBits (spec.interval);
            std::printf (" ");
            printBits (spec.skewCentre);
            std::printf (" ");
            printBits (spec.defaultValue);
            std::printf ("\n");
        }

        return 0;
    }

    /** Reads `key value` lines and prints `key value result`. */
    template <typename Through>
    int perParameter (Through through)
    {
        auto made = parameters();
        std::string key, hex;

        while (std::cin >> key >> hex)
        {
            auto found = made.find (key);

            if (found == made.end())
            {
                std::fprintf (stderr, "unknown key %s\n", key.c_str());
                return 1;
            }

            std::printf ("%s %s ", key.c_str(), hex.c_str());
            printBits (through (*found->second, floatOf (hex)));
            std::printf ("\n");
        }

        return 0;
    }

    /** xorshift32 as a signed 32-bit integer over 2^31, as the Rust side
        makes it. */
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

    int audio()
    {
        bench::JuceEq processor;
        juce::AudioBuffer<float> buffer (2, 4096);
        juce::MidiBuffer midi;
        Noise noise;
        std::map<std::string, juce::RangedAudioParameter*> byKey;
        for (auto* parameter : processor.getParameters())
            if (auto* ranged = dynamic_cast<juce::RangedAudioParameter*> (parameter))
                byKey.emplace (ranged->getParameterID().toStdString(), ranged);
        std::string word;

        while (std::cin >> word)
        {
            if (word == "rate")
            {
                double rate = 0.0;
                std::cin >> rate;
                processor.setRateAndBufferSizeDetails (rate, 4096);
                processor.prepareToPlay (rate, 4096);
            }
            else if (word == "set")
            {
                std::string key, hex;
                std::cin >> key >> hex;
                const auto found = byKey.find (key);
                if (found == byKey.end())
                {
                    std::fprintf (stderr, "unknown key %s\n", key.c_str());
                    return 1;
                }
                found->second->setValue (floatOf (hex));
            }
            else if (word == "reset")
                processor.reset();
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
                processor.processBlock (buffer, midi);
                for (int frame = 0; frame < frames; ++frame)
                {
                    printBits (buffer.getSample (0, frame));
                    std::printf (" ");
                    printBits (buffer.getSample (1, frame));
                    std::printf ("\n");
                }
            }
            else
            {
                std::fprintf (stderr, "unknown script word %s\n", word.c_str());
                return 1;
            }
        }

        return 0;
    }

    int coefficients()
    {
        std::string shape, gain, frequency, q, rate;

        while (std::cin >> shape >> gain >> frequency >> q >> rate)
        {
            const auto kind = shape == "hp"     ? bench::Shape::highPass
                              : shape == "low"  ? bench::Shape::lowShelf
                              : shape == "peak" ? bench::Shape::peak
                                                : bench::Shape::highShelf;
#if BENCH_PORTED_DSP
            const auto normalized = bench::port::coefficients (kind, floatOf (gain), floatOf (frequency), floatOf (q), doubleOf (rate));
#else
            juce::dsp::IIR::Coefficients<float> made;
            made = bench::rawCoefficients (kind, floatOf (gain), floatOf (frequency), floatOf (q), doubleOf (rate));
            const auto& normalized = made.coefficients;
#endif
            std::printf ("%s %s %s %s %s", shape.c_str(), gain.c_str(), frequency.c_str(), q.c_str(), rate.c_str());

            for (const auto coefficient : normalized)
            {
                std::printf (" ");
                printBits (coefficient);
            }

            std::printf ("\n");
        }

        return 0;
    }
} // namespace

int main (int argc, char* argv[])
{
    const std::string mode = argc > 1 ? argv[1] : "";

    if (mode == "ranges")
        return ranges();

    if (mode == "snap")
        return perParameter ([] (juce::RangedAudioParameter& parameter, float value) {
            return parameter.getNormalisableRange().snapToLegalValue (value);
        });

    if (mode == "restore")
        return perParameter ([] (juce::RangedAudioParameter& parameter, float value) {
            parameter.setValue (parameter.convertTo0to1 (value));
            return valueOf (parameter);
        });

    if (mode == "normalized")
        return perParameter ([] (juce::RangedAudioParameter& parameter, float normalized) {
            parameter.setValue (normalized);
            return valueOf (parameter);
        });

    if (mode == "coefficients")
        return coefficients();

    if (mode == "audio")
        return audio();

    std::fprintf (stderr, "usage: bench-parity ranges|snap|restore|normalized|coefficients|audio\n");
    return 2;
}
