//! Faults: what the exporters catch where a host calls in, and what an
//! instance's own state did wrong with no host's call to blame, each of the
//! [`Fault`] kinds, counted for the process and for each instance.
//!
//! To a user each looks like a plugin gone quiet, or a project that does not
//! come back as it was saved, so each is made visible.
//! The rule in full, which other docs point to:
//!
//! - **Counting** is wait-free and safe on any thread, the audio thread
//!   included. Every fault counts for the process ([`boundary_faults`]),
//!   and an instance's for the instance too.
//! - **Reporting** writes a line: to an attached debugger on Windows, to
//!   stderr elsewhere. Writing is a system call, so Seam never writes on the
//!   audio thread.
//! - **An instance's fault** is only recorded where it happens. The first
//!   of each kind is reported from the instance's next tick, on the main
//!   thread, or from its release; later ones are counted, not reported.
//!   A release reports on its own thread: the main thread for a release as
//!   a host should make it, another for one that is itself a fault. A
//!   release from a host callback inside a tick reports nothing: the
//!   instance leaks, and a kind first recorded during that tick is never
//!   written.
//! - **A refused load** is not a fault: it counts nowhere, and the host's
//!   answer is the same refusal. The first one of an instance writes a line
//!   with the reason, on the thread the host loaded on, never the audio
//!   thread's; later ones write nothing.
//! - **A fault outside any instance** is reported at once, on the thread it
//!   happens on, the first time its kind happens in the process. The calls
//!   that catch one are not the audio thread's.
//! - **A plugin's refused declarations** fail the instance's creation, as a
//!   panic outside any instance. Each such refusal also writes a line with
//!   the plugin's name and the reason, on the thread that tried to create
//!   it: no instance exists to report it once.
//!
//! The panic itself is another matter. A processor that panics does so on
//! the audio thread. There the panic allocates its payload, and std's
//! default panic hook writes the message to stderr under its lock. A debug
//! build's failed assertion is a panic too. Seam installs no hook to stop
//! that: the hook is the whole process's, and belongs to the host.

use std::error::Error;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

/// Every fault the exporters caught, process-wide.
static TOTAL: AtomicU64 = AtomicU64::new(0);

/// The kinds reported for faults outside any instance.
static REPORTED: AtomicU8 = AtomicU8::new(0);

/// Returns how many faults the exporters have recorded in this process, on
/// every thread and in every instance.
///
/// The faults include panics that would have unwound into the host, calls
/// refused for the thread they arrived on, and a save that could not write
/// every parameter made at run time as the plugin holds them. Nothing in a
/// working plugin moves the count. In a host, a count that moved is the
/// first thing to check when a plugin goes quiet or a project does not come
/// back as it was saved. Each instance also writes the first fault of each
/// kind as a line of text, to an attached debugger on Windows and to stderr
/// elsewhere.
///
/// # Examples
///
/// ```
#[doc = include_str!("../doc/gain.rs")]
/// // The C interface's test host, which needs the `cabi` and `test-host`
/// // features.
/// # #[cfg(all(feature = "cabi", feature = "test-host"))]
/// # {
/// use seam_host::boundary_faults;
/// use seam_host::cabi::TestHost;
///
/// // `Gain` is a plugin with one parameter, whose `idle` panics while the
/// // static `BREAK` is set.
/// let before = boundary_faults();
/// let host = TestHost::<Gain>::new();
/// assert_eq!(boundary_faults(), before, "a working plugin moves nothing");
///
/// BREAK.store(true, Ordering::Relaxed); // the plugin's next tick panics
/// host.expect_faults(1);
/// host.idle(); // the interface catches the panic and counts it
/// assert_eq!(boundary_faults(), before + 1);
/// # }
/// ```
#[cfg_attr(docsrs, doc(cfg(any(feature = "vst3", feature = "cabi"))))]
#[cfg_attr(
    not(any(feature = "vst3", feature = "cabi")),
    expect(unreachable_pub, reason = "an exporter re-exports it at the root")
)]
pub fn boundary_faults() -> u64 {
    TOTAL.load(Ordering::Relaxed)
}

