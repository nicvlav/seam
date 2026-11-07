#pragma once

// Seam's continuous range, ported so this JUCE build stores the same
// parameter values as the Rust EQ, bit for bit. Every step mirrors
// crates/params/src/decl.rs: the lattice laid out in whole numbers of the
// finest decimal place, so every point of a decimal range, 0 dB included,
// lands on the nearest float to its decimal; the nearest legal value with a
// tie to the higher, the end always reachable, never -0, and the 0..1
// mapping with its skew, all in double through the same C runtime.

#include <algorithm>
#include <cmath>
#include <optional>

namespace bench
{

/** The decimal an f32 was written as: the fewest places, up to nine, that
    read back as the same f32, with its places; or the f32 itself and -1. */
struct Written
{
    double decimal;
    int places;
};

inline Written written (float value) noexcept
{
    const auto wide = static_cast<double> (value);
    auto scale = 1.0;

    for (int places = 0; places < 10; ++places)
    {
        const auto decimal = std::round (wide * scale) / scale;

        if (static_cast<float> (decimal) == value)
            return { decimal, places };

        scale *= 10.0;
    }

    return { wide, -1 };
}

/** A stepped range's lattice: the points (s + k * t) / scale for k from 0 to
    last, with s and t the start and the interval as whole numbers of
    1 / scale. */
struct Lattice
{
    double s, t, scale, last;
    float start, end;

    static std::optional<Lattice> of (float start, float end, float interval) noexcept
    {
        if (std::isnan (interval) || interval <= 0.0f)
            return std::nullopt;

        constexpr auto exact = 9007199254740992.0; // 2^53
        const auto [startDecimal, startPlaces] = written (start);
        const auto [stepDecimal, stepPlaces] = written (interval);
        const auto endDecimal = written (end).decimal;

        auto s = startDecimal, t = stepDecimal, scale = 1.0, e = endDecimal;

        if (startPlaces >= 0 && stepPlaces >= 0)
        {
            auto whole = 1.0;

            for (int place = 0; place < std::max (startPlaces, stepPlaces); ++place)
                whole *= 10.0;

            const auto scaledStart = std::round (startDecimal * whole);
            const auto scaledStep = std::round (stepDecimal * whole);
            const auto scaledEnd = endDecimal * whole;

            if (scaledStep > 0.0 && std::abs (scaledStart) < exact && std::abs (scaledEnd) < exact)
            {
                s = scaledStart;
                t = scaledStep;
                scale = whole;
                e = scaledEnd;
            }
        }

        return Lattice { s, t, scale, std::max (std::floor ((e - s) / t), 0.0), start, end };
    }

    double below (double value) const noexcept
    {
        return std::clamp (std::floor ((value * scale - s) / t), 0.0, last);
    }

    float point (double k) const noexcept
    {
        return std::clamp (static_cast<float> ((s + k * t) / scale), start, end);
    }
};

/** One continuous parameter's range, as Seam declares it. */
struct SeamRange
{
    float start, end, interval;
    std::optional<float> skewCentre;

    /** The nearest legal value to `value`, never -0 (Kind::snap). */
    float snap (float value) const noexcept
    {
        if (! std::isfinite (value))
            return start;

        float snapped;

        if (value <= start)
            snapped = start;
        else if (value >= end)
            snapped = end;
        else if (const auto lattice = Lattice::of (start, end, interval))
        {
            const auto v = static_cast<double> (value);
            const auto k = lattice->below (v);
            const auto lower = lattice->point (k);
            const auto upper = k < lattice->last ? lattice->point (k + 1.0) : end;
            snapped = std::abs (static_cast<double> (upper) - v) <= std::abs (v - static_cast<double> (lower))
                          ? upper
                          : lower;
        }
        else
            snapped = value;

        return snapped + 0.0f;
    }

    std::optional<double> skewExponent() const noexcept
    {
        if (! skewCentre)
            return std::nullopt;

        const auto s = static_cast<double> (start), e = static_cast<double> (end);
        const auto proportion = (static_cast<double> (*skewCentre) - s) / (e - s);

        if (proportion > 0.0 && proportion < 1.0)
            return std::log (0.5) / std::log (proportion);

        return std::nullopt;
    }

    /** A host's 0..1 in real units, before snapping (Kind::denormalize). */
    float denormalize (double normalized) const noexcept
    {
        normalized = std::isnan (normalized) ? 0.0 : std::clamp (normalized, 0.0, 1.0);
        const auto s = static_cast<double> (start), e = static_cast<double> (end);
        const auto exponent = skewExponent();
        const auto proportion = exponent ? std::pow (normalized, 1.0 / *exponent) : normalized;
        return static_cast<float> (s + (e - s) * proportion);
    }

    /** `value` as a host's 0..1, not snapped (Kind::normalize). */
    double normalize (float value) const noexcept
    {
        if (std::isnan (value))
            return 0.0;

        const auto s = static_cast<double> (start), e = static_cast<double> (end);
        const auto span = e - s;

        if (std::isnan (span) || span <= 0.0)
            return 0.0;

        const auto proportion = std::clamp ((static_cast<double> (value) - s) / span, 0.0, 1.0);
        const auto exponent = skewExponent();
        return std::clamp (exponent ? std::pow (proportion, *exponent) : proportion, 0.0, 1.0);
    }
};

} // namespace bench
