//! What each mode of the parity example computes, written to any `Write`.

use std::io::{BufRead, Write};
use std::sync::Arc;

use seam_example_eq::ExampleEq;
use seam_example_eq::dsp::{Shape, coefficients};
use seam_example_eq::params::EqParams;
use seam_host::{Block, EventList, Layout, Parts, Plugin, ProcessSetup, Processor};
use seam_params::{Float, Kind, ParamDecl, ParamSet, Range};

fn bits(value: f32) -> String {
    format!("{:08x}", value.to_bits())
}

fn bits64(value: f64) -> String {
    format!("{:016x}", value.to_bits())
}

/// The EQ's parameters, and the set they are declared in.
fn declared() -> (Arc<ParamSet>, EqParams) {
    ParamSet::declare::<EqParams>().expect("valid declarations")
}

/// Every parameter's declaration, in the order hosts list them.
fn declarations() -> Vec<ParamDecl> {
    let (set, _) = declared();
    set.ids().map(|id| *set.decl(id)).collect()
}

fn range(decl: &ParamDecl) -> Option<Range> {
    match decl.kind {
        Kind::Continuous(range) => Some(range),
        _ => None,
    }
}

/// Every lattice point's input, as `start + k * interval` read from the
/// written decimals, which the snap must land on its own decimal.
fn lattice_inputs(range: Range) -> impl Iterator<Item = f32> {
    let span = f64::from(range.end()) - f64::from(range.start());
    let count = (span / f64::from(range.interval())).round() as u64;
    (0..=count).map(move |k| {
        let point = f64::from(range.start()) + k as f64 * f64::from(range.interval());
        point as f32
    })
}

/// A small, fixed sequence, the same on every run.
struct Seeded(u64);

impl Seeded {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// In 0..1, as an `f32`.
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn pick(&mut self, count: usize) -> usize {
        (self.next() % count as u64) as usize
    }
}

fn shape_name(shape: Shape) -> &'static str {
    match shape {
        Shape::HighPass => "hp",
        Shape::LowShelf => "low",
        Shape::Peak => "peak",
        Shape::HighShelf => "high",
    }
}

pub(crate) fn ranges(out: &mut impl Write) {
    for decl in &declarations() {
        let (kind, start, end, interval, centre) = match decl.kind {
            Kind::Continuous(range) => (
                "continuous",
                range.start(),
                range.end(),
                range.interval(),
                range.skew_centre().unwrap_or(f32::NAN),
            ),
            Kind::Bool => ("bool", 0.0, 1.0, 1.0, f32::NAN),
            Kind::Choice(_) => unreachable!("the EQ declares no choice"),
        };
        writeln!(
            out,
            "{} {kind} {} {} {} {} {}",
            decl.key,
            bits(start),
            bits(end),
            bits(interval),
            bits(centre),
            bits(decl.default)
        )
        .expect("write a case");
    }
}

pub(crate) fn snap(out: &mut impl Write) {
    for decl in &declarations() {
        let Some(range) = range(decl) else { continue };
        for point in lattice_inputs(range) {
            for fraction in [0.0, 0.25, 0.5, 0.75] {
                let input = (f64::from(point) + fraction * f64::from(range.interval())) as f32;
                let snapped = decl.kind.snap(input).expect("finite");
                writeln!(out, "{} {} {}", decl.key, bits(input), bits(snapped))
                    .expect("write a case");
            }
        }
    }
}

pub(crate) fn normalized(out: &mut impl Write) {
    let mut seeded = Seeded(0x9E37_79B9_7F4A_7C15);
    for decl in &declarations() {
        let even = (0..=4_096).map(|step| step as f32 / 4_096.0);
        let random: Vec<f32> = (0..4_096).map(|_| seeded.unit()).collect();
        for normalized in even.chain(random) {
            // The host write: clamp, convert, snap.
            let value = decl
                .kind
                .snap(decl.kind.denormalize(f64::from(normalized)))
                .expect("finite");
            writeln!(out, "{} {} {}", decl.key, bits(normalized), bits(value))
                .expect("write a case");
        }
    }
}

/// Every lattice point of a stepped parameter, snapped.
fn points(decl: &ParamDecl) -> Vec<f32> {
    let range = range(decl).expect("stepped");
    lattice_inputs(range)
        .map(|input| decl.kind.snap(input).expect("finite"))
        .collect()
}

