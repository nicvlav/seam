//! The host's side of an editor's view: asking for one, attaching it to a
//! window, sizing and scaling it, and a frame that records what the plugin
//! asks of it.

use std::cell::RefCell;
use std::ffi::{CStr, c_void};

use ::vst3::Steinberg::Vst::IEditControllerTrait;
use ::vst3::Steinberg::{
    IPlugFrame, IPlugFrameTrait, IPlugView, IPlugViewContentScaleSupport,
    IPlugViewContentScaleSupportTrait, IPlugViewTrait, ViewRect, kInvalidArgument,
    kPlatformTypeHWND, kResultOk, kResultTrue, tresult,
};
use ::vst3::{Class, ComPtr, ComRef, ComWrapper};

use super::{Refused, TestHost};
use crate::Plugin;

impl<P: Plugin> TestHost<P> {
    /// Asks the plugin for its editor's view, as a host does when the user
    /// opens the editor, and gives it a frame: `None` for a plugin with no
    /// editor.
    ///
    /// The view keeps the instance alive, so it may outlive this host.
    pub fn view(&self) -> Option<TestView> {
        // SAFETY: the component is alive while this host holds its
        // interfaces; the name is a terminated string valid for the call,
        // and the view comes with the one reference the result holds.
        let view = unsafe { ComPtr::from_raw(self.controller.createView(c"editor".as_ptr())) }?;
        let frame = ComWrapper::new(Frame::default());
        let pointer = frame.as_com_ref::<IPlugFrame>()?.as_ptr();
        // SAFETY: the view was just made; `pointer` is a live frame, and the
        // view keeps its own reference to it.
        if unsafe { view.setFrame(pointer) } != kResultOk {
            return None;
        }
        Some(TestView {
            scale: view.cast::<IPlugViewContentScaleSupport>(),
            view,
            frame,
        })
    }
}

/// An editor's view, as a host holds one: made by [`TestHost::view`], with a
/// frame that records the sizes the plugin asks for and, as a host does,
/// passes each back to the view's `onSize`.
///
/// A view's sizes are in the screen's pixels: an editor's logical size times
/// the scale the host gave.
///
/// # Examples
///
/// ```
#[doc = include_str!("../../../doc/editor.rs")]
/// use seam_host::vst3::TestHost;
///
/// let host = TestHost::<Panel>::new();
/// let view = host.view().expect("the plugin has an editor");
/// assert_eq!(view.size(), Some((320, 200)));
///
/// // A screen at 150 %: the view asks the host for its size there.
/// view.set_scale(1.5)?;
/// view.attach(1)?; // the panel never touches the window
/// view.set_scale(2.0)?;
/// assert_eq!(view.resizes(), [(640, 400)]);
/// assert_eq!(view.size(), Some((640, 400)));
/// view.remove()?;
/// # Ok::<(), seam_host::vst3::Refused>(())
/// ```
pub struct TestView {
    view: ComPtr<IPlugView>,
    scale: Option<ComPtr<IPlugViewContentScaleSupport>>,
    frame: ComWrapper<Frame>,
}

impl TestView {
    /// Returns whether the view attaches to the platform's window type
    /// `platform`, such as `c"HWND"`.
    pub fn supports(&self, platform: &CStr) -> bool {
        // SAFETY: the view is alive while this holds it; the name is a
        // terminated string valid for the call.
        unsafe { self.view.isPlatformTypeSupported(platform.as_ptr()) == kResultTrue }
    }

    /// Attaches the view to `window`, a Windows window handle, as a host
    /// does once it has made the window: the editor opens in it.
    ///
    /// A test whose editor never touches the window may pass any number but
    /// 0; one that opens a real editor passes a [`HiddenWindow`]'s.
    ///
    /// # Errors
    ///
    /// The [`Refused`] answer: the editor did not open.
    pub fn attach(&self, window: isize) -> Result<(), Refused> {
        // SAFETY: the view is alive while this holds it. The handle is the
        // test's: a window that lives until `remove`, or a number its editor
        // never touches.
        Refused::check(unsafe { self.view.attached(window as *mut c_void, kPlatformTypeHWND) })
    }

    /// Removes the view from its window, as a host does before closing it.
    ///
    /// # Errors
    ///
    /// The [`Refused`] answer.
    pub fn remove(&self) -> Result<(), Refused> {
        // SAFETY: the view is alive while this holds it.
        Refused::check(unsafe { self.view.removed() })
    }

    /// Returns the view's size as `getSize` answers it, width and height in
    /// the screen's pixels.
    pub fn size(&self) -> Option<(i32, i32)> {
        let mut rect = ViewRect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        // SAFETY: the view is alive while this holds it; `rect` is valid
        // for writing during the call.
        let answer = unsafe { self.view.getSize(&mut rect) };
        (answer == kResultOk).then(|| (rect.right - rect.left, rect.bottom - rect.top))
    }

