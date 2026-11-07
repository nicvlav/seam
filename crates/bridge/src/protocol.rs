//! The messages between the native side and the page.

use serde::de::{self, Unexpected};
use serde::{Deserialize, Deserializer, Serialize};
#[cfg(feature = "typescript")]
use ts_rs::TS;

/// The protocol's version. It changes whenever a message changes shape, and
/// whenever text that decoded before decodes otherwise or is refused. The page
/// sends the version it was built against in [`PageMessage::Ready`].
///
/// A rejected page drops its writes and rejects its calls from then on. The
/// [crate overview](crate) describes the handshake. A test fails when the
/// message shapes change and this does not (`tests/protocol_version.rs`); a
/// change to what decodes has no such test, so it is raised by hand.
pub const PROTOCOL_VERSION: u32 = 2;

/// A message from the page.
///
/// Decode with [`PageMessage::decode`], which lets a `ready` carry unknown
/// fields. Deserializing the type directly refuses them there too.
///
/// # Examples
///
/// ```
/// use seam_bridge::{PageMessage, Position};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let set = PageMessage::decode(r#"{"type":"set","param":"mix","normalized":0.75}"#)?;
/// let normalized = Position::new(0.75).ok_or("in range")?;
/// assert_eq!(set, PageMessage::Set { param: "mix".into(), normalized });
///
/// // A field this version does not know is refused, so a changed message
/// // cannot slip past a native side of the same version.
/// assert!(PageMessage::decode(r#"{"type":"gestureEnd","param":"mix","x":1}"#).is_err());
///
/// // Except in a `ready`, so a newer page can always say which version it
/// // speaks, and be told this side's.
/// let ready = PageMessage::decode(r#"{"type":"ready","protocol":3,"features":[]}"#)?;
/// assert_eq!(ready, PageMessage::Ready { protocol: 3 });
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(TS))]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum PageMessage {
    /// The page's listeners are in place.
    ///
    /// The native side answers with a [`NativeMessage::Snapshot`] when
    /// `protocol` is its own version and with a [`NativeMessage::Reject`] when
    /// it is not. A page may send it again until it is answered.
    Ready {
        /// The [`PROTOCOL_VERSION`] the page was generated against.
        protocol: u32,
    },
    /// A control starts changing a parameter (a pointer goes down). Several
    /// open gestures on one parameter reach the host as one.
    GestureBegin {
        /// The parameter's key (`ParamDecl::key`).
        param: String,
    },
    /// Sets a parameter, inside a gesture or as a one-shot edit. The native
    /// side answers with a [`NativeMessage::Param`], changed or not, so the
    /// page learns the snapped value of its own write.
    Set {
        /// The parameter's key (`ParamDecl::key`).
        param: String,
        /// The position, 0 to 1. Rust snaps it to the parameter's range.
        /// Decoding refuses a position outside 0 to 1.
        #[cfg_attr(feature = "typescript", ts(type = "number"))]
        normalized: Position,
    },
    /// The control is done (pointer up or cancelled). The native side answers
    /// with a [`NativeMessage::Param`] carrying the store after the close,
    /// which may hold a host write or a restore that landed during the drag.
    GestureEnd {
        /// The parameter's key (`ParamDecl::key`).
        param: String,
    },
    /// Calls a command; the native side answers with a
    /// [`NativeMessage::Reply`] carrying the same `call`.
    Invoke {
        /// Chosen by the page, unique among its calls still waiting.
        call: u32,
        /// The command's name.
        command: String,
        /// The command's arguments. Absent decodes as null: JSON drops an
        /// `undefined` field.
        #[serde(default)]
        #[cfg_attr(feature = "typescript", ts(type = "unknown"))]
        args: serde_json::Value,
    },
}

