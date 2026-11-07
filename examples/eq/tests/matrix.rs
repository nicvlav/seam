//! The example EQ through both exporters across the conditions hosts make,
//! under the allocation gate (debug and release) and the audio-thread mark
//! the exporters hold around a block: the block sizes and sample rates
//! below, every layout the EQ offers, every parameter moving with several
//! points a block, and blocks longer than the setup promised, which play in
//! pieces.
//!
//! Each block also plays through the processor driven directly, after the
//! same values, and the two must agree sample for sample: an exporter applies
//! each queue's last point before the block, so many points per parameter
//! compare with one write. The output must be finite and free of denormals.
//!
//! `soak` runs the grid at random for `SEAM_SOAK_SECONDS` (default 10), and
//! is ignored in a normal run. It prints its seed; `SEAM_SOAK_SEED` repeats a
//! run.
//!
//! ```sh
//! SEAM_SOAK_SECONDS=1800 cargo test --release -p seam-example-eq \
//!     --test matrix -- --ignored soak
//! ```
//!
//! The exporters hand a plugin no MIDI yet, so only the processor-direct
//! side fills its event lists, to capacity.

seam_testkit::install_alloc_gate!();

use std::time::{Duration, Instant};

use seam_example_eq::{ExampleEq, PARAMS};
use seam_host::cabi::TestHost as CHost;
use seam_host::vst3::TestHost as Vst3Host;
use seam_host::{Block, Event, EventList, Layout, Plugin, ProcessSetup, Processor};
use seam_params::{audio_thread, host_id};
use seam_testkit::alloc::assert_no_allocations;

const BLOCK_SIZES: [usize; 8] = [1, 2, 31, 32, 64, 512, 4_096, 8_192];
const RATES: [f64; 6] = [44_100.0, 48_000.0, 88_200.0, 96_000.0, 176_400.0, 192_000.0];
const MAX_FRAMES: usize = 8_192;
/// Up to this many points per parameter in a block. The exporter has room
/// for 8 per parameter on average, so a block with every parameter moving
/// overflows it only by chance, about one in a hundred; `overflow.rs`
/// reaches the branch that drops points on purpose.
const MOST_POINTS: usize = 12;
/// The capacity of the event lists the processor-direct side fills.
const EVENT_ROOM: usize = 256;

/// xorshift64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, count: usize) -> usize {
        usize::try_from(self.next() % count as u64).expect("a small count")
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn sample(&mut self) -> f32 {
        (self.unit() * 2.0 - 1.0) as f32
    }

    /// A host's 0..1 value, with the edges and the lattice's last points
    /// over-represented.
    fn value(&mut self) -> f64 {
        match self.below(8) {
            0 => 0.0,
            1 => 1.0,
            2 => 0.95,
            3 => 0.999_999,
            4 => 1e-9,
            _ => self.unit(),
        }
    }
}

/// One parameter's points in a block: `(sample offset, 0..1)`, offsets
/// ascending.
struct Queue {
    parameter: usize,
    points: Vec<(i32, f64)>,
}

fn queues(rng: &mut Rng, frames: usize, density: usize) -> Vec<Queue> {
    let mut chosen = Vec::new();
    for parameter in 0..PARAMS.len() {
        if density == 0 || rng.below(density) != 0 {
            continue;
        }
        let count = 1 + rng.below(MOST_POINTS.min(frames));
        let mut offsets: Vec<i32> = (0..count)
            .map(|_| i32::try_from(rng.below(frames)).expect("a block size"))
            .collect();
        offsets.sort_unstable();
        offsets.dedup();
        let points = offsets
            .into_iter()
            .map(|offset| (offset, rng.value()))
            .collect();
        chosen.push(Queue { parameter, points });
    }
    chosen
}

/// What the example EQ does with `queues` and `input`, run directly.
struct Direct {
    plugin: ExampleEq,
    processor: Option<<ExampleEq as Plugin>::Processor>,
    saved: std::sync::Arc<seam_host::SavedState>,
    events: EventList,
    out_events: EventList,
}

impl Direct {
    fn open(layout: Layout, rate: f64) -> Self {
        let mut plugin = ExampleEq::new();
        let saved = plugin.saved();
        let processor = plugin.activate(&ProcessSetup {
            sample_rate: rate,
            max_frames: MAX_FRAMES,
            inputs: layout.inputs,
            outputs: layout.outputs,
            max_events: EVENT_ROOM,
            max_out_events: EVENT_ROOM,
        });
        Direct {
            plugin,
            processor: Some(processor),
            saved,
            events: EventList::with_capacity(EVENT_ROOM),
            out_events: EventList::with_capacity(EVENT_ROOM),
        }
    }

