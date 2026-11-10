//! The audio thread's check of whether any parameter in a group moved.

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

use crate::{AudioParams, ParamId};

/// A group of up to `N` parameters and the change counts it last saw, so a
/// processor asks once per block whether any of them moved.
///
/// A processor keeps one per group of parameters it derives something from,
/// a filter's coefficients say, and recomputes only when
/// [`moved`](Self::moved) answers true. The group lives inline, so the check
/// is a load per member and a compare: no allocation, lock or pointer to
/// follow. [`Watch`](crate::Watch) is its UI-thread relative, for one
/// parameter on one surface.
///
/// # Examples
///
/// ```
/// use seam_params::{Changes, Float, Handle, Kind, ParamDecl, ParamSet};
///
/// let set = ParamSet::from_decls([
///     ParamDecl::new("freq", "Frequency", Kind::continuous(20.0, 20_000.0, 1.0, None), 1_000.0),
///     ParamDecl::new("q", "Q", Kind::continuous(0.1, 30.0, 0.01, None), 1.0),
///     ParamDecl::new("mix", "Mix", Kind::continuous(0.0, 1.0, 0.0, None), 1.0),
/// ])?;
/// let freq: Float = set.bind("freq")?;
/// let q: Float = set.bind("q")?;
/// let mix: Float = set.bind("mix")?;
/// let audio = set.audio();
///
/// let mut filter: Changes<2> = Changes::new([freq.id(), q.id()]);
/// assert!(filter.moved(&audio)); // the first check: tune from the defaults
/// assert!(!filter.moved(&audio));
///
/// audio.host_write(mix.id(), 0.5); // outside the group
/// assert!(!filter.moved(&audio));
/// audio.host_write(q.id(), 0.5);
/// assert!(filter.moved(&audio));
/// assert!(!filter.moved(&audio));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct Changes<const N: usize> {
    /// The members, in `ids[..len]`; the slots past them are unused.
    ids: [ParamId; N],
    len: usize,
    /// The sum of the members' counts at the last check; `None` before it.
    seen: Option<u64>,
}

impl<const N: usize> Changes<N> {
    /// Creates a group of the parameters `ids`, which has seen nothing yet,
    /// so its first [`moved`](Self::moved) answers true.
    ///
    /// The type says how many it may hold, so a group's size may vary up to
    /// `N`: a filter band with a gain and one without can share a type.
    ///
    /// # Panics
    ///
    /// When `ids` holds more than `N` parameters.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{Changes, Kind, ParamDecl, ParamSet};
    ///
    /// let set = ParamSet::from_decls([ParamDecl::new("on", "On", Kind::Bool, 1.0)])?;
    /// let on = set.id("on").ok_or("declared")?;
    ///
    /// // Room for four, holding one.
    /// let mut band: Changes<4> = Changes::new([on]);
    /// assert!(band.moved(&set.audio()));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[expect(
        clippy::panic,
        reason = "runs where the processor is built, off the audio thread; a group larger than its type is a bug in the plugin, documented under Panics"
    )]
    pub fn new(ids: impl IntoIterator<Item = ParamId>) -> Self {
        let mut slots = [ParamId::from_index(0); N];
        let mut len = 0;
        for id in ids {
            let Some(slot) = slots.get_mut(len) else {
                panic!("a Changes<{N}> holds at most {N} parameters");
            };
            *slot = id;
            len = len.saturating_add(1);
        }
        Changes {
            ids: slots,
            len,
            seen: None,
        }
    }

    fn members(&self) -> &[ParamId] {
        self.ids.get(..self.len).unwrap_or_default()
    }

    /// Returns whether any parameter in the group took a write since the
    /// last call, a repeated value included, and true at the first call.
    /// [`Changes`] shows it answering.
    ///
    /// Atomics only: it never locks or allocates.
    #[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]
    pub fn moved(&mut self, params: &AudioParams) -> bool {
        let set = params.set();
        // Each count only grows, so their sum moves whenever one does. It
        // would come back round only after 2^64 writes.
        let now = self
            .members()
            .iter()
            .fold(0_u64, |sum, &id| sum.wrapping_add(set.changes(id)));
        if self.seen == Some(now) {
            return false;
        }
        self.seen = Some(now);
        true
    }
}