    /// Resizes the view as a host does when the user drags its window's
    /// edge: `checkSizeConstraint`, then `onSize`, in the screen's pixels.
    ///
    /// # Errors
    ///
    /// The [`Refused`] answer of the first call that refused.
    pub fn resize(&self, width: i32, height: i32) -> Result<(), Refused> {
        let mut rect = ViewRect {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        // SAFETY: the view is alive while this holds it; `rect` is valid
        // for the calls.
        unsafe {
            if self.view.checkSizeConstraint(&mut rect) != kResultTrue {
                return Err(Refused(kInvalidArgument));
            }
            Refused::check(self.view.onSize(&mut rect))
        }
    }

    /// Returns whether the view says its window may be resized.
    pub fn can_resize(&self) -> bool {
        // SAFETY: the view is alive while this holds it.
        unsafe { self.view.canResize() == kResultTrue }
    }

    /// Gives the view the screen's scale, as a host does on Windows when
    /// the window opens or moves to another screen.
    ///
    /// # Errors
    ///
    /// The [`Refused`] answer, or `kInvalidArgument` for a view with no
    /// content scale support.
    pub fn set_scale(&self, scale: f32) -> Result<(), Refused> {
        let Some(support) = &self.scale else {
            return Err(Refused(kInvalidArgument));
        };
        // SAFETY: the view is alive while this holds it.
        Refused::check(unsafe { support.setContentScaleFactor(scale) })
    }

    /// Returns the sizes the view asked its frame for since the last call,
    /// width and height in the screen's pixels.
    pub fn resizes(&self) -> Vec<(i32, i32)> {
        std::mem::take(&mut *self.frame.resizes.borrow_mut())
    }

    /// The view's interface, for the exporter's own tests.
    #[cfg(test)]
    pub(crate) fn interface(&self) -> &ComPtr<IPlugView> {
        &self.view
    }
}

impl std::fmt::Debug for TestView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestView").finish_non_exhaustive()
    }
}

/// The host's frame around a view: it records each size the view asks for,
/// then gives it to the view's `onSize`, as a host resizes the window.
#[derive(Default)]
struct Frame {
    resizes: RefCell<Vec<(i32, i32)>>,
}

impl Class for Frame {
    type Interfaces = (IPlugFrame,);
}

impl IPlugFrameTrait for Frame {
    unsafe fn resizeView(&self, view: *mut IPlugView, size: *mut ViewRect) -> tresult {
        // SAFETY: the plugin's contract: `size` is valid for the call.
        let Some(rect) = (unsafe { size.as_mut() }) else {
            return kInvalidArgument;
        };
        self.resizes
            .borrow_mut()
            .push((rect.right - rect.left, rect.bottom - rect.top));
        // SAFETY: the plugin's contract: `view` is the view asking, alive
        // for the call.
        match unsafe { ComRef::from_raw(view) } {
            // SAFETY: as above; `rect` is valid for the call.
            Some(view) => unsafe { view.onSize(rect) },
            None => kInvalidArgument,
        }
    }
}

/// A window of the test's own that is never shown: a real editor opens in
/// it with no window on the screen, no focus taken and no input sent.
/// Windows only.
///
/// It is destroyed when dropped, on the thread that made it.
///
/// # Examples
///
/// ```
#[doc = include_str!("../../../doc/editor.rs")]
/// # #[cfg(windows)]
/// # {
/// use seam_host::vst3::{HiddenWindow, TestHost};
///
/// let host = TestHost::<Panel>::new();
/// let view = host.view().expect("the plugin has an editor");
/// let window = HiddenWindow::new().expect("a window");
/// view.attach(window.handle())?;
/// view.remove()?;
/// # }
/// # Ok::<(), seam_host::vst3::Refused>(())
/// ```
#[cfg(windows)]
#[derive(Debug)]
pub struct HiddenWindow {
    window: windows_sys::Win32::Foundation::HWND,
}

#[cfg(windows)]
impl HiddenWindow {
    /// Makes the window: a top-level one of 800 by 600 pixels that is never
    /// shown. `None` when the system refuses one.
    pub fn new() -> Option<Self> {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, WS_CLIPCHILDREN, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
            WS_OVERLAPPEDWINDOW,
        };

        // SAFETY: a window of the system's own static class, with a static
        // name, no parent, menu or creation data; it is never shown, since
        // no style makes it visible.
        let window = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                windows_sys::w!("STATIC"),
                windows_sys::w!("seam test window"),
                WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                0,
                0,
                800,
                600,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        (!window.is_null()).then_some(HiddenWindow { window })
    }

    /// Returns its handle, for [`TestView::attach`].
    pub fn handle(&self) -> isize {
        self.window as isize
    }
}

#[cfg(windows)]
impl Drop for HiddenWindow {
    fn drop(&mut self) {
        // SAFETY: the window this made, on the thread that made it: the
        // type is neither `Send` nor `Sync`, as a raw handle is not.
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow(self.window) };
    }
}
