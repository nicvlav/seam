//! The filters: JUCE's `dsp::IIR` coefficient formulas in single precision
//! and its order-2 transposed direct form, with the denormal snap it makes
//! at the end of every block.
//!
//! The same EQ written in JUCE matches this one sample for sample when it
//! runs on x86-64 (`JUCE_SNAP_TO_ZERO` does nothing elsewhere), with the
//! same C runtime for `powf`, `sinf`, `cosf` and `tanf`, on the same
//! parameter values (its ranges must snap as Seam's do), and clamping
//! each frequency to the Nyquist frequency as [`coefficients`] does, which
//! JUCE's `make*` functions only assert.

// Audio-path code: a panic here ends a host. A site that is provably safe
// says why in an `#[expect]`.
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

use std::f32::consts::PI;

/// What a band does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// A second-order high-pass; its gain is unused.
    HighPass,
    /// A low shelf.
    LowShelf,
    /// A peak.
    Peak,
    /// A high shelf.
    HighShelf,
}

/// `std::powf` from the C runtime, never folded at compile time, as a C++
/// build calls it.
fn powf(base: f32, exponent: f32) -> f32 {
    std::hint::black_box(base).powf(exponent)
}

/// `Decibels::decibelsToGain` with its default floor of -100 dB.
pub(crate) fn decibels_to_gain(decibels: f32) -> f32 {
    if decibels > -100.0 {
        powf(10.0, decibels * 0.05)
    } else {
        0.0
    }
}

/// `Decibels::gainWithLowerBound(gain, -300)`.
fn gain_with_lower_bound(gain: f32) -> f32 {
    gain.max(powf(10.0, -300.0 * 0.05))
}

/// `JUCE_SNAP_TO_ZERO`.
fn snap_to_zero(value: f32) -> f32 {
    if !(value < -1.0e-8 || value > 1.0e-8) {
        0.0
    } else {
        value
    }
}

/// Returns a band's coefficients, b0, b1, b2, a1 and a2, normalized by a0
/// (JUCE's `make*` functions and `Coefficients::assignImpl`). `gain_db` is
/// unused by the high-pass.
pub fn coefficients(
    shape: Shape,
    gain_db: f32,
    frequency_hz: f32,
    q: f32,
    sample_rate: f64,
) -> [f32; 5] {
    let frequency = frequency_hz.clamp(1.0, (sample_rate / 2.0) as f32);
    let rate = sample_rate as f32;
    let [b0, b1, b2, a0, a1, a2] = match shape {
        Shape::HighPass => {
            let n = (PI * frequency / rate).tan();
            let n_squared = n * n;
            let inv_q = 1.0 / q;
            let c1 = 1.0 / (1.0 + inv_q * n + n_squared);
            [
                c1,
                c1 * -2.0,
                c1,
                1.0,
                c1 * 2.0 * (n_squared - 1.0),
                c1 * (1.0 - inv_q * n + n_squared),
            ]
        }
        Shape::Peak => {
            let a = gain_with_lower_bound(decibels_to_gain(gain_db)).sqrt();
            let omega = (2.0 * PI * frequency.max(2.0)) / rate;
            let alpha = omega.sin() / (q * 2.0);
            let c2 = -2.0 * omega.cos();
            let alpha_times_a = alpha * a;
            let alpha_over_a = alpha / a;
            [
                1.0 + alpha_times_a,
                c2,
                1.0 - alpha_times_a,
                1.0 + alpha_over_a,
                c2,
                1.0 - alpha_over_a,
            ]
        }
        Shape::LowShelf | Shape::HighShelf => {
            let a = gain_with_lower_bound(decibels_to_gain(gain_db)).sqrt();
            let a_minus_1 = a - 1.0;
            let a_plus_1 = a + 1.0;
            let omega = (2.0 * PI * frequency.max(2.0)) / rate;
            let cos = omega.cos();
            let beta = omega.sin() * a.sqrt() / q;
            let a_minus_1_times_cos = a_minus_1 * cos;
            if shape == Shape::LowShelf {
                [
                    a * (a_plus_1 - a_minus_1_times_cos + beta),
                    a * 2.0 * (a_minus_1 - a_plus_1 * cos),
                    a * (a_plus_1 - a_minus_1_times_cos - beta),
                    a_plus_1 + a_minus_1_times_cos + beta,
                    -2.0 * (a_minus_1 + a_plus_1 * cos),
                    a_plus_1 + a_minus_1_times_cos - beta,
                ]
            } else {
                [
                    a * (a_plus_1 + a_minus_1_times_cos + beta),
                    a * -2.0 * (a_minus_1 + a_plus_1 * cos),
                    a * (a_plus_1 + a_minus_1_times_cos - beta),
                    a_plus_1 - a_minus_1_times_cos + beta,
                    2.0 * (a_minus_1 - a_plus_1 * cos),
                    a_plus_1 - a_minus_1_times_cos - beta,
                ]
            }
        }
    };
    // Multiplied by 1/a0, or by 0 when a0 is about 0.
    let inverse = if a0.abs() > f32::MIN_POSITIVE {
        1.0 / a0
    } else {
        0.0
    };
    [b0, b1, b2, a1, a2].map(|coefficient| coefficient * inverse)
}