    /// Writes each queue's last point, as an exporter does before a block,
    /// then runs the block with both event lists full to capacity.
    fn process(&mut self, channels: &mut [&mut [f32]], frames: usize, queues: &[Queue]) {
        let set = self.saved.set();
        let writes = set.audio();
        for queue in queues {
            let id = set
                .by_host_id(host_id(PARAMS[queue.parameter].key))
                .expect("declared");
            let (_, last) = *queue.points.last().expect("a point");
            writes.host_write(id, last);
        }
        self.events.clear();
        self.out_events.clear();
        for offset in 0..=EVENT_ROOM {
            self.events.push(Event {
                offset: u32::try_from(offset % frames).expect("a block size"),
                words: [0x2090_3C00, 0, 0, 0],
            });
        }
        let _audio_thread = audio_thread::enter();
        let processor = self.processor.as_mut().expect("active");
        assert_no_allocations("the processor driven directly", || {
            processor.process(Block::new(
                channels,
                frames,
                &self.events,
                &mut self.out_events,
            ));
        });
    }
}

impl Drop for Direct {
    fn drop(&mut self) {
        if let Some(processor) = self.processor.take() {
            self.plugin.deactivate(processor);
        }
    }
}

/// An exporter driven the way a host drives it.
trait Shell {
    const NAME: &'static str;
    /// Activates for blocks of at most `max_frames`.
    fn open(layout: Layout, rate: f64, max_frames: usize) -> Self;
    /// Processes `channels` in place after `queues`, under the allocation
    /// gate.
    fn block(&mut self, channels: &mut [&mut [f32]], frames: usize, queues: &[Queue]);
}

struct Vst3 {
    host: Vst3Host<ExampleEq>,
}

impl Shell for Vst3 {
    const NAME: &'static str = "VST3";

    fn open(layout: Layout, rate: f64, max_frames: usize) -> Self {
        let host = Vst3Host::<ExampleEq>::new();
        assert!(host.arrange(layout));
        assert!(host.setup(rate, max_frames));
        assert!(host.activate());
        assert!(host.start());
        Vst3 { host }
    }

    fn block(&mut self, channels: &mut [&mut [f32]], frames: usize, queues: &[Queue]) {
        let sends: Vec<(u32, &[(i32, f64)])> = queues
            .iter()
            .map(|queue| {
                (
                    host_id(PARAMS[queue.parameter].key),
                    queue.points.as_slice(),
                )
            })
            .collect();
        assert_no_allocations("a block through the VST3 exporter", || {
            assert!(self.host.process_queues(channels, frames, &sends));
        });
    }
}

struct CInterface {
    host: CHost<ExampleEq>,
}

impl Shell for CInterface {
    const NAME: &'static str = "C interface";

    fn open(layout: Layout, rate: f64, max_frames: usize) -> Self {
        let host = CHost::<ExampleEq>::new();
        assert!(host.activate(rate, u32::try_from(max_frames).expect("small"), layout));
        for (index, decl) in PARAMS.iter().enumerate() {
            let info = host
                .parameter(u32::try_from(index).expect("small"))
                .expect("listed");
            assert_eq!(
                info.key, decl.key,
                "the C interface lists the declared order"
            );
        }
        CInterface { host }
    }

    /// The C interface carries a value, not points: the last of each queue
    /// is written on the audio side before the block.
    fn block(&mut self, channels: &mut [&mut [f32]], frames: usize, queues: &[Queue]) {
        let side = self.host.audio_side();
        assert_no_allocations("a block through the C interface", || {
            for queue in queues {
                let (_, last) = *queue.points.last().expect("a point");
                side.set(u32::try_from(queue.parameter).expect("small"), last);
            }
            side.process_in_place(channels, frames);
        });
    }
}

/// What a run has done, for the soak's closing line.
#[derive(Default)]
struct Tally {
    blocks: u64,
    frames: u64,
    points: u64,
}