/// A message to the page.
///
/// # Examples
///
/// ```
/// use seam_bridge::{NativeMessage, ParamState, Position};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let message = NativeMessage::Param {
///     state: ParamState {
///         param: "gain".into(),
///         normalized: Position::new(0.5).ok_or("in range")?,
///         text: "0.0 dB".into(),
///     },
/// };
/// assert_eq!(
///     message.encode(),
///     r#"{"type":"param","state":{"param":"gain","normalized":0.5,"text":"0.0 dB"}}"#,
/// );
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(TS))]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum NativeMessage {
    /// The answer to a [`PageMessage::Ready`] of the native side's own
    /// version: every parameter as it is now.
    Snapshot {
        /// The native side's [`PROTOCOL_VERSION`].
        protocol: u32,
        /// Every parameter, in declaration order.
        params: Vec<ParamState>,
    },
    /// The answer to a [`PageMessage::Ready`] of a version the native side
    /// cannot serve. The page fails: it stops sending and rejects its calls.
    Reject {
        /// The native side's [`PROTOCOL_VERSION`].
        protocol: u32,
        /// Why, for a developer.
        message: String,
    },
    /// A parameter's state, after a change or in answer to a write.
    ///
    /// The native side sends one after a change by any origin (the host, undo,
    /// a learned controller, another control on the page), and in answer to
    /// every [`PageMessage::Set`] and [`PageMessage::GestureEnd`], changed or
    /// not.
    Param {
        /// Its new state.
        state: ParamState,
    },
    /// The answer to an [`PageMessage::Invoke`].
    Reply {
        /// The `call` of the invoke this answers.
        call: u32,
        /// What the command returned.
        result: CallResult,
    },
    /// Something happened that has no current value to hold (a learn
    /// captured a controller, a preset finished loading).
    Event {
        /// The event's name.
        name: String,
        /// The event's data.
        #[cfg_attr(feature = "typescript", ts(type = "unknown"))]
        payload: serde_json::Value,
    },
}

/// A parameter as the page shows it.
///
/// # Examples
///
/// ```
/// use seam_bridge::{ParamState, Position};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// // The page gets the position and the text Rust formatted, never the range.
/// let state = ParamState {
///     param: "gain".into(),
///     normalized: Position::new(0.75).ok_or("in range")?,
///     text: "6.0 dB".into(),
/// };
/// assert_eq!(state.text, "6.0 dB");
///
/// // A position is 0 to 1 by its type, so a NaN cannot reach the page.
/// assert!(Position::new(f64::NAN).is_none());
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParamState {
    /// The parameter's key (`ParamDecl::key`).
    pub param: String,
    /// Its position, 0 to 1.
    #[cfg_attr(feature = "typescript", ts(type = "number"))]
    pub normalized: Position,
    /// Its value as text, in its unit ("-6.0 dB").
    pub text: String,
}

/// What a command returned.
///
/// # Examples
///
/// ```
/// use seam_bridge::{CallResult, NativeMessage};
///
/// // The answer to the page's call 7, a command that returns nothing.
/// let reply = NativeMessage::Reply {
///     call: 7,
///     result: CallResult::Ok { value: serde_json::Value::Null },
/// };
/// assert_eq!(
///     reply.encode(),
///     r#"{"type":"reply","call":7,"result":{"status":"ok","value":null}}"#,
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(TS))]
#[serde(tag = "status", rename_all = "camelCase", deny_unknown_fields)]
pub enum CallResult {
    /// The command ran.
    Ok {
        /// What it returned; `null` for a command that returns nothing, since
        /// the field is always written.
        #[cfg_attr(feature = "typescript", ts(type = "unknown"))]
        value: serde_json::Value,
    },
    /// The command was unknown, its arguments were wrong, or it failed.
    Err {
        /// Why, for a developer.
        message: String,
    },
}

/// A message that could not be decoded. Its text says why, so it reports no
/// separate [`source`](std::error::Error::source).
///
/// # Examples
///
/// ```
/// use seam_bridge::PageMessage;
///
/// let error = PageMessage::decode(r#"{"type":"teleport"}"#).err();
/// assert!(error.is_some_and(|error| error.to_string().starts_with("malformed bridge message")));
/// ```
#[derive(Debug)]
pub struct DecodeError(serde_json::Error);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "malformed bridge message: {}", self.0)
    }
}

impl std::error::Error for DecodeError {}