/// One second-order section over up to two channels.
#[derive(Debug, Clone, Default)]
pub(crate) struct Biquad {
    /// From [`coefficients()`]; a retune replaces them and keeps the state.
    pub(crate) coefficients: [f32; 5],
    /// Per channel, the two state values of the transposed direct form.
    state: [[f32; 2]; 2],
}

impl Biquad {
    /// Starts from silence.
    pub(crate) fn reset(&mut self) {
        self.state = [[0.0; 2]; 2];
    }

    /// Filters the first `frames` samples of up to two channels in place;
    /// `bypassed`, it still runs, but the samples keep their input.
    pub(crate) fn process(&mut self, channels: &mut [&mut [f32]], frames: usize, bypassed: bool) {
        let [b0, b1, b2, a1, a2] = self.coefficients;
        for (samples, state) in channels.iter_mut().zip(&mut self.state) {
            let [mut lv1, mut lv2] = *state;
            for sample in samples.iter_mut().take(frames) {
                let input = *sample;
                let output = (input * b0) + lv1;
                if !bypassed {
                    *sample = output;
                }
                lv1 = (input * b1) - (output * a1) + lv2;
                lv2 = (input * b2) - (output * a2);
            }
            *state = [snap_to_zero(lv1), snap_to_zero(lv2)];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    /// The Audio EQ Cookbook's coefficients in f64, an oracle written apart
    /// from JUCE's forms: b0, b1, b2, a0, a1, a2.
    fn cookbook(shape: Shape, gain_db: f64, frequency: f64, q: f64) -> [f64; 6] {
        let w0 = 2.0 * std::f64::consts::PI * frequency / RATE;
        let (cos, sin) = (w0.cos(), w0.sin());
        let alpha = sin / (2.0 * q);
        let a = 10f64.powf(gain_db / 40.0);
        let root = 2.0 * a.sqrt() * alpha;
        match shape {
            Shape::HighPass => [
                (1.0 + cos) / 2.0,
                -(1.0 + cos),
                (1.0 + cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ],
            Shape::Peak => [
                1.0 + alpha * a,
                -2.0 * cos,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cos,
                1.0 - alpha / a,
            ],
            Shape::LowShelf => [
                a * ((a + 1.0) - (a - 1.0) * cos + root),
                2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                a * ((a + 1.0) - (a - 1.0) * cos - root),
                (a + 1.0) + (a - 1.0) * cos + root,
                -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                (a + 1.0) + (a - 1.0) * cos - root,
            ],
            Shape::HighShelf => [
                a * ((a + 1.0) + (a - 1.0) * cos + root),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                a * ((a + 1.0) + (a - 1.0) * cos - root),
                (a + 1.0) - (a - 1.0) * cos + root,
                2.0 * ((a - 1.0) - (a + 1.0) * cos),
                (a + 1.0) - (a - 1.0) * cos - root,
            ],
        }
    }

    /// |H| in dB at `frequency`, for b0, b1, b2 over a0, a1, a2.
    fn magnitude_db(b: [f64; 3], a: [f64; 3], frequency: f64) -> f64 {
        let w = 2.0 * std::f64::consts::PI * frequency / RATE;
        let at = |c: [f64; 3]| {
            let re = c[0] + c[1] * w.cos() + c[2] * (2.0 * w).cos();
            let im = -c[1] * w.sin() - c[2] * (2.0 * w).sin();
            re.hypot(im)
        };
        20.0 * (at(b) / at(a)).log10()
    }

    #[test]
    fn every_shape_has_the_cookbook_response() {
        // Q away from 1 and gains away from 0, so a Q or gain term in the
        // wrong place shows as decibels; f32 coefficients alone stay within
        // a few thousandths of a decibel here.
        for (shape, gain_db, frequency, q) in [
            (Shape::HighPass, 0.0, 120.0, 2.5),
            (Shape::LowShelf, 9.0, 300.0, 0.6),
            (Shape::Peak, -7.5, 1_000.0, 3.0),
            (Shape::HighShelf, 6.0, 4_000.0, 1.8),
        ] {
            let ours = coefficients(shape, gain_db as f32, frequency as f32, q as f32, RATE);
            let [b0, b1, b2, a1, a2] = ours.map(f64::from);
            let oracle = cookbook(shape, gain_db, frequency, q);
            for probe in [frequency / 3.0, frequency, frequency * 3.0] {
                let expected = magnitude_db(
                    [oracle[0], oracle[1], oracle[2]],
                    [oracle[3], oracle[4], oracle[5]],
                    probe,
                );
                let got = magnitude_db([b0, b1, b2], [1.0, a1, a2], probe);
                assert!(
                    (got - expected).abs() < 0.01,
                    "{shape:?} at {probe} Hz: {got} dB, the cookbook says {expected} dB"
                );
            }
        }
    }

    #[test]
    fn the_state_snaps_to_zero_at_the_end_of_a_block() {
        // JUCE_SNAP_TO_ZERO: a decaying tail below 1e-8 becomes exactly 0, so
        // a filter fed silence ends in silence and never runs on denormals.
        let mut filter = Biquad {
            coefficients: coefficients(Shape::HighPass, 0.0, 16.0, 1.0, RATE),
            ..Biquad::default()
        };
        let mut impulse = [0.0f32; 64];
        impulse[0] = 1e-6;
        filter.process(&mut [&mut impulse], 64, false);
        let mut silence = [0.0f32; 64];
        for _ in 0..200 {
            silence = [0.0; 64];
            filter.process(&mut [&mut silence], 64, false);
        }
        assert!(
            silence.iter().all(|sample| sample.to_bits() == 0),
            "{silence:?}"
        );
    }

    #[test]
    fn a_bypassed_filter_keeps_its_input_and_still_runs() {
        let tuned = Biquad {
            coefficients: coefficients(Shape::Peak, 12.0, 200.0, 2.0, RATE),
            ..Biquad::default()
        };
        let (mut running, mut bypassed) = (tuned.clone(), tuned);
        let signal: Vec<f32> = (0..256).map(|n| (n as f32 * 0.1).sin()).collect();
        let (mut a, mut b) = (signal.clone(), signal.clone());
        running.process(&mut [&mut a], 256, false);
        bypassed.process(&mut [&mut b], 256, true);
        assert_eq!(b, signal, "bypassed, the samples keep their input");
        assert_eq!(
            running.state, bypassed.state,
            "and the state follows the signal"
        );
    }
}
