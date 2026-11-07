//! The drift gate for the page's generated protocol module.

use std::path::Path;

#[test]
fn the_committed_protocol_module_matches_the_rust_types() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../page/src/generated/protocol.ts");
    seam_testkit::drift::check_generated(&path, &seam_bridge::typescript::protocol());
}
