//! The main-thread timer, for a shell whose host gives it no tick of its
//! own. One per instance, on the thread bound as main, until the instance
//! goes.
//!
//! A timer runs its tick inside the panic guard, and only on the thread that
//! started it. Dropping it on that thread stops it at once. A timer that must
//! leak instead, its instance released inside a tick or on another thread, is
//! [`abandon`]ed: from any thread, it calls its tick no more.
//!
//! Without a platform's timer there is none: no [`Timer`] is made, and
//! [`abandon`] has nothing to stop.

/// No timer on this platform.
#[derive(Debug)]
pub(crate) struct Timer;

/// Nothing to stop either.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Handle;

/// Detaches a timer that will never be dropped from its tick, from any
/// thread: for an instance whose main-thread state, timer included, is
/// leaked.
pub(crate) fn abandon(_: Handle) {}
