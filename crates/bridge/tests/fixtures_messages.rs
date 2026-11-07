//! The messages in `fixtures/messages.json` decode and re-encode unchanged.
//! The page's own test (`page/test/fixture.test.ts`) reads the same file, so
//! the two sides cannot drift apart without one of them failing.

use seam_bridge::{NativeMessage, PageMessage};
use serde_json::Value;

fn fixture() -> Value {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/messages.json");
    let text = std::fs::read_to_string(path).expect("the shared fixture");
    serde_json::from_str(&text).expect("the fixture is JSON")
}

/// The fixture's list, which must hold an entry: a list that went missing or
/// empty would otherwise leave its test looping over nothing.
fn entries<'a>(fixture: &'a Value, list: &str) -> &'a [Value] {
    let entries = fixture[list]
        .as_array()
        .unwrap_or_else(|| panic!("fixture list {list}"));
    assert!(!entries.is_empty(), "fixture list {list} is empty");
    entries
}

#[test]
#[should_panic(expected = "is empty")]
fn an_empty_fixture_list_fails() {
    entries(&serde_json::json!({ "page": [] }), "page");
}

#[test]
fn every_page_message_decodes_and_encodes_back() {
    let fixture = fixture();
    for entry in entries(&fixture, "page") {
        let (name, message) = (&entry["name"], &entry["message"]);
        let decoded = PageMessage::decode(&message.to_string())
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let encoded = serde_json::to_value(&decoded).expect("encodes");
        assert_eq!(&encoded, message, "{name}");
    }
}

#[test]
fn every_native_message_decodes_and_encodes_back() {
    let fixture = fixture();
    for entry in entries(&fixture, "native") {
        let (name, message) = (&entry["name"], &entry["message"]);
        let decoded: NativeMessage = serde_json::from_value(message.clone())
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let encoded: Value = serde_json::from_str(&decoded.encode()).expect("JSON");
        assert_eq!(&encoded, message, "{name}");
    }
}

#[test]
fn what_the_native_side_refuses_it_refuses() {
    let fixture = fixture();
    for entry in entries(&fixture, "refusedByNative") {
        let text = entry["text"].as_str().expect("text");
        assert!(PageMessage::decode(text).is_err(), "{}", entry["name"]);
    }
}

#[test]
fn the_rust_decoder_refuses_the_native_messages_listed() {
    // Native messages deny unknown fields, as page messages do: a build that
    // adds a field must raise the protocol version, not slip past a page.
    // The page's client refuses the same list (`page/test/client.test.ts`).
    let fixture = fixture();
    for entry in entries(&fixture, "refusedAsNative") {
        let text = entry["text"].as_str().expect("text");
        assert!(
            serde_json::from_str::<NativeMessage>(text).is_err(),
            "{}",
            entry["name"]
        );
    }
}

#[test]
fn the_widest_native_numbers_decode_and_encode_back() {
    let fixture = fixture();
    for entry in entries(&fixture, "nativeLimits") {
        let (name, message) = (&entry["name"], &entry["message"]);
        let decoded: NativeMessage = serde_json::from_value(message.clone())
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let encoded: Value = serde_json::from_str(&decoded.encode()).expect("JSON");
        assert_eq!(&encoded, message, "{name}");
    }
}

#[test]
fn what_the_native_side_tolerates_decodes_as_listed() {
    let fixture = fixture();
    for entry in entries(&fixture, "tolerated") {
        let text = entry["text"].as_str().expect("text");
        let decoded =
            PageMessage::decode(text).unwrap_or_else(|e| panic!("{}: {e}", entry["name"]));
        assert_eq!(
            serde_json::to_value(&decoded).expect("encodes"),
            entry["message"],
            "{}",
            entry["name"]
        );
    }
}