/// What went wrong: at the boundary, or in what an instance saves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fault {
    /// A host's call panicked inside and was answered as refused.
    Panic,
    /// The processor panicked: blocks play silence until the next
    /// activation.
    AudioPanic,
    /// A main-thread call arrived on a thread that is not the instance's
    /// main thread, and was refused. A release refused so leaks the
    /// instance.
    WrongThread,
    /// A main-thread call to an instance bound to no thread found a call on
    /// another thread holding the plugin, and was refused once its wait ran
    /// out. A release refused so leaks the instance.
    HeldElsewhere,
    /// A host's call arrived while the plugin was inside one of its own
    /// calls to the host, and was refused.
    Reentered,
    /// No main-thread timer started: the system refused one, or the
    /// instance has no thread of its own to run one on.
    NoTimer,
    /// Two parameters the plugin made at run time, and still holds, were
    /// declared under one path; a save writes only the later.
    SharedPath,
    /// A save left out parameters made at run time: the block had no room
    /// for them.
    NoRoom,
}

impl Fault {
    /// Every kind, each once, in the order a report lists them. Eight fill
    /// the `u8`s the kinds are kept in; a ninth widens them.
    const ALL: [Fault; 8] = [
        Fault::Panic,
        Fault::AudioPanic,
        Fault::WrongThread,
        Fault::HeldElsewhere,
        Fault::Reentered,
        Fault::NoTimer,
        Fault::SharedPath,
        Fault::NoRoom,
    ];

    fn bit(self) -> u8 {
        1 << self as u8
    }

    fn what(self) -> &'static str {
        match self {
            Fault::Panic => "a call from the host panicked and was answered as refused",
            Fault::AudioPanic => {
                "the processor panicked; it plays silence until the host activates it again"
            }
            Fault::WrongThread => {
                "a call that must run on the plugin's main thread arrived on another thread \
                 and was refused, or the instance was released there and leaked"
            }
            Fault::HeldElsewhere => {
                "a call waited too long for a call on another thread to let the plugin go and \
                 was refused, or the instance was released while one held it and leaked"
            }
            Fault::Reentered => {
                "a call arrived while the plugin was busy inside one of its own calls to the \
                 host, and was refused"
            }
            Fault::NoTimer => {
                "a main-thread timer did not start (the system refused one, or the instance has \
                 no thread of its own), so it never ticks the plugin"
            }
            Fault::SharedPath => {
                "two parameters made at run time were declared under one path; a save writes \
                 only the later"
            }
            Fault::NoRoom => {
                "a save left out parameters made at run time: the saved block had no room for them"
            }
        }
    }
}

// Ties `ALL` to the enum. The match is exhaustive only while `ALL` holds
// every kind, so a kind added to the enum fails to compile here until it is
// listed. A kind listed twice makes a pattern unreachable, which the lints
// refuse. The copy's length makes `ALL` fail to compile here when it grows,
// until the match reads its new place too. The assertion keeps each kind's
// bit inside the `u8`s that hold them.
const _: () = {
    const ALL: [Fault; 8] = Fault::ALL;
    const FIRST: Fault = ALL[0];
    const SECOND: Fault = ALL[1];
    const THIRD: Fault = ALL[2];
    const FOURTH: Fault = ALL[3];
    const FIFTH: Fault = ALL[4];
    const SIXTH: Fault = ALL[5];
    const SEVENTH: Fault = ALL[6];
    const EIGHTH: Fault = ALL[7];
    assert!(Fault::ALL.len() <= u8::BITS as usize);
    match Fault::Panic {
        FIRST | SECOND | THIRD | FOURTH | FIFTH | SIXTH | SEVENTH | EIGHTH => {}
    }
};

/// Counts a fault for the process, and on this thread in tests.
fn count() {
    TOTAL.fetch_add(1, Ordering::Relaxed);
    #[cfg(test)]
    tests::counted();
}

/// Records a fault outside any instance, and reports its kind the first
/// time it happens in the process. The caller is not the audio thread.
pub(crate) fn record_fault(fault: Fault) {
    count();
    if REPORTED.fetch_or(fault.bit(), Ordering::Relaxed) & fault.bit() == 0 {
        write_line(&format!("seam: {}.", fault.what()));
    }
}

