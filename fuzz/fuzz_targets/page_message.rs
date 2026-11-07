//! The text the page sends to the native side. Decoding answers a message
//! or an error, never panics, and a decoded message encodes to text that
//! decodes to the same message.
#![no_main]

use libfuzzer_sys::fuzz_target;
use seam_bridge::PageMessage;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data)
        && let Ok(message) = PageMessage::decode(text)
    {
        let encoded = serde_json::to_string(&message).expect("a page message encodes");
        assert_eq!(
            PageMessage::decode(&encoded).ok().as_ref(),
            Some(&message),
            "{encoded} does not decode to what it encoded"
        );
    }
});