/// A parameter's position from 0 to 1, as a page sets it and the native side
/// reports it.
///
/// Decoding refuses one outside 0 to 1 rather than clamp it: it never
/// changes a number to make it fit, and the page limits its positions before
/// it sends them. So a `Position` in hand is always in range. On the wire, and
/// in the generated TypeScript, it is a plain number.
///
/// # Examples
///
/// ```
/// use seam_bridge::{PageMessage, Position};
///
/// assert_eq!(Position::new(0.25).map(Position::get), Some(0.25));
/// assert_eq!(Position::new(1.5), None);
/// assert_eq!(Position::new(f64::NAN), None);
///
/// let text = r#"{"type":"set","param":"mix","normalized":1.5}"#;
/// assert!(PageMessage::decode(text).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Position(f64);

impl Position {
    /// `value` as a position, or `None` when it lies outside 0 to 1 or is
    /// NaN.
    pub fn new(value: f64) -> Option<Position> {
        (0.0..=1.0).contains(&value).then_some(Position(value))
    }

    /// The position, from 0 to 1.
    pub fn get(self) -> f64 {
        self.0
    }
}

impl<'de> Deserialize<'de> for Position {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = f64::deserialize(deserializer)?;
        Position::new(value).ok_or_else(|| {
            de::Error::invalid_value(Unexpected::Float(value), &"a position from 0 to 1")
        })
    }
}

impl PageMessage {
    /// Decodes a message the page sent.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError`] for text that is not JSON or names an unknown
    /// `type`. The same error covers an unknown, missing or repeated field, a
    /// number of the wrong type or range, and a position outside 0 to 1. A
    /// `ready` may carry unknown fields, so a newer page can always say which
    /// version it speaks.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_bridge::{PageMessage, Position};
    ///
    /// let message = PageMessage::decode(r#"{"type":"set","param":"mix","normalized":0.25}"#)?;
    /// let normalized = Position::new(0.25).ok_or("in range")?;
    /// assert_eq!(message, PageMessage::Set { param: "mix".into(), normalized });
    ///
    /// // A fractional call id is rejected, not truncated.
    /// let text = r#"{"type":"invoke","call":3.7,"command":"undo"}"#;
    /// assert!(PageMessage::decode(text).is_err());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn decode(text: &str) -> Result<Self, DecodeError> {
        /// What a `ready` must hold; whatever else it carries is ignored. A
        /// field named twice is still refused.
        #[derive(Deserialize)]
        #[serde(tag = "type")]
        enum Lenient {
            #[serde(rename = "ready")]
            Ready { protocol: u32 },
            #[serde(other)]
            Other,
        }
        let strict = match serde_json::from_str(text) {
            Ok(message) => return Ok(message),
            Err(error) => error,
        };
        match serde_json::from_str(text) {
            Ok(Lenient::Ready { protocol }) => Ok(PageMessage::Ready { protocol }),
            Ok(Lenient::Other) => Err(DecodeError(strict)),
            Err(error) => Err(DecodeError(error)),
        }
    }
}

