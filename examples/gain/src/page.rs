//! The gain's page: a slider on the gain and a field for the label, in the
//! host's window. Its source is `page/`, a Vite and React app; `npm run
//! build` there writes the files below, which the plugin is built with. A
//! build that finds them missing says so, and the plugin offers no editor.

use seam_host::{Editor, Plugin, Size};
use seam_webview::{Asset, Page};

use crate::Gain;

/// The page's built files, when they were there to build in.
#[cfg(seam_page)]
pub const PAGE: Option<&[Asset]> = Some(&[
    Asset {
        path: "index.html",
        bytes: include_bytes!("../page/dist/index.html"),
    },
    Asset {
        path: "assets/index.js",
        bytes: include_bytes!("../page/dist/assets/index.js"),
    },
    Asset {
        path: "assets/index.css",
        bytes: include_bytes!("../page/dist/assets/index.css"),
    },
]);

/// The page's built files, when they were there to build in.
#[cfg(not(seam_page))]
pub const PAGE: Option<&[Asset]> = None;

/// The editor that shows the page: none when the page was not built in.
/// The label's two commands go through the same value the plugin saves, so
/// a rename from the page is one undo step and saved with the plugin.
pub(crate) fn editor(gain: &Gain) -> Option<Box<dyn Editor>> {
    let (read, write) = (gain.label.clone(), gain.label.clone());
    Page::new(&Gain::INFO, gain.ui().clone(), PAGE?)
        .with_size(Size {
            width: 420,
            height: 220,
        })
        .command("getLabel", move |()| Ok(read.get()))
        .command("setLabel", move |label: String| {
            write.set(label);
            Ok(())
        })
        .into_editor()
}