/// Plays `rounds` blocks of `frames` frames through shell `S`, activated for
/// blocks of at most `max_frames`, and the processor driven directly, and
/// checks they agree and the output is clean.
fn play<S: Shell>(
    rng: &mut Rng,
    layout: Layout,
    rate: f64,
    (max_frames, frames): (usize, usize),
    rounds: usize,
    tally: &mut Tally,
) {
    let mut shell = S::open(layout, rate, max_frames);
    let mut direct = Direct::open(layout, rate);
    let mut got = vec![vec![0.0f32; frames]; layout.outputs];
    let mut expected = got.clone();
    for round in 0..rounds {
        // Steady blocks, blocks with a few moves and blocks with every
        // parameter moving; some of the input is silence, to let the
        // filters ring out.
        let density = [0, 8, 3, 1][round % 4];
        let queues = queues(rng, frames, density);
        let silent = round % 5 == 4;
        for sample in got.iter_mut().flatten() {
            *sample = if silent { 0.0 } else { rng.sample() };
        }
        expected.clone_from(&got);
        let mut got_channels: Vec<&mut [f32]> = got.iter_mut().map(Vec::as_mut_slice).collect();
        shell.block(&mut got_channels, frames, &queues);
        let mut expected_channels: Vec<&mut [f32]> =
            expected.iter_mut().map(Vec::as_mut_slice).collect();
        direct.process(&mut expected_channels, frames, &queues);
        for (channel, (got, expected)) in got.iter().zip(&expected).enumerate() {
            for (frame, (&sample, &expected)) in got.iter().zip(expected).enumerate() {
                assert!(
                    sample.is_finite() && !sample.is_subnormal(),
                    "{}: {sample} at frame {frame}, channel {channel}, {frames} frames, \
                     {rate} Hz, {}-in {}-out, round {round}",
                    S::NAME,
                    layout.inputs,
                    layout.outputs
                );
                assert!(
                    agree(sample, expected),
                    "{} differs from the processor driven directly at frame {frame}, \
                     channel {channel}, {frames} frames, {rate} Hz, {}-in {}-out, \
                     round {round}: {sample:e} against {expected:e}",
                    S::NAME,
                    layout.inputs,
                    layout.outputs
                );
            }
        }
        tally.blocks += 1;
        tally.frames += frames as u64;
        tally.points += queues
            .iter()
            .map(|queue| queue.points.len() as u64)
            .sum::<u64>();
    }
}

/// Whether two samples are the same, bit for bit, except in a filter's ringing
/// tail below -400 dBFS: an exporter flushes denormals to zero and a
/// processor driven directly does not, so the tails drift apart there while
/// both are silent.
fn agree(got: f32, expected: f32) -> bool {
    const SILENT: f32 = 1e-20;
    got.to_bits() == expected.to_bits() || (got.abs() < SILENT && expected.abs() < SILENT)
}

/// Fewer rounds for the long blocks, which cost more each.
fn rounds_for(frames: usize) -> usize {
    if frames >= 4_096 { 8 } else { 24 }
}

#[test]
fn eight_block_sizes_and_six_rates_in_every_layout_through_both_exporters() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut tally = Tally::default();
    for layout in ExampleEq::INFO.layouts {
        for rate in RATES {
            for frames in BLOCK_SIZES {
                let (rounds, sizes) = (rounds_for(frames), (MAX_FRAMES, frames));
                play::<Vst3>(&mut rng, *layout, rate, sizes, rounds, &mut tally);
                play::<CInterface>(&mut rng, *layout, rate, sizes, rounds, &mut tally);
            }
        }
    }
    assert!(
        tally.points > 0,
        "the grid moved parameters, with several points a block"
    );
}

#[test]
fn a_block_longer_than_the_setup_plays_in_pieces_through_both_exporters() {
    // 1,500 frames from a setup of 512: two whole pieces and a short one,
    // against the processor driven directly over the whole block.
    let sizes = (512, 1_500);
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut tally = Tally::default();
    for layout in ExampleEq::INFO.layouts {
        play::<Vst3>(&mut rng, *layout, 48_000.0, sizes, 24, &mut tally);
        play::<CInterface>(&mut rng, *layout, 48_000.0, sizes, 24, &mut tally);
    }
    assert!(tally.points > 0, "the pieces carried moving parameters");
}

#[test]
#[ignore = "runs for SEAM_SOAK_SECONDS; see the module docs"]
fn soak() {
    let seconds: u64 = std::env::var("SEAM_SOAK_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(10);
    let seed: u64 = std::env::var("SEAM_SOAK_SEED")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("after 1970")
                .as_nanos() as u64
                | 1
        });
    eprintln!("soak: {seconds} s, SEAM_SOAK_SEED={seed}");
    let mut rng = Rng(seed);
    let mut tally = Tally::default();
    let (mut sessions, deadline) = (0u64, Instant::now() + Duration::from_secs(seconds));
    let layouts = ExampleEq::INFO.layouts;
    while Instant::now() < deadline {
        let layout = layouts[rng.below(layouts.len())];
        let rate = RATES[rng.below(RATES.len())];
        let frames = BLOCK_SIZES[rng.below(BLOCK_SIZES.len())];
        let (rounds, sizes) = (1 + rng.below(64), (MAX_FRAMES, frames));
        if rng.below(2) == 0 {
            play::<Vst3>(&mut rng, layout, rate, sizes, rounds, &mut tally);
        } else {
            play::<CInterface>(&mut rng, layout, rate, sizes, rounds, &mut tally);
        }
        sessions += 1;
    }
    eprintln!(
        "soak: {sessions} activations, {} blocks, {} frames, {} automation points, \
         no allocation, fault, NaN, denormal or drift from the processor driven directly",
        tally.blocks, tally.frames, tally.points
    );
}
