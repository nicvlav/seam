//! Saved state beyond parameters.
//!
//! A plugin's saved block holds its parameters and one `extra` object. Each
//! owner of other state (MIDI learn's mappings, an editor's layout) is a
//! [participant][glossary-participant] ([`Participant`]) that saves one
//! [section][glossary-section] ([`Section`]) of that object under a stable
//! key. [`Session`] is the store, shared by every thread but audio;
//! [`SessionUi`] is the UI thread's handle, which registers participants and
//! applies [restores][glossary-restore] in a [pass][glossary-pass] on each
//! [tick][glossary-tick] ([`SessionUi::tick`]).
//!
//! - A restore stores every section at once and applies later, on a UI tick,
//!   so a save straight after it returns exactly what was restored. Saving
//!   never runs participant code.
//! - An apply or a [capture][glossary-capture] commits only if no newer
//!   restore landed while the participant ran; that restore is applied next.
//! - A participant whose apply fails or panics keeps its raw section for the
//!   next save, and the others still apply ([`Participant::apply`]).
//! - A capture nested deeper than [`MAX_SECTION_DEPTH`], or too large for the
//!   saved block, is refused ([`Registration::refused`]).
//! - Sections no participant claims are saved back verbatim, and applied to
//!   the next participant that registers the key.
//!
//! The crate has no window, WebView or host dependency.
//!
//! # Examples
//!
//! A participant registers, a save captures it, and a restore applies:
//!
//! ```
//! use std::cell::RefCell;
//! use std::rc::Rc;
//!
//! use seam_params::ParamSetBuilder;
//! use seam_session::{ApplyError, Participant, Section, Session};
//! use serde_json::json;
//!
//! struct Zoom(f64);
//!
//! impl Participant for Zoom {
//!     fn capture(&self) -> Section {
//!         json!(self.0)
//!     }
//!
//!     fn apply(&mut self, section: Option<&Section>) -> Result<(), ApplyError> {
//!         self.0 = section.and_then(Section::as_f64).unwrap_or(1.0);
//!         Ok(())
//!     }
//! }
//!
//! let session = Session::new(&ParamSetBuilder::new().build());
//! let ui = session.take_ui().ok_or("the first UI handle")?;
//! let zoom = Rc::new(RefCell::new(Zoom(2.0)));
//! let registration = ui.register("zoom", &zoom)?;
//! assert_eq!(session.save(), Some(json!({ "zoom": 2.0 })));
//!
//! // The host restores: the section is stored now and applied on the next tick.
//! session.restore(Some(&json!({ "zoom": 3.0 })));
//! ui.tick();
//! assert_eq!(zoom.borrow().0, 3.0);
//!
//! // The participant's state changed on its own: say so, and the next save has it.
//! zoom.borrow_mut().0 = 4.0;
//! registration.changed();
//! assert_eq!(session.save(), Some(json!({ "zoom": 4.0 })));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! [glossary-participant]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#participant
//! [glossary-section]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#section
//! [glossary-restore]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#restore
//! [glossary-pass]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#pass
//! [glossary-tick]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#tick
//! [glossary-capture]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#capture

mod session;
mod ui;

pub use session::{MAX_SECTION_DEPTH, MAX_STATE_BYTES, Restored, Session};
pub use ui::{Registration, SessionUi};

/// One participant's saved state: a JSON value, or `Null` for nothing to
/// save, which leaves the key out of the saved block.
///
/// # Examples
///
/// ```
/// use seam_session::Section;
/// use serde_json::json;
///
/// let layout: Section = json!({ "zoom": 2.0, "panel": "mixer" });
/// let nothing = Section::Null;
/// assert!(layout.is_object() && nothing.is_null());
/// ```
pub type Section = serde_json::Value;

