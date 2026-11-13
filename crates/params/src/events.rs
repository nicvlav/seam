//! What edits tell the host, and the gestures undo records.

use crate::{ParamId, ParamRef, RestoreStamp};

/// What an edit tells the host: a user's, the plugin's own or an undo
/// replay's.
///
/// The format exporter drains these through a
/// [`HostEventDrain`](crate::HostEventDrain), in balanced brackets: VST3 on
/// the UI thread, CLAP as output events.
///
/// # Examples
///
/// ```
/// # use seam_params as sp;
/// # let set = sp::ParamSet::from_decls([
/// #     sp::ParamDecl::new("gain", "Gain", sp::Kind::continuous(-60.0, 12.0, 0.1, None), 0.0),
/// # ])?;
/// # let gain = set.id("gain").expect("declared");
/// # let ui = set.take_ui().expect("the first UI handle");
///
/// use seam_params::HostEvent;
///
/// let mut drain = set.take_host_event_drain().expect("the first drain");
/// ui.write(gain, -6.0); // a click on the control
///
/// let mut heard = Vec::new();
/// drain.drain(|event| heard.push(event));
/// assert_eq!(heard[0], HostEvent::Begin(gain));
/// assert!(matches!(heard[1], HostEvent::Value { id, .. } if id == gain));
/// assert_eq!(heard[2], HostEvent::End(gain));
/// # Ok::<(), seam_params::DeclError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HostEvent {
    /// A gesture on the parameter began.
    Begin(ParamId),
    /// The parameter's new value, as the host's 0..1.
    Value {
        /// Which parameter.
        id: ParamId,
        /// The snapped value, normalised.
        normalized: f64,
    },
    /// The gesture ended.
    End(ParamId),
}

/// A user's gesture that closed with the value changed: what undo records as
/// one step.
///
/// Program edits and undo replays produce none: recording a replay would add
/// a phantom step and destroy the redo.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GestureRecord {
    /// Which parameter.
    pub id: ParamId,
    /// The value the gesture showed before the user's first change, so a
    /// program edit earlier in the gesture is not part of the step.
    pub before: f32,
    /// The last value the user wrote, never a re-read of the store, so
    /// automation that landed during the drag is not part of the step.
    pub after: f32,
    /// The restore clock's counts at the close, so undo can drop a step from
    /// before a restore it has not yet seen.
    pub stamp: RestoreStamp,
    /// The close's place in the clock's one sequence, so undo records it in
    /// time order with plain values and lists.
    pub sequence: u64,
}

/// A user gesture on a parameter made at run time, as [`GestureRecord`] is
/// for a listed one: the parameter by its store, which no id names.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct RunTimeRecord {
    /// The parameter.
    pub param: ParamRef,
    /// The value before the gesture's first changing write.
    pub before: f32,
    /// Its last write.
    pub after: f32,
    /// The restore it happened after.
    pub stamp: RestoreStamp,
    /// Its place among every change, listed or made at run time.
    pub sequence: u64,
}