/// Writes one line for a plugin whose declarations were refused, naming
/// `plugin` and the reason, `error`. The instance is not built: the caller
/// panics next, and the shell's guard counts that. The caller is not the
/// audio thread.
pub(crate) fn report_refused_declarations(plugin: &str, error: &dyn Error) {
    write_line(&format!(
        "seam: {plugin}: no instance was built, since a declaration was refused: {error}."
    ));
}

/// One instance's faults: how many, from every thread, and which kinds are
/// waiting to be reported.
#[derive(Debug, Default)]
pub(crate) struct Faults {
    count: AtomicU64,
    /// Kinds recorded and not yet reported.
    pending: AtomicU8,
    /// Kinds reported already: each is reported once.
    reported: AtomicU8,
    /// Whether a refused load was reported already: only the first is.
    refused_load: AtomicBool,
}

impl Faults {
    /// Records a fault. Wait-free, and safe on any thread, the audio thread
    /// included: it writes nothing.
    pub(crate) fn record(&self, fault: Fault) {
        count();
        self.count.fetch_add(1, Ordering::Relaxed);
        self.pending.fetch_or(fault.bit(), Ordering::Release);
    }

    /// Runs an entry point's body for this instance, answering `refused`
    /// if it panics: a panic must never unwind into the host, which would
    /// abort it.
    pub(crate) fn guard<R>(&self, refused: R, body: impl FnOnce() -> R) -> R {
        panic::catch_unwind(AssertUnwindSafe(body)).unwrap_or_else(|_| {
            self.record(Fault::Panic);
            refused
        })
    }

    /// Whether a fault is waiting to be reported.
    pub(crate) fn pending(&self) -> bool {
        self.pending.load(Ordering::Relaxed) != 0
    }

    /// How many faults the instance has had, on every thread.
    #[cfg(any(test, feature = "test-host"))]
    pub(crate) fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    /// Writes one line for each kind recorded since the last call that was
    /// never reported for this instance; `who` names the instance. The
    /// caller is not the audio thread.
    pub(crate) fn report(&self, who: &str) {
        let pending = self.pending.swap(0, Ordering::Acquire);
        let new = pending & !self.reported.fetch_or(pending, Ordering::Relaxed);
        for fault in Fault::ALL
            .into_iter()
            .filter(|fault| new & fault.bit() != 0)
        {
            write_line(&format!(
                "seam: {who}: {}. Later faults of this kind in this instance are counted, \
                 not reported.",
                fault.what()
            ));
        }
    }

    /// Writes one line for the first refused load, with the reason: the
    /// error's message, then each source's. A refused load is not a fault, so
    /// nothing is counted. `who` names the instance, only when a line is
    /// written. The caller is not the audio thread.
    pub(crate) fn report_refused_load(&self, who: impl FnOnce() -> String, error: &dyn Error) {
        if self.refused_load.swap(true, Ordering::Relaxed) {
            return;
        }
        let mut reason = error.to_string();
        let mut source = error.source();
        while let Some(cause) = source {
            reason.push_str(": ");
            reason.push_str(&cause.to_string());
            source = cause.source();
        }
        write_line(&format!(
            "seam: {}: the host's saved state was refused: {reason}.",
            who()
        ));
    }
}

/// Writes one diagnostic line where a developer looks: an attached
/// debugger on Windows, stderr elsewhere. In this crate's tests, a list the
/// test reads.
fn write_line(line: &str) {
    #[cfg(test)]
    tests::wrote(line);
    #[cfg(all(not(test), windows, not(miri)))]
    debugger::write(line);
    #[cfg(all(not(test), not(all(windows, not(miri)))))]
    {
        use std::io::Write as _;
        // No other place to report a failed write, and a panic here would
        // unwind into the host.
        let _ = writeln!(std::io::stderr(), "{line}");
    }
}

#[cfg(all(not(test), windows, not(miri)))]
mod debugger {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OutputDebugStringW(text: *const u16);
    }

    /// Hands `line` to an attached debugger, as Windows plugins report.
    pub(super) fn write(line: &str) {
        let text: Vec<u16> = line.encode_utf16().chain(['\n' as u16, 0]).collect();
        // SAFETY: `text` is a NUL-terminated UTF-16 string that outlives the
        // call, which only reads it.
        unsafe { OutputDebugStringW(text.as_ptr()) };
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use tests::thread_faults;
#[cfg(all(test, any(feature = "vst3", feature = "cabi")))]
pub(crate) use tests::written;
