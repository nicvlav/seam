//! The generated files: what `cargo xtask gen` writes and the drift gate
//! compares.

use std::sync::Arc;

use seam_params::{Kind, ParamDecl, ParamSet, ParamSetBuilder};

/// One generated file.
#[derive(Debug)]
pub struct Generated {
    /// Where it lives, from the repository root, with `/` separators.
    pub path: &'static str,
    /// What it holds.
    pub contents: String,
}

/// Returns every file the repository commits that Rust generates.
pub fn generated() -> Vec<Generated> {
    vec![
        Generated {
            path: "page/src/generated/protocol.ts",
            contents: seam_bridge::typescript::protocol(),
        },
        Generated {
            path: "page/test/generated/params.ts",
            contents: seam_bridge::typescript::param_map(
                &page_test_params(),
                "xtask/src/generate.rs (page_test_params)",
            ),
        },
        Generated {
            path: "crates/host/include/seam.h",
            contents: seam_host::cabi::header(),
        },
        Generated {
            path: "examples/eq/seam-juce.cmake",
            contents: seam_host::cabi::juce_settings::<seam_example_eq::ExampleEq>(
                "examples/eq/src/lib.rs (ExampleEq::INFO)",
            ),
        },
    ]
}

/// Returns the parameters the page's own tests bind.
///
/// They are declared here so the page is typed by a generated map, as a
/// plugin's page is.
#[expect(
    clippy::missing_panics_doc,
    reason = "the four declarations below are fixed and valid, and the drift test builds them"
)]
pub fn page_test_params() -> Arc<ParamSet> {
    let continuous = |start, end, skew_centre| Kind::continuous(start, end, 0.0, skew_centre);
    let param = |key, name, unit, kind, default, bypass: bool| {
        let decl = ParamDecl::new(key, name, kind, default).with_unit(unit);
        if bypass { decl.as_bypass() } else { decl }
    };
    let mut builder = ParamSetBuilder::new();
    for decl in [
        param(
            "gain",
            "Gain",
            "dB",
            continuous(-60.0, 12.0, Some(-12.0)),
            0.0,
            false,
        ),
        param("pan", "Pan", "", continuous(-1.0, 1.0, None), 0.0, false),
        param(
            "mode",
            "Mode",
            "",
            Kind::Choice(&["Clean", "Warm"]),
            0.0,
            false,
        ),
        param("bypass", "Bypass", "", Kind::Bool, 0.0, true),
    ] {
        builder
            .add(decl)
            .expect("the page test parameters are valid");
    }
    builder.build()
}
