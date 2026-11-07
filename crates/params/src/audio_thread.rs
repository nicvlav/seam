//! A debug-build mark for "this thread is the audio thread".
//!
//! Some calls must never run on the audio thread, since they lock or
//! allocate, and three types must never be dropped there: [`AudioParams`],
//! [`HostEventDrain`] and learn's `LearnAudio`. They are `Send` and `Sync`, so
//! the compiler cannot say so. Each such call, `Value::set` to
//! `Session::save`, asserts with [`assert_not_here`] that the thread holds no
//! mark from [`enter`].
//!
//! Without `debug_assertions` there is no mark and no check, so the mark
//! proves what the tests run: it catches a locking call made from a test or a
//! debug build of a host, not one that only a release build reaches.
//!
//! [`AudioParams`]: crate::AudioParams
//! [`HostEventDrain`]: crate::HostEventDrain

use std::marker::PhantomData;

#[cfg(debug_assertions)]
thread_local! {
    static DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// The mark [`enter`] makes, held until dropped; without `debug_assertions`
/// it marks nothing.
///
/// Guards nest. It stays on the thread that made it, so it is not `Send`.
///
/// # Examples
///
/// ```
/// use seam_params::audio_thread;
///
/// let outer = audio_thread::enter();
/// let inner = audio_thread::enter();
/// drop(inner);
/// // The outer guard still marks the thread (in a debug build).
/// assert_eq!(audio_thread::is_here(), cfg!(debug_assertions));
/// drop(outer);
/// assert!(!audio_thread::is_here());
/// ```
#[derive(Debug)]
#[must_use = "the mark clears as soon as the guard drops"]
pub struct AudioThreadGuard {
    _one_thread: PhantomData<*const ()>,
}

/// Marks the calling thread as the audio thread until the guard drops.
///
/// An exporter calls it around everything a host may run on the audio
/// thread: its process call, its flush and any other. A plugin never needs
/// it.
///
/// # Examples
///
/// ```
/// use seam_params::audio_thread;
///
/// assert!(!audio_thread::is_here());
/// {
///     let _audio = audio_thread::enter();
///     // The host's process call runs here.
/// }
/// assert!(!audio_thread::is_here());
/// ```
pub fn enter() -> AudioThreadGuard {
    #[cfg(debug_assertions)]
    DEPTH.with(|depth| depth.set(depth.get() + 1));
    AudioThreadGuard {
        _one_thread: PhantomData,
    }
}

impl Drop for AudioThreadGuard {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        DEPTH.with(|depth| depth.set(depth.get() - 1));
    }
}

/// Whether the calling thread is marked as the audio thread. Always false
/// without `debug_assertions`.
///
/// # Examples
///
/// ```
/// use seam_params::audio_thread;
///
/// let _audio = audio_thread::enter();
/// assert_eq!(audio_thread::is_here(), cfg!(debug_assertions));
/// ```
pub fn is_here() -> bool {
    #[cfg(debug_assertions)]
    {
        DEPTH.with(|depth| depth.get() > 0)
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

/// Checks that the calling thread is not the audio thread. `what` names the
/// call, for the message.
///
/// Free of cost and of effect without `debug_assertions`. A thread that is
/// already unwinding is never checked, so a drop that runs during a panic
/// cannot turn it into an abort.
///
/// # Panics
///
/// With `debug_assertions`, if the thread holds an [`AudioThreadGuard`].
///
/// # Examples
///
/// ```
/// use seam_params::audio_thread;
///
/// // A call that locks starts with this line, and passes off the audio thread.
/// audio_thread::assert_not_here("Session::save");
///
/// let _audio = audio_thread::enter();
/// let refused = std::panic::catch_unwind(|| audio_thread::assert_not_here("Session::save"));
/// assert_eq!(refused.is_err(), cfg!(debug_assertions));
/// ```
#[inline(always)]
#[track_caller]
pub fn assert_not_here(what: &str) {
    #[cfg(debug_assertions)]
    if is_here() && !std::thread::panicking() {
        panic!("{what} must not run on the audio thread");
    }
    #[cfg(not(debug_assertions))]
    let _ = what;
}
