//! Builds the gain's page in when `page/dist` holds it, by setting the
//! `seam_page` cfg `src/page.rs` reads, and warns when it does not: the
//! plugin then offers no editor. Cargo never runs Node.

use std::path::Path;

/// The files `npm run build` in `page/` writes, under fixed names.
const BUILT: [&str; 3] = [
    "page/dist/index.html",
    "page/dist/assets/index.js",
    "page/dist/assets/index.css",
];

fn main() {
    println!("cargo::rustc-check-cfg=cfg(seam_page)");
    for file in BUILT {
        println!("cargo::rerun-if-changed={file}");
    }
    if BUILT.iter().all(|file| Path::new(file).is_file()) {
        println!("cargo::rustc-cfg=seam_page");
    } else {
        println!(
            "cargo::warning=the gain's page is not built, so the plugin has no editor; \
             build it with `npm ci` and `npm run build` in examples/gain/page"
        );
    }
}
