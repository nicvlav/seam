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

use crate::Param;

/// What a reader last saw of a group of parameters, so a processor asks once
/// per block whether any of them moved.
///
/// A processor keeps one per group of parameters it derives something from,
/// a filter's coefficients say, and recomputes only when
/// [`moved`](Self::moved) answers true. It holds no parameter, only a stamp:
/// the members come with each call, and may be of different kinds. The check
/// is a load per member and a compare: no allocation or lock.
/// [`Watch`](crate::Watch) is its UI-thread relative, for one parameter on
/// one surface.
///
/// # Examples
///
/// ```
/// use seam_params::{Changes, Float, Kind, ParamDecl, ParamSet};
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
/// let mut filter = Changes::new();
/// assert!(filter.moved([&freq, &q])); // the first check: tune from the defaults
/// assert!(!filter.moved([&freq, &q]));
///
/// audio.host_write(mix.id(), 0.5); // outside the group
/// assert!(!filter.moved([&freq, &q]));
/// audio.host_write(q.id(), 0.5);
/// assert!(filter.moved([&freq, &q]));
/// assert!(!filter.moved([&freq, &q]));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, Default)]
pub struct Changes {
    /// Which members, and the sum of their counts, at the last check; `None`
    /// before it.
    seen: Option<(u64, u64)>,
}

impl Changes {
    /// Creates one that has seen nothing yet, so its first
    /// [`moved`](Self::moved) answers true.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns whether any of `members` took a write since the last call, a
    /// repeated value included. True at the first call, and when `members`
    /// are other parameters than last time.
    ///
    /// Atomics only: it never locks or allocates.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{Bool, Changes, Float, Kind, ParamDecl, ParamSet};
    ///
    /// let set = ParamSet::from_decls([
    ///     ParamDecl::new("gain", "Gain", Kind::continuous(-60.0, 12.0, 0.1, None), 0.0),
    ///     ParamDecl::new("on", "On", Kind::Bool, 1.0),
    /// ])?;
    /// let gain: Float = set.bind("gain")?;
    /// let on: Bool = set.bind("on")?;
    ///
    /// let mut band = Changes::new();
    /// assert!(band.moved([&gain, &on])); // two kinds in one group
    /// assert!(!band.moved([&gain, &on]));
    /// assert!(band.moved([&gain])); // another group than last time
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]
    pub fn moved<const N: usize>(&mut self, members: [&dyn Param; N]) -> bool {
        let now = members
            .iter()
            .fold((0_u64, 0_u64), |(identity, count), member| {
                let (address, changes) = member.stamp();
                // Each count only grows, so their sum moves whenever one
                // does; it would come back round only after 2^64 writes. The
                // identity mixes in each member's address, in order, so
                // another group reads as a move.
                (
                    identity.rotate_left(7) ^ address as u64,
                    count.wrapping_add(changes),
                )
            });
        if self.seen == Some(now) {
            return false;
        }
        self.seen = Some(now);
        true
    }
}
