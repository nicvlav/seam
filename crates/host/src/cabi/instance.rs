//! One instance behind the C interface: the plugin's [`Engine`], linked to
//! the host's callbacks, with the interface's block and status codes.

use seam_params::{HostEvent, ParamId, ParamSet};

use super::abi::{
    RESTART_LATENCY, RESTART_VALUES, STATUS_CONTINUE, STATUS_CONTINUE_IF_NOT_QUIET, STATUS_SLEEP,
    STATUS_TAIL, SeamHost, SeamProcess,
};
use crate::Status;
use crate::engine::{Engine, HostLink, Restart};

/// An instance as `create` makes it: the engine, linked to the host's
/// callbacks.
pub(super) type Instance<P> = Engine<P, Callbacks>;

/// The host's callbacks, as the engine reaches them.
#[derive(Debug, Clone, Copy)]
pub(super) struct Callbacks(pub(super) SeamHost);

// SAFETY: the pointers cross threads unread. The engine calls the callbacks,
// with their context, only inside `idle` or `destroy`, on the thread that
// made that call (`tell` and `restart` below). The header lets it do so.
unsafe impl Send for Callbacks {}

impl HostLink for Callbacks {
    /// The host reads the latency as `activate` returns.
    const ACTIVATION_TELLS_LATENCY: bool = true;
    /// The host writes its automation through `set`, on any thread.
    const QUEUES: bool = false;

    fn tell(&self, _set: &ParamSet, event: HostEvent) {
        let host = self.0;
        let index = |id: ParamId| id.index() as u32;
        // SAFETY: the header's contract for `SeamHost`: the callbacks and
        // their `context` stay valid until `destroy` returns. They may be
        // called inside `idle` or `destroy`, on the thread that made that
        // call, the only places the engine calls this. None unwinds.
        unsafe {
            match event {
                HostEvent::Begin(id) => {
                    if let Some(begin) = host.begin_edit {
                        begin(host.context, index(id));
                    }
                }
                HostEvent::Value { id, normalized } => {
                    if let Some(perform) = host.perform_edit {
                        perform(host.context, index(id), normalized);
                    }
                }
                HostEvent::End(id) => {
                    if let Some(end) = host.end_edit {
                        end(host.context, index(id));
                    }
                }
            }
        }
    }

    fn restart(&self, restart: Restart) {
        let mut flags = 0;
        if restart.values {
            flags |= RESTART_VALUES;
        }
        if restart.latency {
            flags |= RESTART_LATENCY;
        }
        if let Some(callback) = self.0.restart {
            // SAFETY: as in `tell`: a callback the header keeps valid until
            // `destroy` returns, called inside `idle`, on its thread.
            unsafe { callback(self.0.context, flags) };
        }
    }
}

/// The status as the C interface answers it.
pub(super) fn status_code(status: Status) -> u32 {
    match status {
        Status::Continue => STATUS_CONTINUE,
        Status::ContinueIfNotQuiet => STATUS_CONTINUE_IF_NOT_QUIET,
        Status::Tail => STATUS_TAIL,
        Status::Sleep => STATUS_SLEEP,
    }
}

/// A host's list of channel pointers.
///
/// # Safety
///
/// `channels` is null, or points to `count` pointers that stay valid for
/// reads, and unchanged, for `'a`.
pub(super) unsafe fn channel_list<'a>(channels: *const *mut f32, count: u32) -> &'a [*mut f32] {
    if channels.is_null() || count == 0 {
        return &[];
    }
    // SAFETY: the caller's contract, with `channels` non-null and `count`
    // above 0 here.
    unsafe { std::slice::from_raw_parts(channels, count as usize) }
}

/// Zeroes every output channel.
///
/// # Safety
///
/// `block.outputs` is null or holds `output_count` channel pointers, each
/// null or valid for writing `frames` floats until the caller returns, with
/// nothing else reading or writing them meanwhile.
pub(super) unsafe fn silence(block: &SeamProcess) {
    let frames = block.frames as usize;
    // SAFETY: the caller's contract gives `output_count` pointers for the
    // call, as `channel_list` requires.
    for &channel in unsafe { channel_list(block.outputs, block.output_count) } {
        if !channel.is_null() {
            // SAFETY: the caller's contract: a non-null output channel is
            // valid for writing `frames` floats.
            unsafe { std::ptr::write_bytes(channel, 0, frames) };
        }
    }
}
