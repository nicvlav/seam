//! The page bridge: the messages a plugin and its web page exchange, the
//! native side that answers them, and the TypeScript the page is typed with,
//! generated from the Rust types here.
//!
//! The bridge knows nothing of how text travels. A shell that shows the page
//! (a WebView in a plugin window, a standalone app, a mock in a browser) is an
//! [`Outbox`] on the native side and a `Transport` on the page side
//! (`page/src/transport.ts`), and hands the [`Dispatcher`] each message the
//! page sends. This crate builds with no window, WebView or host dependency.
//!
//! Open: the protocol is a first draft, to be settled against the first
//! pages. The [`Dispatcher`] is the native side; the page's half is
//! `page/`. What is fixed:
//!
//! - Messages are JSON objects tagged by `type`. A transport delivers them in
//!   the order they were sent, both ways.
//! - A page subscribes, then sends [`PageMessage::Ready`], again every
//!   second until it is answered. The native side answers every `ready`: with
//!   a [snapshot][glossary-snapshot] ([`NativeMessage::Snapshot`]) when it
//!   speaks the page's protocol version, with a [`NativeMessage::Reject`]
//!   when it does not. Later changes arrive as [`NativeMessage::Param`].
//!   Subscribing first means no change can fall between the seed and the
//!   subscription.
//! - A page [holds][glossary-held] its sets, gestures and calls until the
//!   snapshot arrives, then sends them in the order and within the bound that
//!   [`page/src/held.ts`][held] states, and a `reject` drops them.
//! - Each `ready` starts a page session, which holds the gestures the page
//!   opens. The next `ready`, or the page going away, closes those it left
//!   open, once. Every write follows the page's last `ready`, so a repeated
//!   `ready` never closes a gesture of the page that sent it. A command runs
//!   to its end inside the call that made it, so no call is still running
//!   when a new session starts.
//! - Every [`PageMessage::Set`] and [`PageMessage::GestureEnd`] is answered
//!   with a [`NativeMessage::Param`], changed or not, so the page ends on the
//!   [store][glossary-store]'s value. A write that names a key the plugin
//!   does not list is refused unanswered, and counted ([`Refusals`]).
//! - Parameter values cross as normalised positions (0 to 1) and as the
//!   display text Rust formats. Ranges, skew and snapping stay in Rust, so
//!   the page never repeats that arithmetic.
//! - Decoding never truncates, wraps or clamps a number to fit: it refuses
//!   ([`PageMessage::decode`] lists what), so the page limits a control's
//!   position to 0 to 1 before it sends it.
//! - A message that changes shape changes [`PROTOCOL_VERSION`].
//!
//! The `typescript` feature adds the generator of the page's declarations.
//!
//! # Examples
//!
//! A shell hands the [`Dispatcher`] each message the page sends, and ticks it
//! with the plugin; what it posts reaches the page through the shell's
//! [`Outbox`], here a test's buffer:
//!
//! ```
//! use seam_bridge::{Commands, Dispatcher, NativeMessage, ParamState, Position};
//! use seam_params::{Kind, ParamDecl, ParamSet};
//!
//! let set = ParamSet::from_decls([
//!     ParamDecl::new("mix", "Mix", Kind::continuous(0.0, 100.0, 1.0, None), 100.0)
//!         .with_unit("%"),
//! ])?;
//! let ui = set.take_ui().expect("the first UI handle");
//! let mut native = Dispatcher::new(ui, Commands::new(), Vec::<String>::new());
//!
//! // The page is listening, then sets the mix.
//! native.receive(r#"{"type":"ready","protocol":2}"#);
//! native.receive(r#"{"type":"set","param":"mix","normalized":0.5}"#);
//!
//! // The native side answered the ready with a snapshot, and the set with the
//! // store's value and its text.
//! let posted = std::mem::take(native.outbox_mut());
//! let answer: NativeMessage = serde_json::from_str(&posted[1])?;
//! let normalized = Position::new(0.5).ok_or("in range")?;
//! let state = ParamState { param: "mix".into(), normalized, text: "50 %".into() };
//! assert_eq!(answer, NativeMessage::Param { state });
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! [glossary-snapshot]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#snapshot
//! [glossary-held]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#held
//! [glossary-store]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#store
//! [held]: https://github.com/nicvlav/seam/blob/main/page/src/held.ts

// Shows the feature a gated item needs on docs.rs, which builds with all of
// them and `--cfg docsrs` (`[package.metadata.docs.rs]`).
#![cfg_attr(docsrs, feature(doc_cfg))]

mod dispatcher;
pub mod protocol;
#[cfg(feature = "typescript")]
#[cfg_attr(docsrs, doc(cfg(feature = "typescript")))]
pub mod typescript;

pub use dispatcher::{Commands, Dispatcher, Refusals};
pub use protocol::{CallResult, DecodeError, PROTOCOL_VERSION, ParamState, Position};
#[doc(inline)]
pub use protocol::{NativeMessage, PageMessage};

/// Where the native side posts encoded messages for the page: wry, Tauri, a
/// dev-mode socket, or a buffer in tests.
///
/// # Examples
///
/// ```
/// use seam_bridge::{NativeMessage, Outbox, ParamState, Position};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// // The buffer a test reads back what the page would have received from.
/// let mut page: Vec<String> = Vec::new();
///
/// let changed = NativeMessage::Param {
///     state: ParamState {
///         param: "mix".into(),
///         normalized: Position::new(0.5).ok_or("in range")?,
///         text: "50 %".into(),
///     },
/// };
/// page.post(&changed.encode());
/// assert_eq!(page.len(), 1);
/// # Ok(())
/// # }
/// ```
pub trait Outbox {
    /// Posts one encoded message, which reaches the page after every message
    /// posted before it. The trait's example posts into a buffer.
    ///
    /// Thread: UI.
    fn post(&mut self, message: &str);
}

impl Outbox for Vec<String> {
    fn post(&mut self, message: &str) {
        self.push(message.to_owned());
    }
}
