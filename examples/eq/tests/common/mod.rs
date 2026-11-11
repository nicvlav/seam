//! What the EQ's tests share: the EQ built as an exporter builds it, its
//! declarations, the noise they play, and samples as bits, to compare outputs
//! exactly.
#![allow(dead_code, reason = "each test binary uses only some of it")]

use std::sync::Arc;

use seam_example_eq::ExampleEq;
use seam_example_eq::params::EqParams;
use seam_host::{Parts, Plugin, SavedState};
use seam_params::{ParamDecl, ParamSet};

/// The EQ, built as an exporter builds it, and the saved state a host's save
/// and load go through.
pub(crate) fn built() -> (ExampleEq, Arc<SavedState>) {
    let (params, parts) = Parts::for_plugin::<ExampleEq>().expect("valid declarations");
    let saved = Arc::clone(parts.saved());
    (ExampleEq::new(params, parts), saved)
}

/// Every parameter's declaration, in the order hosts list them.
pub(crate) fn declarations() -> Vec<ParamDecl> {
    let (set, _) = ParamSet::declare::<EqParams>().expect("valid declarations");
    set.ids().map(|id| *set.decl(id)).collect()
}

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
