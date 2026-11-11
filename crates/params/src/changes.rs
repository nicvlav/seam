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

use crate::AnyParam;
use crate::param::sealed::Token;

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
    /// are other parameters than last time, short of a collision in the mix
    /// that tells groups apart, which is chance, about once in 2^64.
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
    pub fn moved<const N: usize>(&mut self, members: [&dyn AnyParam; N]) -> bool {
        let now = stamp(&members);
        if self.seen == Some(now) {
            return false;
        }
        self.seen = Some(now);
        true
    }
}

/// Which members, mixed in order, and the sum of their counts.
fn stamp(members: &[&dyn AnyParam]) -> (u64, u64) {
    members
        .iter()
        .fold((0_u64, 0_u64), |(identity, count), member| {
            let (serial, changes) = member.stamp(Token(()));
            // Each count only grows, so their sum moves whenever one does;
            // it would come back round only after 2^64 writes.
            (mix(identity, serial), count.wrapping_add(changes))
        })
}

/// Mixes one more member's serial into a group's identity, so another group
/// reads as a move.
///
/// The multiply carries every bit of the input upward and the shift folds the
/// high half back down, so the identity is not linear in the serials: two
/// groups share one only by chance, about once in 2^64. The added constant
/// keeps a member numbered 0 from leaving the identity as it was.
fn mix(identity: u64, member: u64) -> u64 {
    let spread = (identity.rotate_left(5) ^ member)
        .wrapping_add(0x9e37_79b9_7f4a_7c15)
        .wrapping_mul(0xbf58_476d_1ce4_e5b9);
    spread ^ (spread >> 31)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// Whether every group of one or two of `members`, in either order, has
    /// an identity of its own; the first two groups that share one if not.
    fn shared_identity(members: &[u64]) -> Option<(Vec<u64>, Vec<u64>)> {
        // The empty group's identity is the fold's start.
        let mut seen: HashMap<u64, Vec<u64>> = HashMap::from([(0, Vec::new())]);
        for &a in members {
            let mut groups = vec![vec![a]];
            groups.extend(members.iter().filter(|&&b| b != a).map(|&b| vec![a, b]));
            for group in groups {
                let identity = group.iter().fold(0, |identity, &m| mix(identity, m));
                if let Some(other) = seen.insert(identity, group.clone()) {
                    return Some((group, other));
                }
            }
        }
        None
    }

    #[test]
    #[cfg_attr(miri, ignore = "65,536 groups take minutes in the interpreter")]
    fn no_two_small_groups_share_an_identity() {
        // Members numbered in order, and members spaced as heap cells are.
        let numbered: Vec<u64> = (0..256).collect();
        assert_eq!(shared_identity(&numbered), None);
        let spaced: Vec<u64> = (0..256).map(|i| 0x0000_01e4_5a3c_0000 + i * 192).collect();
        assert_eq!(shared_identity(&spaced), None);
    }
}
