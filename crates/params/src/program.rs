//! Writes from any thread but audio: syncs and restores.

use std::sync::Arc;

use crate::{Origin, ParamId, ParamSet, RestoreWriter, audio_thread};

/// The handle for writes from any thread but audio.
///
/// A sync is the plugin's own write that the host should re-read rather
/// than record: a preset recall, a randomise. A restore is a saved
/// state being loaded, written through a [`Load`]. Neither tells the
/// host a gesture, opens one, or records undo.
///
/// # Examples
///
/// ```
/// # use seam_params as sp;
/// # let set = sp::ParamSet::from_decls([
/// #     sp::ParamDecl::new("gain", "Gain", sp::Kind::continuous(-60.0, 12.0, 0.1, None), 0.0),
/// # ])?;
/// # let gain = set.id("gain").expect("declared");
///
/// let program = set.program();
///
/// // A preset recall: the host is asked to re-read, not told a gesture.
/// assert!(program.sync(gain, -6.0));
/// assert!(set.take_rescan());
///
/// // A saved state loads through a `Load`.
/// let load = program.load();
/// load.restore(gain, 3.0);
/// assert_eq!(set.value(gain), 3.0);
/// # Ok::<(), seam_params::DeclError>(())
/// ```
#[derive(Debug, Clone)]
pub struct ProgramParams {
    set: Arc<ParamSet>,
}

impl ProgramParams {
    pub(crate) fn new(set: Arc<ParamSet>) -> Self {
        Self { set }
    }

    /// Returns the set these writes reach.
    pub fn set(&self) -> &ParamSet {
        &self.set
    }

    /// Writes `value`, in real units, as a sync: snapped, stored when it
    /// changes the store, and followed by a request that the host re-read
    /// values, which the exporter takes with [`ParamSet::take_rescan`].
    /// Returns whether it changed the store; a value that is not finite is
    /// refused.
    ///
    /// Thread: any but audio.
    ///
    /// # Examples
    ///
    /// ```
    /// # use seam_params as sp;
    /// # let set = sp::ParamSet::from_decls([
    /// #     sp::ParamDecl::new("gain", "Gain", sp::Kind::continuous(-60.0, 12.0, 0.1, None), 0.0),
    /// # ])?;
    /// # let gain = set.id("gain").expect("declared");
    ///
    /// let program = set.program();
    ///
    /// assert!(program.sync(gain, 3.0));
    /// assert!(!program.sync(gain, 3.0)); // the value did not change
    /// assert!(!program.sync(gain, f32::NAN)); // refused
    /// assert_eq!(set.value(gain), 3.0);
    /// # Ok::<(), seam_params::DeclError>(())
    /// ```
    pub fn sync(&self, id: ParamId, value: f32) -> bool {
        let Some(value) = self.set.decl(id).edit_value(value) else {
            return false;
        };
        if value == self.set.value(id) {
            return false;
        }
        self.set.store_then_publish(id, value, Origin::Program, || {
            self.set.host().request_rescan();
        });
        true
    }

    /// Starts loading a saved state, marking the restore as stored first.
    ///
    /// An undo from then on replays nothing recorded before the load, and
    /// each write through the [`Load`] marks it again, so nothing recorded
    /// between its writes replays over them either. [`Load`] shows one.
    pub fn load(&self) -> Load {
        let clock = self.set.clock().writer();
        clock.stored();
        Load {
            set: Arc::clone(&self.set),
            clock,
        }
    }
}

/// A saved state being loaded, after the restore was marked stored. Each
/// write through it marks the restore stored again, so an undo never writes
/// over a loaded value.
///
/// # Examples
///
/// ```
/// # use seam_params as sp;
/// # let set = sp::ParamSet::from_decls([
/// #     sp::ParamDecl::new("gain", "Gain", sp::Kind::continuous(-60.0, 12.0, 0.1, None), 0.0),
/// # ])?;
/// # let gain = set.id("gain").expect("declared");
///
/// let load = set.program().load();
/// load.restore(gain, 3.0);
/// assert_eq!(set.value(gain), 3.0);
/// # Ok::<(), seam_params::DeclError>(())
/// ```
#[derive(Debug)]
pub struct Load {
    set: Arc<ParamSet>,
    clock: RestoreWriter,
}

impl Load {
    /// Writes `value`, in real units, as a restore, storing what
    /// [`ParamDecl::restore_value`](crate::ParamDecl::restore_value) makes of
    /// it.
    ///
    /// Every call stores and publishes, with no equality check. It waits
    /// while an undo replay is writing, then marks the restore stored
    /// before it stores, so the replay's write never lands after it and the
    /// replay stops before its next one.
    ///
    /// Thread: any but audio.
    ///
    /// # Panics
    ///
    /// In a build with `debug_assertions`, while the thread holds an
    /// [`audio_thread`] mark.
    ///
    /// # Examples
    ///
    /// ```
    /// # use seam_params as sp;
    /// # let set = sp::ParamSet::from_decls([
    /// #     sp::ParamDecl::new("gain", "Gain", sp::Kind::continuous(-60.0, 12.0, 0.1, None), 0.0),
    /// # ])?;
    /// # let gain = set.id("gain").expect("declared");
    ///
    /// let load = set.program().load();
    ///
    /// load.restore(gain, 99.0); // snapped to the range
    /// assert_eq!(set.value(gain), 12.0);
    /// load.restore(gain, f32::NAN); // a corrupt state falls back to the default
    /// assert_eq!(set.value(gain), 0.0);
    /// # Ok::<(), seam_params::DeclError>(())
    /// ```
    pub fn restore(&self, id: ParamId, value: f32) {
        audio_thread::assert_not_here("Load::restore");
        let value = self.set.decl(id).restore_value(value);
        let _held = self.clock.hold_loads();
        // Under the hold a replay step checks the count under, so the step
        // either wrote before this or sees the count moved and writes nothing.
        self.clock.stored();
        self.set.store(id, value, Origin::Restore);
    }
}
