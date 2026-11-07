//! The real-time sanitizer's canary: a processor that allocates, played
//! through the C interface, whose `process` carries the nonblocking mark.
//!
//! It is built only for the nightly sanitizer lane (`--cfg seam_rtsan`),
//! which expects the test to abort. A canary that passes shows the lane
//! checks nothing: the marks are missing, or the sanitizer is not linked.
#![cfg(all(seam_rtsan, feature = "cabi", feature = "test-host"))]

use std::sync::Arc;

use seam_host::cabi::TestHost;
use seam_host::{
    Block, Category, Layout, Plugin, PluginInfo, ProcessSetup, Processor, SavedState, Status,
};
use seam_params::{Kind, ParamDecl, ParamSet};
use seam_session::Session;

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

struct Canary(Arc<SavedState>);

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
        au_codes: (*b"cnry", *b"Seam"),
        category: Category::Effect,
        features: &[],
        layouts: &[Layout::STEREO],
        midi_input: false,
        midi_output: false,
    };
    const PARAMS: &'static [ParamDecl] = &[ParamDecl::new(
        "gain",
        "Gain",
        Kind::continuous(0.0, 1.0, 0.0, None),
        0.0,
    )];
    type Processor = Allocates;

    fn new() -> Self {
        let set = ParamSet::from_decls(Self::PARAMS.iter().copied()).expect("valid");
        Canary(Arc::new(SavedState::new(&set, &Session::new(&set))))
    }

    fn saved(&self) -> Arc<SavedState> {
        Arc::clone(&self.0)
    }

    fn activate(&mut self, _setup: &ProcessSetup) -> Allocates {
        Allocates
    }
}

#[test]
fn an_allocation_inside_the_marked_process_aborts() {
    let host = TestHost::<Canary>::new();
    assert!(host.activate(48_000.0, 64, Layout::STEREO));
    let (mut left, mut right) = ([1.0_f32; 64], [1.0_f32; 64]);
    host.process_in_place(&mut [&mut left, &mut right], 64);
    // Reached only when the sanitizer let the allocation through.
    assert_eq!(left, [0.0; 64]);
}