impl NativeMessage {
    /// Encodes a message as the JSON text the page receives, which never
    /// fails; the [`NativeMessage`] example shows the wire form.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_bridge::NativeMessage;
    ///
    /// // The text is one JSON object tagged by `type`.
    /// let reject = NativeMessage::Reject { protocol: 2, message: "too old".into() };
    /// assert_eq!(
    ///     reject.encode(),
    ///     r#"{"type":"reject","protocol":2,"message":"too old"}"#,
    /// );
    /// ```
    #[expect(
        clippy::missing_panics_doc,
        reason = "serde_json cannot fail on these types: struct variants of strings, integers, \
                  positions from 0 to 1, lists and JSON values, whose maps have string keys \
                  and whose numbers are finite"
    )]
    pub fn encode(&self) -> String {
        serde_json::to_string(self).expect("bridge messages always encode")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_messages_are_tagged_by_type() {
        let message =
            PageMessage::decode(r#"{"type":"set","param":"mix","normalized":0.25}"#).unwrap();
        assert_eq!(
            message,
            PageMessage::Set {
                param: "mix".into(),
                normalized: Position::new(0.25).expect("in range")
            }
        );
    }

    #[test]
    fn a_fractional_or_out_of_range_integer_is_rejected_not_truncated() {
        for call in ["3.7", "-1", "4294967296"] {
            let text = format!(r#"{{"type":"invoke","call":{call},"command":"undo","args":null}}"#);
            assert!(PageMessage::decode(&text).is_err(), "call {call} decoded");
        }
    }

    #[test]
    fn a_position_outside_0_to_1_is_refused_not_clamped() {
        let set = |normalized: &str| {
            PageMessage::decode(&format!(
                r#"{{"type":"set","param":"mix","normalized":{normalized}}}"#
            ))
        };
        for normalized in ["-0.25", "-1e-9", "1.0000001", "2.5", "1e300"] {
            assert!(set(normalized).is_err(), "{normalized} decoded");
        }
        for (normalized, value) in [("0", 0.0), ("-0", 0.0), ("0.5", 0.5), ("1", 1.0)] {
            assert_eq!(
                set(normalized).unwrap(),
                PageMessage::Set {
                    param: "mix".into(),
                    normalized: Position::new(value).expect("in range")
                },
                "{normalized}"
            );
        }
        // Read directly, the type refuses it too.
        let text = r#"{"type":"set","param":"mix","normalized":2.5}"#;
        assert!(serde_json::from_str::<PageMessage>(text).is_err());
    }

    #[test]
    fn an_invoke_without_args_decodes_with_null_args() {
        let message =
            PageMessage::decode(r#"{"type":"invoke","call":1,"command":"undo"}"#).unwrap();
        assert_eq!(
            message,
            PageMessage::Invoke {
                call: 1,
                command: "undo".into(),
                args: serde_json::Value::Null
            }
        );
    }

    #[test]
    fn unknown_fields_and_types_are_rejected() {
        assert!(PageMessage::decode(r#"{"type":"set","param":"a","normalized":0,"x":0}"#).is_err());
        assert!(PageMessage::decode(r#"{"type":"teleport"}"#).is_err());
    }

    #[test]
    fn a_field_named_twice_is_refused_not_resolved_to_the_last() {
        for text in [
            r#"{"type":"set","param":"a","param":"b","normalized":0.5}"#,
            r#"{"type":"set","param":"a","normalized":0.5,"normalized":0.25}"#,
            r#"{"type":"gestureEnd","param":"a","type":"ready","protocol":2}"#,
            r#"{"type":"ready","protocol":2,"type":"ready"}"#,
            r#"{"type":"ready","protocol":2,"protocol":3}"#,
            r#"{"type":"ready","protocol":2,"extra":1,"protocol":3}"#,
        ] {
            assert!(PageMessage::decode(text).is_err(), "{text}");
        }
        // A repeated unknown field of a ready is ignored like any other.
        let text = r#"{"type":"ready","protocol":2,"extra":1,"extra":2}"#;
        assert_eq!(
            PageMessage::decode(text).unwrap(),
            PageMessage::Ready { protocol: 2 }
        );
    }

    #[test]
    fn a_ready_of_a_newer_page_decodes_whatever_else_it_says() {
        let text = r#"{"type":"ready","protocol":99,"features":[],"nested":{"a":[1]}}"#;
        assert_eq!(
            PageMessage::decode(text).unwrap(),
            PageMessage::Ready { protocol: 99 }
        );
        // The version itself must still be a version.
        for text in [
            r#"{"type":"ready","features":[]}"#,
            r#"{"type":"ready","protocol":-1}"#,
            r#"{"type":"ready","protocol":1.5}"#,
            r#"{"type":"ready","protocol":4294967296}"#,
            r#"{"type":"ready","protocol":"2"}"#,
        ] {
            assert!(PageMessage::decode(text).is_err(), "{text}");
        }
    }

    #[test]
    fn a_decode_error_shows_its_cause_once() {
        use std::error::Error as _;
        let error = PageMessage::decode(r#"{"type":"teleport"}"#).unwrap_err();
        assert!(error.to_string().contains("teleport"), "{error}");
        assert!(error.source().is_none(), "the cause shows in a chain twice");
    }

    #[test]
    fn native_messages_round_trip() {
        let message = NativeMessage::Reply {
            call: 7,
            result: CallResult::Err {
                message: "no such command".into(),
            },
        };
        let text = message.encode();
        assert_eq!(
            serde_json::from_str::<NativeMessage>(&text).unwrap(),
            message
        );
    }
}
