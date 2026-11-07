//! Instruction counts for reading UMP: splitting a stream into packets and
//! decoding a channel voice message. Run by CI with iai-callgrind, which
//! compares them with the pull request's base.

#![allow(missing_docs, reason = "the benchmark macros generate public items")]

use std::hint::black_box;

use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use seam_ump::{ChannelVoice, Packets};

/// A stream of one packet of every size, repeated.
fn stream() -> Vec<u32> {
    let packets: [&[u32]; 5] = [
        &[0x2090_3C7F],
        &[0x4090_3C00, 0xC000_0000],
        &[0x0000_0000],
        &[0x3016_7E7F, 0x0D70_0201],
        &[0xF000_0000, 0, 0, 0],
    ];
    (0..64)
        .flat_map(|_| packets.iter().copied().flatten().copied())
        .collect()
}

#[library_benchmark]
#[bench::mixed(stream())]
fn split_a_stream(words: Vec<u32>) -> usize {
    black_box(Packets::new(black_box(&words), 0).count())
}

#[library_benchmark]
#[bench::midi1_note_on([0x2090_3C7F, 0, 0, 0])]
#[bench::midi2_note_on([0x4090_3C00, 0xC000_0000, 0, 0])]
#[bench::midi2_controller([0x40B1_0200, 0x8000_0000, 0, 0])]
fn decode_a_voice(words: [u32; 4]) -> Option<ChannelVoice> {
    black_box(ChannelVoice::decode(black_box(words)))
}

library_benchmark_group!(name = ump; benchmarks = split_a_stream, decode_a_voice);
main!(library_benchmark_groups = ump);
