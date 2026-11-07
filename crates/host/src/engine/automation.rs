//! The host's automation for one block, sample positions kept.
//!
//! A host may send several points per parameter in a block, each at a
//! sample offset: VST3 as one queue per parameter, CLAP as one list of
//! every parameter's points in time order. The engine carries every point
//! it has room for, in the host's order. It applies each parameter's last
//! point before the block, so the processor sees one value per parameter
//! per block. Of two queues for one parameter the first wins, but for one
//! exception; [`Points::queue`] states the rule and its edge cases.
//!
//! The walk is bounded however much a host claims. It takes one queue for
//! each parameter of the plugin, and passes over at most
//! [`MAX_SKIPPED_QUEUES`] others. It reads at most as many points as the
//! block has room for, plus each queue's last point.
//!
//! Open: no processor reads the carried points yet, since sample-accurate
//! automation is not built. The walk reads and stores them every block, and
//! only the tests look at them.

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

use seam_params::{AudioParams, ParamId};

/// Room for this many points per parameter in a block, on average.
const POINTS_PER_PARAMETER: usize = 8;

/// The most queues one block's walk passes over.
///
/// A queue passed over is a missing or empty one, one for an id the plugin
/// does not declare, such as a hidden proxy, or a repeat ([`Points::queue`]
/// says when). A host lists those in any order, before the plugin's own too,
/// so they have a bound of their own and never use up the parameters'.
///
/// The bound is above learn's grid of hidden proxies in VST3, 16 channels by
/// 130 controllers, with room for the pool of MIDI 2.0 controllers beside
/// it. A block that moves every proxy therefore still reaches the plugin's
/// own queues.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "only the VST3 shell walks queues")
)]
pub(crate) const MAX_SKIPPED_QUEUES: usize = 4096;

/// One host write at a sample position in the block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Point {
    pub(crate) id: ParamId,
    pub(crate) offset: u32,
    pub(crate) normalized: f64,
}

/// One block's automation, at a capacity fixed when the engine is built.
#[derive(Debug)]
pub(crate) struct Automation {
    /// Every point carried, in the host's order.
    points: Vec<Point>,
    /// Each parameter the block's queues named, once, in the order the host
    /// first named it.
    taken: Vec<Taken>,
    /// Where each parameter sits in `taken` this block, by the parameter's
    /// index.
    places: Box<[Option<usize>]>,
    /// Points the block had no room for; each queue's last still lands.
    dropped: u64,
}

/// A parameter a block's queues named.
#[derive(Debug, Clone, Copy)]
struct Taken {
    id: ParamId,
    /// The value it takes for the block: the last point of its first queue,
    /// or its latest point from a host that sends points one at a time.
    /// `None` while no such point could be read.
    normalized: Option<f64>,
}

impl Automation {
    /// Room for the automation of `parameters` parameters, or none for a
    /// shell whose host sends no queues.
    pub(crate) fn new(parameters: usize, queues: bool) -> Self {
        let queues = if queues { parameters } else { 0 };
        Automation {
            points: Vec::with_capacity(queues.saturating_mul(POINTS_PER_PARAMETER)),
            taken: Vec::with_capacity(queues),
            places: vec![None; queues].into_boxed_slice(),
            dropped: 0,
        }
    }

    /// Empties the room for the next block. Only the places the last block
    /// set are cleared, so the cost is that block's, not the parameter
    /// count's.
    fn clear(&mut self) {
        for taken in &self.taken {
            if let Some(place) = self.places.get_mut(taken.id.index()) {
                *place = None;
            }
        }
        self.points.clear();
        self.taken.clear();
        self.dropped = 0;
    }

    /// Takes `id` for a queue: its place in `taken`, and whether an earlier
    /// queue in the block named it already. `None` when there is no room for
    /// it, which only a shell given no room for queues meets.
    fn take(&mut self, id: ParamId) -> Option<(usize, bool)> {
        let place = self.places.get_mut(id.index())?;
        if let Some(earlier) = *place {
            return Some((earlier, true));
        }
        let index = self.taken.len();
        // Never full: a parameter takes one place, and there is one for each.
        if index >= self.taken.capacity() {
            return None;
        }
        self.taken.push(Taken {
            id,
            normalized: None,
        });
        *place = Some(index);
        Some((index, false))
    }

