//! The drift gate for the page's parameter map: `page/src/generated/params.ts`
//! is the gain's own declarations, generated. It is checked here rather than
//! by `cargo xtask gen`, since xtask linking a second plugin would bring two
//! sets of VST3 entry points into one binary. Rewrite it with `SEAM_BLESS=1
//! cargo test -p seam-example-gain --test page_params`.

use std::path::Path;

use seam_example_gain::GainParams;
use seam_params::ParamSet;

/// The header's instruction for a map `cargo xtask gen` writes, and this
/// one's.
const XTASK: &str = "regenerate with `cargo xtask gen`";
const HERE: &str =
    "regenerate with `SEAM_BLESS=1 cargo test -p seam-example-gain --test page_params`";

#[test]
fn the_pages_parameter_map_matches_the_gains_declarations() {
    let (set, _) = ParamSet::declare::<GainParams>().expect("valid declarations");
    let map = seam_bridge::typescript::param_map(&set, "examples/gain/src/lib.rs (GainParams)");
    assert!(
        map.contains(XTASK),
        "the map's header changed: update this test"
    );
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("page/src/generated/params.ts");
    seam_testkit::drift::check_generated(&path, &map.replace(XTASK, HERE));
}
