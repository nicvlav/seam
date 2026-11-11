//! The real-time sanitizer's canary: a processor that allocates, played
//! through the C interface, whose `process` carries the nonblocking mark.
//!
//! It is built only for the nightly sanitizer lane (`--cfg seam_rtsan`),
//! which expects the test to abort. A canary that passes shows the lane
//! checks nothing: the marks are missing, or the sanitizer is not linked.
#![cfg(all(seam_rtsan, feature = "cabi", feature = "test-host"))]

use seam_host::cabi::TestHost;
use seam_host::{
    AuCodes, Block, Category, Layout, Parts, Plugin, PluginInfo, ProcessSetup, Processor, Status,
};
use seam_params::{Declarer, Float, FloatDecl, Params};

/// Allocates in every block, as no processor may.
struct Allocates;

impl Processor for Allocates {
    fn process(&mut self, block: Block<'_, '_>) -> Status {
        let scratch = std::hint::black_box(vec![0.0_f32; block.frames]);
        for channel in block.channels.iter_mut() {
            channel[..block.frames].copy_from_slice(&scratch);
        }
        Status::Continue
    }
}

struct Gain {
    _gain: Float,
}

impl Params for Gain {
    fn declare(p: &mut Declarer) -> Self {
        Gain {
            _gain: p.add(FloatDecl::new("gain", "Gain", 0.0..=1.0, 0.0)),
        }
    }
}

struct Canary;

impl Plugin for Canary {
    const INFO: PluginInfo = PluginInfo {
        name: "Canary",
        vendor: "Seam",
        url: "",
        email: "",
        description: "",
        version: "0.1.0",
        id: "dev.seam.rtsan-canary",
        vst3_class_id: [9; 16],
        au_codes: AuCodes {
            subtype: *b"cnry",
            manufacturer: *b"Seam",
        },
        category: Category::Effect,
        features: &[],
        layouts: &[Layout::STEREO],
        midi_input: false,
        midi_output: false,
    };
    type Params = Gain;
    type Processor = Allocates;

    fn new(_: Gain, _: Parts) -> Self {
        Canary
    }

    fn activate(&mut self, _setup: &ProcessSetup) -> Allocates {
        Allocates
    }
}

#[test]
fn an_allocation_inside_the_marked_process_aborts() {
    let host = TestHost::<Canary>::new();
    assert_eq!(host.activate(48_000.0, 64, Layout::STEREO), Ok(()));
    let (mut left, mut right) = ([1.0_f32; 64], [1.0_f32; 64]);
    host.process_in_place(&mut [&mut left, &mut right], 64);
    // Reached only when the sanitizer let the allocation through.
    assert_eq!(left, [0.0; 64]);
}
