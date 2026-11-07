//! What the audio thread is handed each block.

pub use seam_ump::{Event, EventList};

/// The most frames a processor is built for per block: 65,536, about 1.4
/// seconds at 48 kHz.
///
/// The processor's buffers are allocated up front, so the exporters build a
/// setup that promises more for this many, and a longer block plays in pieces.
pub const MAX_FRAMES: usize = 65_536;

/// How the host will run the processor.
///
/// The main bus only. Open: a sidechain or any other extra bus is not
/// designed yet, and needs its own shape before a plugin that has one.
///
/// # Examples
///
/// ```
/// use seam_host::{Layout, ProcessSetup};
///
/// // A host's activation for a stereo effect: a processor sizes everything
/// // it needs from this, once, and the audio thread never allocates again.
/// let setup = ProcessSetup {
///     sample_rate: 44_100.0,
///     max_frames: 1_024,
///     inputs: Layout::STEREO.inputs,
///     outputs: Layout::STEREO.outputs,
///     max_events: 256,
///     max_out_events: 0,
/// };
/// let scratch = vec![0.0_f32; setup.max_frames * setup.outputs];
/// assert_eq!(scratch.len(), 2_048);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProcessSetup {
    /// Samples per second.
    pub sample_rate: f64,
    /// The most frames a block will hold, at most [`MAX_FRAMES`].
    pub max_frames: usize,
    /// Input channels.
    pub inputs: usize,
    /// Output channels.
    pub outputs: usize,
    /// The most MIDI events one block may carry in; more are dropped and
    /// counted.
    pub max_events: usize,
    /// The most MIDI events one block may send; more are dropped and
    /// counted. Zero for a plugin without
    /// [`midi_output`](crate::PluginInfo::midi_output).
    pub max_out_events: usize,
}

/// One block, as [`Processor::process`](crate::Processor::process) gets it.
///
/// Everything here is lent by the exporter for one call. A sidechain or
/// transport joins this struct when it is designed, so adding one does not
/// change the trait.
///
/// # Examples
///
/// ```
/// use seam_host::{Block, Processor, Status};
///
/// /// Halves the level, in place, in every channel.
/// struct Half;
///
/// impl Processor for Half {
///     fn process(&mut self, block: Block<'_, '_>) -> Status {
///         for channel in block.channels.iter_mut() {
///             for sample in channel.iter_mut() {
///                 *sample *= 0.5;
///             }
///         }
///         Status::Continue
///     }
/// }
/// # use seam_host::EventList;
/// # let (events, mut out) = (EventList::with_capacity(0), EventList::with_capacity(0));
/// # let mut left = [1.0_f32; 4];
/// # let mut channels = [&mut left[..]];
/// # Half.process(Block::new(&mut channels, 4, &events, &mut out));
/// # assert_eq!(left, [0.5; 4]);
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub struct Block<'a, 'b> {
    /// `max(inputs, outputs)` channels (from [`ProcessSetup`]), each
    /// [`frames`](Self::frames) long, processed in place.
    ///
    /// On entry the first `inputs` carry the input, and any further ones are
    /// zeroed. On return the first `outputs` carry the output.
    pub channels: &'a mut [&'b mut [f32]],
    /// The frames in this block, at most [`ProcessSetup::max_frames`].
    pub frames: usize,
    /// The MIDI that arrived, in delivery order.
    pub events: &'a EventList,
    /// The MIDI to send; push in offset order. MIDI only: parameter changes
    /// reach the host through the exporter.
    ///
    /// Empty on entry, but for a host's block longer than
    /// [`ProcessSetup::max_frames`], which plays as several blocks. Each
    /// later one finds what the earlier ones sent, already at frames from
    /// the host's block's start.
    pub out_events: &'a mut EventList,
}

/// What a block's processing tells the host about the next ones.
///
/// CLAP's process status, which a host uses to stop calling a quiet plugin.
/// VST3 has no such answer, so its exporter ignores it.
///
/// # Examples
///
/// ```
/// use seam_host::Status;
///
/// // A processor that has nothing left to do tells a host that may stop it.
/// fn after(block_was_silent: bool) -> Status {
///     if block_was_silent { Status::Sleep } else { Status::Continue }
/// }
/// assert_eq!(after(true), Status::Sleep);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Keep processing, whatever the input.
    Continue,
    /// Keep processing while the input or the output is not silent.
    ContinueIfNotQuiet,
    /// Keep processing for the plugin's [`tail`](crate::Plugin::tail) once
    /// the input is silent.
    Tail,
    /// Nothing to do until input or events arrive again.
    Sleep,
}

impl<'a, 'b> Block<'a, 'b> {
    /// Creates a block over the given buffers, for an exporter or a test that
    /// drives a processor.
    pub fn new(
        channels: &'a mut [&'b mut [f32]],
        frames: usize,
        events: &'a EventList,
        out_events: &'a mut EventList,
    ) -> Self {
        Block {
            channels,
            frames,
            events,
            out_events,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_processor_sends_midi_through_its_block() {
        use crate::Processor;

        struct Echo;
        impl Processor for Echo {
            fn process(&mut self, block: Block<'_, '_>) -> Status {
                for &event in block.events.as_slice() {
                    block.out_events.push(event);
                }
                for channel in block.channels.iter_mut() {
                    channel[..block.frames].fill(0.0);
                }
                Status::Sleep
            }
        }

        let mut events = EventList::with_capacity(2);
        // The first packet of a MIDI-CI Discovery inquiry, as SysEx7.
        events.push(Event {
            offset: 3,
            words: [0x3016_7E7F, 0x0D70_0201, 0, 0],
        });
        let mut out = EventList::with_capacity(1);
        let mut left = [1.0_f32; 4];
        let mut channels = [&mut left[..]];
        let status = Echo.process(Block::new(&mut channels, 4, &events, &mut out));
        assert_eq!(status, Status::Sleep);
        assert_eq!(out.as_slice(), events.as_slice());
        assert_eq!(left, [0.0; 4]);
    }
}
