//! A controller's value as 0 to 1.
//!
//! MIDI 1.0 values normalise centre-anchored, as the MIDI 2.0 specification
//! translates them, so 0, the centre and the maximum land on 0, 0.5 and 1
//! at every width. MIDI 2.0 values are linear: `v / max`.

// Audio-path code: a panic here ends a host, so indexing, unwrap, expect,
// panic and unchecked arithmetic are refused, and a site that is provably
// safe says why in an `#[expect]`.
#![cfg_attr(
    not(test),
    deny(
        clippy::indexing_slicing,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::arithmetic_side_effects
    )
)]

/// Centre-anchored: `v ≤ c` gives `v / (2c)`, otherwise
/// `0.5 + (v − c) / (2(max − c))`.
fn centred(value: u32, centre: u32, max: u32) -> f64 {
    let value = f64::from(value.min(max));
    let (centre, max) = (f64::from(centre), f64::from(max));
    if value <= centre {
        value / (2.0 * centre)
    } else {
        0.5 + (value - centre) / (2.0 * (max - centre))
    }
}

/// A MIDI 1.0 7-bit value: a controller or a velocity.
pub(crate) fn seven(value: u8) -> f64 {
    centred(u32::from(value), 64, 127)
}

/// A MIDI 1.0 14-bit value: an NRPN's data entry.
pub(crate) fn fourteen(value: u16) -> f64 {
    centred(u32::from(value), 8192, 16383)
}

/// A MIDI 2.0 16-bit velocity.
pub(crate) fn sixteen(value: u16) -> f64 {
    f64::from(value) / f64::from(u16::MAX)
}

/// A MIDI 2.0 32-bit controller value.
pub(crate) fn thirty_two(value: u32) -> f64 {
    f64::from(value) / f64::from(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MIDI 1.0 to MIDI 2.0 value translation, as the specification's
    /// Min-Center-Max scaling gives it.
    ///
    /// A shift, and above the centre the lower bits repeated. Ported from
    /// JUCE 9.0.0's `Conversion::scaleTo16` and `Conversion::scaleTo32`
    /// (`juce_UMPConversion.h`), which is how a host's translation reaches a
    /// plugin.
    fn up_7_to_16(value: u8) -> u16 {
        let shifted = u16::from(value) << 9;
        let repeat = u16::from(value) & 0x3F;
        let mask = if value <= 0x40 { 0 } else { 0xFFFF };
        shifted | (((repeat << 3) | (repeat >> 3)) & mask)
    }

    fn up_7_to_32(value: u8) -> u32 {
        let shifted = u32::from(value) << 25;
        let repeat = u32::from(value) & 0x3F;
        let mask = if value <= 0x40 { 0 } else { u32::MAX };
        shifted
            | (((repeat << 19) | (repeat << 13) | (repeat << 7) | (repeat << 1) | (repeat >> 5))
                & mask)
    }

    fn up_14_to_32(value: u16) -> u32 {
        let shifted = u32::from(value) << 18;
        let repeat = u32::from(value) & 0x1FFF;
        let mask = if value <= 0x2000 { 0 } else { u32::MAX };
        shifted | (((repeat << 5) | (repeat >> 8)) & mask)
    }

    #[test]
    fn zero_the_centre_and_the_maximum_land_on_0_half_and_1() {
        assert_eq!([seven(0), seven(64), seven(127)], [0.0, 0.5, 1.0]);
        assert_eq!(
            [fourteen(0), fourteen(8192), fourteen(16383)],
            [0.0, 0.5, 1.0]
        );
        assert_eq!([sixteen(0), sixteen(u16::MAX)], [0.0, 1.0]);
        assert_eq!([thirty_two(0), thirty_two(u32::MAX)], [0.0, 1.0]);
        // Centre-anchoring differs from plain 64 / 127.
        assert!((64.0 / 127.0 - seven(64)) > 3e-3);
    }

    #[test]
    fn values_only_ever_rise() {
        for value in 1..=127 {
            assert!(seven(value) > seven(value - 1));
        }
        for value in 1..=16383 {
            assert!(fourteen(value) > fourteen(value - 1));
        }
    }

    #[test]
    fn a_midi_1_controller_reads_the_same_in_a_hosts_midi_2_translation() {
        // Within 1.3e-5, at both MIDI 1.0 widths, and for a velocity.
        let mut worst = 0.0_f64;
        for value in 0..=127_u8 {
            worst = worst.max((seven(value) - thirty_two(up_7_to_32(value))).abs());
            worst = worst.max((seven(value) - sixteen(up_7_to_16(value))).abs());
        }
        for value in 0..=16383_u16 {
            worst = worst.max((fourteen(value) - thirty_two(up_14_to_32(value))).abs());
        }
        assert!(worst <= 1.3e-5, "worst {worst:e}");
    }

    #[test]
    fn linear_midi_2_values_stay_within_a_bound_of_centre_anchoring() {
        // Within 1e-5 of centre-anchoring at 16 bits, 1.2e-10 at 32.
        let mut worst = 0.0_f64;
        for value in 0..=u16::MAX {
            let centre = centred(u32::from(value), 0x8000, u32::from(u16::MAX));
            worst = worst.max((sixteen(value) - centre).abs());
        }
        assert!(worst <= 1e-5, "16-bit worst {worst:e}");

        let mut worst = 0.0_f64;
        let around_centre = (0x8000_0000_u32 - 4096)..=(0x8000_0000 + 4096);
        let stride = (0..=u32::MAX).step_by(65_521);
        for value in around_centre.chain(stride).chain([u32::MAX]) {
            let centre = centred(value, 0x8000_0000, u32::MAX);
            worst = worst.max((thirty_two(value) - centre).abs());
        }
        assert!(worst <= 1.2e-10, "32-bit worst {worst:e}");
    }
}
