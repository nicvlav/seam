//! What the exporters' tests share: the noise they play, and samples as
//! bits, to compare outputs exactly.

/// xorshift32, as a signed 32-bit integer over 2^31.
pub(crate) struct Noise(pub(crate) u32);

impl Noise {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }

    pub(crate) fn sample(&mut self) -> f32 {
        self.next() as i32 as f32 / 2_147_483_648.0
    }

    pub(crate) fn below(&mut self, count: usize) -> usize {
        self.next() as usize % count
    }

    pub(crate) fn unit(&mut self) -> f64 {
        f64::from(self.next()) / f64::from(u32::MAX)
    }
}

/// The samples' bit patterns, so a comparison is exact and shows any NaN.
pub(crate) fn bits(samples: &[f32]) -> Vec<u32> {
    samples.iter().map(|sample| sample.to_bits()).collect()
}