    /// Carries `point` while more than `reserve` places are left, and
    /// counts it as dropped otherwise.
    fn carry(&mut self, point: Point, reserve: usize) {
        if self.points.len() < self.points.capacity().saturating_sub(reserve) {
            self.points.push(point);
        } else {
            self.dropped = self.dropped.saturating_add(1);
        }
    }

    /// The points the last block had no room for.
    #[cfg(test)]
    pub(crate) fn dropped(&self) -> u64 {
        self.dropped
    }
}

/// A shell's way to hand the engine one block's automation, queue by
/// queue or point by point. Wait-free and free of allocation.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "only the VST3 shell walks queues, which these count"
    )
)]
#[derive(Debug)]
pub(crate) struct Points<'a> {
    /// `None` when another block holds the engine's room, which only a
    /// host calling `process` twice at once makes happen.
    /// [`queue`](Self::queue) says what the walk does then.
    automation: Option<&'a mut Automation>,
    host: &'a AudioParams,
    /// The plugin's parameter count: the most queues the walk takes.
    queues: usize,
    taken: usize,
    skipped: usize,
    /// Reads of earlier points left for the block, as many as it has room
    /// for, whether they succeed or not.
    reads: usize,
}

impl<'a> Points<'a> {
    pub(super) fn new(automation: Option<&'a mut Automation>, host: &'a AudioParams) -> Self {
        let queues = host.set().len();
        let reads = automation
            .as_deref()
            .map_or(0, |automation| automation.points.capacity());
        let mut points = Points {
            automation,
            host,
            queues,
            taken: 0,
            skipped: 0,
            reads,
        };
        if let Some(automation) = points.automation.as_deref_mut() {
            automation.clear();
        }
        points
    }

