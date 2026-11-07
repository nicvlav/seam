//! MIDI learn: a user arms a target, moves a controller, and from then on
//! that controller drives the target.
//!
//! A plugin makes both halves with [`Learn::new`]:
//!
//! - [`Learn`], on the UI thread, owns the one authoritative table of
//!   [`Mapping`]s, at most one per [target][glossary-target], and publishes a
//!   new [snapshot][glossary-snapshot] for the audio thread on every mapping
//!   change. [`Learn::save`] and [`Learn::restore`] read and write the
//!   session's `midiLearn` section.
//! - [`LearnAudio`], on the audio thread, parses each block's Universal MIDI
//!   Packets (MIDI 1.0 and 2.0), looks them up in the snapshot, and writes
//!   each mapped parameter as a [learned write][glossary-learned-write].
//! - [`Learn::poll`], once per UI [tick][glossary-tick], commits a captured
//!   controller and writes every other [`Target`] the plugin registers, one
//!   gesture per [burst][glossary-burst].
//!
//! To learn, [arm][glossary-arm] a target ([`Learn::arm`]): the audio thread
//! [captures][glossary-capture] the newest legal message, and the next poll
//! commits it, [coarse or fine][glossary-coarse]. [`Learn::poll`] states when
//! half of a MIDI 1.0 14-bit pair settles and which half is learned, and
//! [`Control::Cc`] why a learned pair plays at 7 bits. A
//! [reservation][glossary-reservation] ([`CcReservation`],
//! [`NrpnReservation`]) keeps a plugin's own numbers out of capture.
//!
//! A [`Source`] is a [`Control`] plus the UMP group and channel it must
//! arrive on, each with an "any". A [`Curve`] shapes the value.
//!
//! Invariants:
//!
//! - The audio side never waits, locks, allocates or frees. The snapshot is a
//!   `triple_buffer`, and a replaced table drops on the UI thread.
//! - A message staged for one arm never commits to the next.
//! - This crate builds with no window, WebView or host dependency, and
//!   writes no `unsafe`.
//!
//! # Examples
//!
//! Wire learn to a session, give each activation its own audio side, and
//! drive both halves:
//!
//! ```
//! use std::cell::RefCell;
//! use std::rc::Rc;
//!
//! use seam_learn::{Config, Control, Learn, Mapping, Source};
//! use seam_params::{Kind, ParamDecl, ParamSet};
//! use seam_session::Session;
//! use seam_ump::{ChannelVoice, EventList, Voice};
//!
//! let decl = ParamDecl::new("gain", "Gain", Kind::continuous(0.0, 1.0, 0.0, None), 0.0);
//! let set = ParamSet::from_decls([decl])?;
//! let gain = set.id("gain").ok_or("declared above")?;
//! let session = Session::new(&set);
//! let ui = session.take_ui().ok_or("the first UI handle")?;
//!
//! // Once, on the UI thread: the table, and the session that saves it.
//! let (learn, first_audio) = Learn::new(&set, Config::default());
//! drop(first_audio);
//! let learn = Rc::new(RefCell::new(learn));
//! Learn::attach(&learn, &ui)?;
//! learn.borrow_mut().map(gain, Mapping::new(Source::any(Control::Cc(7))))?;
//!
//! // On each activation: a fresh audio side that starts from the mappings so far.
//! let mut audio = learn.borrow_mut().replace_audio();
//!
//! // In each block, on the audio thread: controller 7 at full writes the gain.
//! let voice = Voice::Midi1ControlChange { controller: 7, value: 127 };
//! let mut midi = EventList::with_capacity(1);
//! midi.push(ChannelVoice { group: 0, channel: 0, voice }.event(0));
//! audio.process(&midi);
//! assert_eq!(set.value(gain), 1.0);
//!
//! // On each UI tick: commit a captured controller, write the other targets.
//! assert!(learn.borrow_mut().poll().is_none()); // nothing was armed
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! [glossary-target]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#target
//! [glossary-snapshot]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#snapshot
//! [glossary-learned-write]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#learned-write
//! [glossary-tick]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#tick
//! [glossary-burst]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#burst
//! [glossary-arm]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#arm
//! [glossary-capture]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#capture
//! [glossary-coarse]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#coarse-fine
//! [glossary-reservation]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#reservation

// The nightly real-time sanitizer lane marks the calls the audio thread makes.
#![cfg_attr(seam_rtsan, feature(sanitize))]

mod audio;
mod capture;
mod curve;
mod field;
mod mappings;
mod parse;
mod reservation;
mod saved;
mod scale;
mod snapshot;
mod source;
mod table;
mod target;

pub use audio::LearnAudio;
pub use capture::{Learned, Takes};
pub use curve::{Curve, CurveError};
pub use mappings::{Config, DEFAULT_MAX_MAPPINGS, Learn, LearnError, Mapping};
pub use reservation::{CcReservation, NrpnReservation, ReservationError};
pub use saved::SESSION_KEY;
pub use source::{Control, Source};
pub use target::{CustomId, Target, TargetId};
