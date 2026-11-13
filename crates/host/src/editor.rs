//! A plugin's editor, as an exporter opens it inside the host's window: the
//! [`Editor`] trait, which knows no toolkit, and its [`Size`].

use std::error::Error;

use crate::ParentWindow;

/// A size in logical pixels: at a scale of 1, as many as the screen's; at 2,
/// each is two by two of them.
///
/// # Examples
///
/// ```
/// use seam_host::Size;
///
/// let size = Size { width: 480, height: 320 };
/// // At a scale of 1.5 the host's window is this many of the screen's pixels.
/// let scale = 1.5;
/// let screen = (f64::from(size.width) * scale, f64::from(size.height) * scale);
/// assert_eq!(screen, (720.0, 480.0));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Size {
    /// The width.
    pub width: u32,
    /// The height.
    pub height: u32,
}

/// A plugin's editor: what it shows inside the window a host makes for it.
///
/// A plugin hands one out from [`Plugin::editor`](crate::Plugin::editor),
/// once for each view a host asks for. The exporter then drives it, and the
/// plugin writes nothing more: no pump and no timer. In order:
///
/// 1. [`size`](Self::size), once: the size the host's window is made for.
/// 2. [`set_scale`](Self::set_scale) and [`set_size`](Self::set_size), with
///    what the host gave so far, then [`open`](Self::open) with the host's
///    window. An error there leaves nothing in the window and is a fault of
///    the instance, with its text in the fault report; the host is told the
///    editor did not open, and the audio goes on.
/// 3. While open, [`set_size`](Self::set_size) and
///    [`set_scale`](Self::set_scale) as the host resizes the window or moves
///    it to a screen of another scale, and [`tick`](Self::tick) on each of
///    the plugin's main-thread ticks, after
///    [`Plugin::idle`](crate::Plugin::idle).
/// 4. [`close`](Self::close) when the host removes it, or when the view goes
///    while open. Another [`open`](Self::open) may follow, with a new window.
///
/// The host may call back into the exporter from inside any of these, while
/// the editor is still in the call. The exporter never calls into an editor
/// that is in a call already. A size, a scale or a close that arrives then
/// waits until the call returns, so a close the host asks for while
/// [`open`](Self::open) runs comes right after it.
///
/// Thread: main. An editor is not `Send`: every call, and its drop, is on
/// the plugin's main thread.
///
/// # Examples
///
/// An editor that keeps the size it is given, and counts its ticks:
///
/// ```
/// use std::error::Error;
///
/// use seam_host::{Editor, ParentWindow, Size};
///
/// #[derive(Default)]
/// struct Counter {
///     size: Option<Size>,
///     ticks: u32,
/// }
///
/// impl Editor for Counter {
///     fn size(&self) -> Size {
///         Size { width: 400, height: 300 }
///     }
///
///     fn open(&mut self, _parent: &ParentWindow) -> Result<(), Box<dyn Error>> {
///         // A real editor makes its window inside the parent here.
///         Ok(())
///     }
///
///     fn close(&mut self) {}
///
///     fn set_size(&mut self, size: Size) {
///         self.size = Some(size);
///     }
///
///     fn set_scale(&mut self, _scale: f64) {}
///
///     fn tick(&mut self) {
///         self.ticks += 1;
///     }
/// }
///
/// let mut editor = Counter::default();
/// editor.set_size(Size { width: 600, height: 450 });
/// editor.tick();
/// assert_eq!((editor.size, editor.ticks), (Some(Size { width: 600, height: 450 }), 1));
/// ```
pub trait Editor {
    /// Returns the size it opens at, in logical pixels. Read once, before
    /// the host makes its window.
    fn size(&self) -> Size;

    /// Opens inside `parent`, the window the host made for it, which is
    /// valid until [`close`](Self::close) and may be borrowed only during
    /// this call.
    ///
    /// # Errors
    ///
    /// Whatever kept it from opening, such as a toolkit the system lacks. It
    /// leaves nothing inside `parent`, and [`close`](Self::close) is not
    /// called after it.
    fn open(&mut self, parent: &ParentWindow) -> Result<(), Box<dyn Error>>;

    /// Closes: whatever [`open`](Self::open) made inside the parent goes.
    fn close(&mut self);

    /// Takes the size the host gave the window, in logical pixels.
    fn set_size(&mut self, size: Size);

    /// Takes the host's scale for the window: 1 at the system's base
    /// density, 1.5 at 150 %.
    fn set_scale(&mut self, scale: f64);

    /// Handles one main-thread tick while open, after the plugin's
    /// [`idle`](crate::Plugin::idle), so it shows what that tick changed.
    fn tick(&mut self);
}
