//! The window a host makes for a plugin's editor, as a safe value.

use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};

/// The host's window an [`Editor`](crate::Editor) opens inside, as a
/// toolkit takes it: through [`HasWindowHandle`].
///
/// Only an exporter makes one, from the window a host hands it, and lends it
/// to [`Editor::open`](crate::Editor::open) for that call alone. It cannot
/// be cloned or kept, so a handle borrowed from it is always one the host
/// keeps valid.
///
/// # Examples
///
/// ```
/// use raw_window_handle::{HasWindowHandle, RawWindowHandle};
/// use seam_host::ParentWindow;
///
/// /// What an editor's `open` reads before it makes its window inside.
/// fn platform(parent: &ParentWindow) -> &'static str {
///     match parent.window_handle().map(|handle| handle.as_raw()) {
///         Ok(RawWindowHandle::Win32(_)) => "a Win32 window",
///         Ok(RawWindowHandle::AppKit(_)) => "an AppKit view",
///         Ok(_) => "another platform's window",
///         Err(_) => "no window",
///     }
/// }
/// ```
#[derive(Debug)]
pub struct ParentWindow {
    raw: RawWindowHandle,
}

impl ParentWindow {
    /// The window `raw` names, which the host keeps valid until it removes
    /// the editor; the exporter lends it to the editor's `open` alone.
    #[cfg_attr(
        not(all(windows, feature = "vst3")),
        expect(
            dead_code,
            reason = "only the VST3 exporter opens an editor, on Windows alone"
        )
    )]
    pub(crate) fn new(raw: RawWindowHandle) -> Self {
        ParentWindow { raw }
    }
}

impl HasWindowHandle for ParentWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        // SAFETY: an exporter makes a `ParentWindow` only from the window a
        // host handed it to attach an editor to, which the host keeps valid
        // until it removes the editor. The exporter makes it inside that
        // call and lends it to the editor's `open` alone; it is not `Clone`,
        // and nothing outside this crate can make one. So the window is
        // valid for every borrow of `self`, and the handle borrows `self`.
        Ok(unsafe { WindowHandle::borrow_raw(self.raw) })
    }
}
