//! What a mapping drives: a parameter, written on the audio thread,
//! or a target learn owns, written on the UI thread one burst at a time.

// Audio-path code: a panic here ends a host, so indexing, unwrap, expect,
// panic and unchecked arithmetic are refused, and a site that is provably
// safe says why in an `#[expect]`.
#![cfg_attr(
    not(test),
    deny(
        clippy::indexing_slicing,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::arithmetic_side_effects
    )
)]

#[cfg(loom)]
use loom::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(not(loom))]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use seam_params::ParamId;

/// What a mapping drives.
///
/// The tier follows from the kind, decided once, when the target exists.
/// A parameter is host-projected, so a learned value reaches its store
/// on the audio thread in the block it arrived in. A [`Custom`](Self::Custom)
/// target is registered with [`Learn::register`](crate::Learn::register)
/// and written on the UI thread.
///
/// # Examples
///
/// ```
/// use seam_learn::{Config, Learn, TargetId};
#[doc = include_str!("../doc/setup.rs")]
/// // A parameter and a registered target are both targets, by `From`.
/// let (mut learn, _audio) = Learn::new(&set, Config::default());
/// let fader = learn.register("fader", Box::new(Fader))?;
/// let param: TargetId = gain.into();
/// let custom: TargetId = fader.into();
/// assert_ne!(param, custom);
/// assert_eq!(learn.key(param), Some("gain"));
/// assert_eq!(learn.key(custom), Some("fader"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TargetId {
    /// A parameter of the set learn was made for.
    Param(ParamId),
    /// A target registered with learn.
    Custom(CustomId),
}

impl From<ParamId> for TargetId {
    fn from(id: ParamId) -> Self {
        TargetId::Param(id)
    }
}

impl From<CustomId> for TargetId {
    fn from(id: CustomId) -> Self {
        TargetId::Custom(id)
    }
}

/// A target registered with [`Learn::register`](crate::Learn::register).
/// The id names that registration only: a later registration under the same
/// key gets a new one.
///
/// # Examples
///
/// ```
/// use seam_learn::{Config, Learn};
#[doc = include_str!("../doc/setup.rs")]
/// let (mut learn, _audio) = Learn::new(&set, Config::default());
/// let first = learn.register("fader", Box::new(Fader))?;
///
/// // Detaching and registering the key again gives a new id.
/// learn.detach(first);
/// let second = learn.register("fader", Box::new(Fader))?;
/// assert_ne!(first, second);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CustomId(pub(crate) u64);