/// Something that owns saved state other than parameters.
///
/// Both methods run on the UI thread, never while the session holds a lock.
/// A participant is registered as an `Rc<RefCell<_>>` its owner keeps; the
/// session holds it weakly and never owns it.
///
/// # Examples
///
/// ```
/// use std::cell::RefCell;
/// use std::rc::Rc;
///
/// use seam_params::ParamSetBuilder;
/// use seam_session::{ApplyError, Participant, Section, Session};
/// use serde_json::json;
///
/// /// An editor's panel width, saved with the plugin.
/// struct Panel {
///     width: u32,
/// }
///
/// impl Participant for Panel {
///     fn capture(&self) -> Section {
///         json!({ "width": self.width })
///     }
///
///     fn apply(&mut self, section: Option<&Section>) -> Result<(), ApplyError> {
///         // No section: a restore that predates this state, so use the default.
///         let Some(section) = section else {
///             self.width = 320;
///             return Ok(());
///         };
///         let width = section["width"]
///             .as_u64()
///             .and_then(|width| u32::try_from(width).ok())
///             .ok_or_else(|| ApplyError::new("width is not a number"))?;
///         self.width = width;
///         Ok(())
///     }
/// }
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let session = Session::new(&ParamSetBuilder::new().build());
/// let ui = session.take_ui().ok_or("the first UI handle")?;
/// let panel = Rc::new(RefCell::new(Panel { width: 480 }));
/// let _registration = ui.register("panel", &panel)?;
///
/// session.restore(Some(&json!({ "panel": { "width": 640 } })));
/// ui.tick();
/// assert_eq!(panel.borrow().width, 640);
///
/// // A section it cannot apply leaves it as it was, and is saved back.
/// let newer = json!({ "panel": { "width": "wide" } });
/// session.restore(Some(&newer));
/// ui.tick();
/// assert_eq!(panel.borrow().width, 640);
/// assert_eq!(session.save(), Some(newer));
/// # Ok(())
/// # }
/// ```
pub trait Participant {
    /// Returns the participant's state now, as the section a save writes.
    ///
    /// Must not change anything. Called at registration, for each
    /// [`Registration::changed`] that takes effect, and after every apply that
    /// succeeded while the participant is still registered; never by a save.
    /// The trait's example implements it.
    fn capture(&self) -> Section;

    /// Replaces the participant's state with `section`, or with its defaults
    /// when the restore had no section for it, as the trait's example shows.
    ///
    /// A JSON `null` section arrives as `None` too, never as `Some(Null)`. It
    /// runs on a UI tick after the restore was stored, or inside
    /// [`SessionUi::register`] for a participant registering after it; never
    /// inside the host's restore call.
    ///
    /// A panic is contained like an error when the plugin is built with
    /// `panic = "unwind"`; with `abort` it ends the process. A panic can stop
    /// an apply halfway, so the session ignores the participant's captures,
    /// and keeps its raw section in every save, until a later apply succeeds.
    ///
    /// # Errors
    ///
    /// Return an error for any section this participant cannot apply, data
    /// that does not decode included. The participant keeps the state it had,
    /// its raw section is saved back unchanged so a newer build's data
    /// survives, and the other participants still apply.
    fn apply(&mut self, section: Option<&Section>) -> Result<(), ApplyError>;
}

/// Why a participant could not apply its section.
///
/// # Examples
///
/// ```
/// use seam_session::ApplyError;
///
/// let error = ApplyError::new("zoom is not a number");
/// assert_eq!(error.message, "zoom is not a number");
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub struct ApplyError {
    /// What went wrong, for a developer.
    pub message: String,
}

impl ApplyError {
    /// Creates an error saying what went wrong, for a developer, as the
    /// [`ApplyError`] example shows.
    pub fn new(message: impl Into<String>) -> Self {
        ApplyError {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "could not apply a session section: {}", self.message)
    }
}

impl std::error::Error for ApplyError {}

/// Why a registration was refused.
///
/// # Examples
///
/// ```
#[doc = include_str!("../doc/setup.rs")]
/// use seam_session::SessionError;
///
/// let session = Session::new(&ParamSetBuilder::new().build());
/// let ui = session.take_ui().ok_or("the first UI handle")?;
/// let first = Rc::new(RefCell::new(Zoom(1.0)));
/// let second = Rc::new(RefCell::new(Zoom(2.0)));
///
/// let _registration = ui.register("zoom", &first)?;
/// assert_eq!(
///     ui.register("zoom", &second).err(),
///     Some(SessionError::KeyTaken("zoom".to_owned())),
/// );
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SessionError {
    /// Keys must not be empty.
    EmptyKey,
    /// Another registration holds this key, even one whose participant is
    /// gone.
    KeyTaken(String),
    /// The participant is already registered, under another key.
    AlreadyRegistered,
    /// The participant's own code holds it right now: register it from
    /// outside its methods.
    Busy,
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::EmptyKey => write!(f, "a session key must not be empty"),
            SessionError::KeyTaken(key) => write!(f, "session key {key:?} is already registered"),
            SessionError::AlreadyRegistered => write!(f, "the participant is already registered"),
            SessionError::Busy => write!(f, "the participant is in use"),
        }
    }
}

impl std::error::Error for SessionError {}