    /// Writes each parameter's value for the block, in the order the host
    /// first named them.
    pub(super) fn apply(self) {
        if let Some(automation) = self.automation {
            for taken in &automation.taken {
                if let Some(normalized) = taken.normalized {
                    self.host.host_write(taken.id, normalized);
                }
            }
        }
    }
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "only the VST3 shell walks queues")
)]
impl Points<'_> {
    /// Whether the walk goes on, as a shell asks before each queue.
    ///
    /// The walk goes on while it has taken queues for fewer parameters than
    /// the plugin has and passed over fewer than [`MAX_SKIPPED_QUEUES`]. A
    /// shell stops at the first `false`, however many queues its host claims.
    pub(crate) fn walking(&self) -> bool {
        self.taken < self.queues && self.skipped < MAX_SKIPPED_QUEUES
    }

    /// Passes over one queue the host listed that names no parameter of
    /// the plugin, or that the shell could not read.
    pub(crate) fn skip(&mut self) {
        self.skipped = self.skipped.saturating_add(1);
    }

    /// Takes one queue of `count` points for parameter `id`.
    ///
    /// `read` answers the point at an index, as its offset and its 0..1
    /// value, or `None` for one the host cannot give. The last point is read
    /// first, and its value is the parameter's for the block. The earlier
    /// points are read while the block's room lasts. Every point neither
    /// carried nor a failed read is counted as dropped. The first queue for a
    /// parameter wins, wherever a repeat comes: a repeat is passed over
    /// unread, as [`skip`](Self::skip) passes one.
    ///
    /// | Case | What happens | Test |
    /// | --- | --- | --- |
    /// | The walk is over | Nothing is read. | `a_host_claiming_more_queues_than_parameters_is_walked_no_further` |
    /// | No points | Passed over, as `skip` passes one. | `queues_for_ids_the_plugin_does_not_know_never_starve_its_own` |
    /// | The last read fails | The queue counts and keeps the parameter's place, so a repeat after it is still passed over. The parameter takes no value this block, and no other point of the queue is read or counted. | `a_first_queue_whose_last_read_fails_still_wins_over_a_repeat` |
    /// | Room runs short | A failed read uses up the room too. An earlier point is carried while places are left for it and the queue's last, and the last while one is left. | `a_queue_whose_reads_fail_is_read_no_further_than_the_block_has_room` |
    /// | Another block holds the room | Nothing records which parameters the block named. Every queue counts toward the parameters, and only its last point is read and written, at once. A repeat's value then lands after the first's, and the walk ends a queue sooner for each repeat. | `while_another_block_holds_the_room_a_repeat_stands_and_takes_a_place` |
    /// | The shell was given no room | The last point is written at once, and the points before it are counted as dropped. | `a_queue_given_no_room_lands_its_last_point_and_counts_only_the_rest_dropped` |
    pub(crate) fn queue(
        &mut self,
        id: ParamId,
        count: usize,
        mut read: impl FnMut(usize) -> Option<(u32, f64)>,
    ) {
        let Some(last_index) = count.checked_sub(1) else {
            self.skip();
            return;
        };
        // A guard only: the shells stop once `walking` is false, so this
        // keeps the walk bounded whatever a shell does.
        if !self.walking() {
            return;
        }
        let place = self
            .automation
            .as_deref_mut()
            .and_then(|automation| automation.take(id));
        // A parameter named again is passed over unread, using a place of
        // the skipped queues' own, so a host that lists one twice starves no
        // other.
        if place.is_some_and(|(_, again)| again) {
            self.skip();
            return;
        }
        self.taken = self.taken.saturating_add(1);
        let Some((offset, normalized)) = read(last_index) else {
            return;
        };
        let (Some(automation), Some((place, _))) = (self.automation.as_deref_mut(), place) else {
            // Another block holds the room, or a shell was given none. The
            // last point lands; only the points before it are dropped.
            if let Some(automation) = self.automation.as_deref_mut() {
                automation.dropped = automation.dropped.saturating_add(last_index as u64);
            }
            self.host.host_write(id, normalized);
            return;
        };
        // The last point's index is how many points come before it.
        let earlier = last_index;
        let tries = earlier.min(self.reads);
        self.reads = self.reads.saturating_sub(tries);
        automation.dropped = automation
            .dropped
            .saturating_add(earlier.saturating_sub(tries) as u64);
        for index in 0..tries {
            if let Some((offset, normalized)) = read(index) {
                // One place stays free for the queue's last point.
                automation.carry(
                    Point {
                        id,
                        offset,
                        normalized,
                    },
                    1,
                );
            }
        }
        automation.carry(
            Point {
                id,
                offset,
                normalized,
            },
            0,
        );
        if let Some(taken) = automation.taken.get_mut(place) {
            taken.normalized = Some(normalized);
        }
    }
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "for CLAP's shell, which is not built yet")
)]
impl Points<'_> {
    /// Takes one point for parameter `id`, from a host that sends every
    /// parameter's points in one list in time order, as CLAP does.
    ///
    /// The point is carried while the block has room, and its value is the
    /// parameter's for the block until a later point for it. It reads
    /// nothing, so the shell's own walk of the host's list is what bounds the
    /// block.
    pub(crate) fn point(&mut self, id: ParamId, offset: u32, normalized: f64) {
        let Some(automation) = self.automation.as_deref_mut() else {
            self.host.host_write(id, normalized);
            return;
        };
        automation.carry(
            Point {
                id,
                offset,
                normalized,
            },
            0,
        );
        match automation.take(id) {
            Some((place, _)) => {
                if let Some(taken) = automation.taken.get_mut(place) {
                    taken.normalized = Some(normalized);
                }
            }
            None => self.host.host_write(id, normalized),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use seam_params::{Kind, ParamDecl, ParamSet};

    use super::*;

    fn two_parameters() -> std::sync::Arc<ParamSet> {
        ParamSet::from_decls(
            ["a", "b"]
                .map(|key| ParamDecl::new(key, key, Kind::continuous(0.0, 1.0, 0.0, None), 0.0)),
        )
        .expect("valid")
    }

    #[test]
    fn points_in_time_order_are_carried_and_each_parameter_takes_its_last() {
        let set = two_parameters();
        let (a, b) = (set.id("a").expect("a"), set.id("b").expect("b"));
        let audio = set.audio();
        let mut room = Automation::new(2, true);
        let sent = [
            (a, 0, 0.125),
            (b, 0, 0.25),
            (a, 4, 0.375),
            (b, 8, 0.625),
            (a, 9, 0.75),
        ];
        {
            let mut points = Points::new(Some(&mut room), &audio);
            for (id, offset, normalized) in sent {
                points.point(id, offset, normalized);
            }
            points.apply();
        }
        let carried: Vec<_> = room
            .points
            .iter()
            .map(|point| (point.id, point.offset, point.normalized))
            .collect();
        assert_eq!(carried, sent, "every point, in the host's order");
        assert_eq!((set.normalized(a), set.normalized(b)), (0.75, 0.625));

        // Past the room, points are dropped and counted, and the last lands.
        {
            let mut points = Points::new(Some(&mut room), &audio);
            for offset in 0..=2 * POINTS_PER_PARAMETER as u32 {
                points.point(b, offset, f64::from(offset) / 64.0);
            }
            points.apply();
        }
        assert_eq!((room.points.len(), room.dropped()), (16, 1));
        assert_eq!(set.normalized(b), 0.25);
    }

    #[test]
    fn a_point_while_another_block_holds_the_room_lands_at_once() {
        let set = two_parameters();
        let a = set.id("a").expect("a");
        let audio = set.audio();
        let mut points = Points::new(None, &audio);
        points.point(a, 3, 0.5);
        assert_eq!(set.normalized(a), 0.5, "written before the block's apply");
        points.apply();
        assert_eq!(set.normalized(a), 0.5);
    }

    #[test]
    fn a_first_queue_whose_last_read_fails_still_wins_over_a_repeat() {
        let set = two_parameters();
        let a = set.id("a").expect("a");
        let audio = set.audio();
        let mut room = Automation::new(2, true);
        {
            let mut points = Points::new(Some(&mut room), &audio);
            points.queue(a, 1, |_| None);
            points.queue(a, 1, |_| Some((0, 0.75)));
            points.apply();
        }
        assert_eq!(set.normalized(a), 0.0, "no value this block");
    }

    #[test]
    fn while_another_block_holds_the_room_a_repeat_stands_and_takes_a_place() {
        let set = two_parameters();
        let (a, b) = (set.id("a").expect("a"), set.id("b").expect("b"));
        let audio = set.audio();
        let mut points = Points::new(None, &audio);
        points.queue(a, 1, |_| Some((0, 0.25)));
        points.queue(a, 1, |_| Some((0, 0.75)));
        points.queue(b, 1, |_| Some((0, 0.5)));
        points.apply();
        assert_eq!(set.normalized(a), 0.75, "each queue wrote at once");
        assert_eq!(set.normalized(b), 0.0, "the repeat took b's place");
    }

    #[test]
    fn a_queue_given_no_room_lands_its_last_point_and_counts_only_the_rest_dropped() {
        let set = two_parameters();
        let a = set.id("a").expect("a");
        let audio = set.audio();
        let mut room = Automation::new(2, false);
        {
            let mut points = Points::new(Some(&mut room), &audio);
            points.queue(a, 3, |index| {
                Some((index as u32, 0.25 * (index + 1) as f64))
            });
            points.apply();
        }
        assert_eq!(set.normalized(a), 0.75, "the last landed");
        assert_eq!(room.dropped(), 2, "the two before it");
    }

    #[test]
    fn a_queue_whose_reads_fail_is_read_no_further_than_the_block_has_room() {
        let set = two_parameters();
        let audio = set.audio();
        let mut room = Automation::new(2, true);
        let reads = Cell::new(0usize);
        let claimed = 16_000_000;
        {
            let mut points = Points::new(Some(&mut room), &audio);
            for key in ["a", "b"] {
                points.queue(set.id(key).expect("declared"), claimed, |index| {
                    reads.set(reads.get() + 1);
                    // Only the last point can be read.
                    (index == claimed - 1).then_some((0, 0.5))
                });
            }
            points.apply();
        }
        assert!(
            reads.get() <= 2 * POINTS_PER_PARAMETER + 2,
            "{} reads on the audio thread",
            reads.get()
        );
        assert_eq!(
            set.value(set.id("b").expect("declared")),
            0.5,
            "the last lands"
        );
        assert_eq!(
            room.dropped(),
            2 * (claimed as u64 - 1) - (2 * POINTS_PER_PARAMETER) as u64,
            "every point not tried is dropped"
        );
    }
}