/// A target learn writes on the UI thread: plugin state that is not a
/// parameter, a plain value say. Learn owns it once registered.
///
/// A burst of learned values is one gesture, so a sweep is one undo step.
/// [`begin`](Self::begin) opens it at the first value. [`apply`](Self::apply)
/// takes the newest value each poll tick that has one. [`end`](Self::end)
/// closes it once 15 ticks pass with no value, or when the target is
/// unmapped or detached, or learn drops.
///
/// Learn calls a target only while it holds itself mutably, inside `poll` or
/// a mapping change. A target that reaches the [`Learn`](crate::Learn) that
/// owns it cannot change its mappings from inside the call: the borrow
/// refuses.
///
/// # Examples
///
/// How many meter bars show, following a controller one burst at a time:
///
/// ```
/// use std::cell::RefCell;
/// use std::rc::Rc;
///
/// use seam_learn::{Config, Control, Learn, Mapping, Source, Target};
#[doc = include_str!("../doc/setup.rs")]
/// // Records each call learn makes.
/// struct Bars {
///     calls: Rc<RefCell<Vec<String>>>,
/// }
///
/// impl Target for Bars {
///     fn begin(&mut self) {
///         self.calls.borrow_mut().push("begin".into());
///     }
///
///     fn apply(&mut self, normalized: f64) {
///         let shown = (normalized * 16.0).round();
///         self.calls.borrow_mut().push(format!("show {shown}"));
///     }
///
///     fn end(&mut self) {
///         self.calls.borrow_mut().push("end".into());
///     }
/// }
///
/// let (mut learn, mut audio) = Learn::new(&set, Config::default());
/// let calls = Rc::new(RefCell::new(Vec::new()));
/// let bars = learn.register("bars", Box::new(Bars { calls: calls.clone() }))?;
/// learn.map(bars, Mapping::new(Source::any(Control::Cc(7))))?;
///
/// // A block moves controller 7 to 64, then to 127. The poll opens a burst
/// // and applies the newest value.
/// audio.process(&block(&[(7, 64), (7, 127)]));
/// learn.poll();
/// assert_eq!(*calls.borrow(), ["begin", "show 16"]);
///
/// // Fifteen ticks with no value close the burst.
/// for _ in 0..15 {
///     learn.poll();
/// }
/// assert_eq!(*calls.borrow(), ["begin", "show 16", "end"]);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub trait Target {
    /// Starts a burst: open one gesture, `Value::begin` say. The [`Target`]
    /// example shows the calls in order.
    fn begin(&mut self);

    /// Takes the newest learned value, 0 to 1 after the mapping's curve.
    fn apply(&mut self, normalized: f64);

    /// Ends the burst: close the gesture.
    fn end(&mut self);

    /// Returns whether the user holds this target's control now.
    ///
    /// While they do, learn closes its own burst and leaves the newest value
    /// waiting, as a parameter's surfaces hold back host writes; it applies
    /// once they let go. The answer must not count learn's own burst. The
    /// default is `false`.
    ///
    /// Open: how a plain `Value` serves as a target is not decided. It
    /// reports only whether some gesture is open, so it cannot tell learn's
    /// burst from the user's drag and cannot implement this correctly yet.
    fn held(&self) -> bool {
        false
    }
}

/// How many poll ticks with no new value close a burst.
pub(crate) const BURST_IDLE_TICKS: u32 = 15;

/// The newest value the audio thread learned for one target, waiting for
/// the UI poll. The table owns it ([`Route`](crate::table::Route)).
#[derive(Debug, Default)]
pub(crate) struct Staging {
    bits: AtomicU64,
    pending: AtomicBool,
}

impl Staging {
    /// Stages the newest value, latest wins. Wait-free.
    ///
    /// Thread: audio.
    pub(crate) fn stage(&self, value: f64) {
        self.bits.store(value.to_bits(), Ordering::Relaxed);
        // A read-modify-write, not a store, as the take's swap is. Under the
        // memory model a store would be correct too. The swap is for loom
        // 0.7.2, which can lose a plain store racing a swap and report an
        // outcome the model forbids.
        self.pending.swap(true, Ordering::Release);
    }

    /// Takes the newest value since the last take.
    ///
    /// Thread: UI.
    pub(crate) fn take(&self) -> Option<f64> {
        self.pending
            .swap(false, Ordering::Acquire)
            .then(|| f64::from_bits(self.bits.load(Ordering::Relaxed)))
    }
}

#[cfg(all(test, loom))]
mod loom_models {
    use std::sync::Arc;

    use super::Staging;

    /// The audio thread stages twice while the UI takes twice. A take that
    /// sees a value pending reads at least that value, so after
    /// the audio thread ends, the newest value is the last one applied.
    /// With a relaxed flag, a take could see the second stage's flag, read
    /// the first value and clear the flag, and the newest would be lost.
    ///
    /// Learn checks `Target::held` before it takes, so a value it cannot
    /// apply stays pending and no retry is needed.
    #[test]
    fn loom_the_newest_staged_value_is_the_last_one_taken() {
        loom::model(|| {
            let staging = Arc::new(Staging::default());
            let audio = Arc::clone(&staging);
            let thread = loom::thread::spawn(move || {
                audio.stage(1.0);
                audio.stage(2.0);
            });
            let mut last = None;
            for _ in 0..2 {
                if let Some(value) = staging.take() {
                    last = Some(value);
                }
            }
            thread.join().expect("the audio thread");
            if let Some(value) = staging.take() {
                last = Some(value);
            }
            assert_eq!(last, Some(2.0));
        });
    }
}
