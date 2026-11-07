//! The text the page sends to the native side. Decoding answers a message
//! or an error, never panics, and a decoded position always lies in 0 to 1.
#![no_main]

use libfuzzer_sys::fuzz_target;
use seam_bridge::PageMessage;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data)
        && let Ok(PageMessage::Set { normalized, .. }) = PageMessage::decode(text)
    {
        let normalized = normalized.get();
        assert!((0.0..=1.0).contains(&normalized), "decoded {normalized}");
    }
});