pub(crate) fn coefficients_grid(out: &mut impl Write) {
    let mut seeded = Seeded(0xD1B5_4A32_D192_ED03);
    let mut print = |shape: Shape, gain: f32, frequency: f32, q: f32, rate: f64| {
        let [b0, b1, b2, a1, a2] = coefficients(shape, gain, frequency, q, rate);
        writeln!(
            out,
            "{} {} {} {} {} {} {} {} {} {}",
            shape_name(shape),
            bits(gain),
            bits(frequency),
            bits(q),
            bits64(rate),
            bits(b0),
            bits(b1),
            bits(b2),
            bits(a1),
            bits(a2)
        )
        .expect("write a case");
    };
    let (set, eq) = declared();
    let decl = |param: &Float| *set.decl(param.id());
    for rate in [44_100.0, 48_000.0, 96_000.0] {
        for band in &eq.bands {
            let (frequency, q) = (decl(&band.frequency), decl(&band.q));
            let gain_points = band
                .gain
                .as_ref()
                .map_or_else(|| vec![0.0], |gain| points(&decl(gain)));
            let (frequencies, qs) = (points(&frequency), points(&q));
            let (frequency, q) = (frequency.default, q.default);
            for &gain in &gain_points {
                print(band.shape, gain, frequency, q, rate);
            }
            let boost = if band.gain.is_some() { 6.0 } else { 0.0 };
            for &frequency in &frequencies {
                print(band.shape, boost, frequency, q, rate);
            }
            for &q in &qs {
                print(band.shape, boost, frequency, q, rate);
            }
            for _ in 0..2_000 {
                let gain = gain_points[seeded.pick(gain_points.len())];
                let frequency = frequencies[seeded.pick(frequencies.len())];
                let q = qs[seeded.pick(qs.len())];
                print(band.shape, gain, frequency, q, rate);
            }
        }
    }
}

/// The noise both sides play: xorshift32 as a signed 32-bit integer over
/// 2^31, the same in C++ bit for bit.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as i32 as f32 / 2_147_483_648.0
    }
}

pub(crate) fn audio(input: impl BufRead, out: &mut impl Write) {
    let (params, parts) = Parts::for_plugin::<ExampleEq>().expect("valid declarations");
    let set = Arc::clone(parts.set());
    let mut plugin = ExampleEq::new(params, parts);
    let audio = set.audio();
    let mut processor = None;
    let mut noise = Noise(0x1234_5678);
    let events = EventList::with_capacity(0);
    let mut events_out = EventList::with_capacity(0);
    let (mut left, mut right) = (vec![0.0f32; 4_096], vec![0.0f32; 4_096]);
    for line in input.lines() {
        let line = line.expect("a script line");
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["rate", rate] => {
                let setup = ProcessSetup {
                    sample_rate: rate.parse().expect("a rate"),
                    max_frames: 4_096,
                    layout: Layout::STEREO,
                    max_events: 0,
                    max_out_events: 0,
                };
                processor = Some(plugin.activate(&setup));
            }
            ["set", key, normalized] => {
                let id = set.id(key).expect("declared");
                let normalized = f32::from_bits(u32::from_str_radix(normalized, 16).expect("hex"));
                audio.host_write(id, f64::from(normalized));
            }
            ["reset"] => processor.as_mut().expect("a rate first").reset(),
            ["block", frames] => {
                let frames: usize = frames.parse().expect("frames");
                let (left, right) = (&mut left[..frames], &mut right[..frames]);
                for (left, right) in left.iter_mut().zip(right.iter_mut()) {
                    let sample = noise.next();
                    (*left, *right) = (sample, -sample);
                }
                let mut channels: [&mut [f32]; 2] = [left, right];
                processor
                    .as_mut()
                    .expect("a rate first")
                    .process(Block::new(&mut channels, frames, &events, &mut events_out));
                let [left, right] = channels;
                for (left, right) in left.iter().zip(right.iter()) {
                    writeln!(out, "{} {}", bits(*left), bits(*right)).expect("write a case");
                }
            }
            _ => panic!("unknown script line {line:?}"),
        }
    }
}
