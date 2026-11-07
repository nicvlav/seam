//! Reading the host's buses: the channel buffers of a process call, silence
//! flags, and the arrangements the exporter takes.

use ::vst3::Steinberg::Vst::{AudioBusBuffers, ProcessData, SpeakerArr, SpeakerArrangement};
use ::vst3::Steinberg::int32;

use crate::Plugin;

/// The channel buffers of a direction's main bus.
///
/// # Safety
///
/// `buses` is null, or points to `count` buses of a 32-bit setup, readable
/// and unchanged for `'a`, whose channel arrays hold `numChannels` pointers
/// for `'a`.
pub(super) unsafe fn bus_channels<'a>(buses: *mut AudioBusBuffers, count: int32) -> &'a [*mut f32] {
    if buses.is_null() || count < 1 {
        return &[];
    }
    // SAFETY: the caller's contract: `buses` is non-null and `count` at least
    // 1, so the first bus is readable.
    let bus = unsafe { &*buses };
    // SAFETY: the caller's contract makes the setup 32-bit, so the union's
    // 32-bit field is the one the host wrote.
    let channels = unsafe { bus.__field0.channelBuffers32 };
    match usize::try_from(bus.numChannels) {
        Ok(length) if length > 0 && !channels.is_null() => {
            // SAFETY: the caller's contract: a non-null channel array holds
            // `numChannels` pointers for `'a`.
            unsafe { std::slice::from_raw_parts(channels, length) }
        }
        _ => &[],
    }
}

/// Zeroes every output channel and flags it silent.
///
/// # Safety
///
/// `data.outputs` is null or holds `numOutputs` buses of a 32-bit setup,
/// valid for reads and writes until the caller returns. Each channel is
/// null or valid for writing `numSamples` floats, and nothing else touches
/// them meanwhile.
pub(super) unsafe fn silence(data: &mut ProcessData) {
    if data.outputs.is_null() {
        return;
    }
    let frames = usize::try_from(data.numSamples).unwrap_or(0);
    let buses = usize::try_from(data.numOutputs).unwrap_or(0);
    for bus in 0..buses {
        // SAFETY: the caller's contract: `bus` is below `numOutputs`, so the
        // bus is one of the host's, as `bus_channels` requires.
        let channels = unsafe { bus_channels(data.outputs.add(bus), 1) };
        for &channel in channels {
            if !channel.is_null() {
                // SAFETY: the caller's contract: each channel is valid for
                // writing `numSamples` floats.
                unsafe { std::ptr::write_bytes(channel, 0, frames) };
            }
        }
    }
    // SAFETY: as above.
    unsafe { set_silence_flags(data, true) };
}

/// Flags every output channel silent, or none.
///
/// # Safety
///
/// `data.outputs` is null or holds `numOutputs` buses, valid for reads and
/// writes until the caller returns, which nothing else touches meanwhile.
pub(super) unsafe fn set_silence_flags(data: &mut ProcessData, silent: bool) {
    let buses = usize::try_from(data.numOutputs).unwrap_or(0);
    if data.outputs.is_null() {
        return;
    }
    for bus in 0..buses {
        // SAFETY: the caller's contract: `bus` is below `numOutputs` and
        // `outputs` non-null, so this is one of the host's buses, and no
        // other reference to it exists during the call.
        let bus = unsafe { &mut *data.outputs.add(bus) };
        let channels = u32::try_from(bus.numChannels).unwrap_or(0).min(64);
        bus.silenceFlags = if silent && channels > 0 {
            u64::MAX >> (64 - channels)
        } else {
            0
        };
    }
}

/// The arrangement of a bus of `channels`: none, mono or stereo, the only
/// ones [`check_layouts`](crate::vst3::check_layouts) lets a plugin declare.
/// [`channels_of`] reads each back.
pub(super) fn arrangement(channels: usize) -> Option<SpeakerArrangement> {
    match channels {
        0 => Some(SpeakerArr::kEmpty),
        1 => Some(SpeakerArr::kMono),
        2 => Some(SpeakerArr::kStereo),
        _ => None,
    }
}

/// The channels of an arrangement the exporter takes.
pub(super) fn channels_of(arrangement: SpeakerArrangement) -> Option<usize> {
    match arrangement {
        SpeakerArr::kEmpty => Some(0),
        SpeakerArr::kMono => Some(1),
        SpeakerArr::kStereo => Some(2),
        _ => None,
    }
}

/// Whether the plugin has a main audio input bus.
pub(super) fn has_input<P: Plugin>() -> bool {
    P::INFO.layouts.iter().any(|layout| layout.inputs > 0)
}
