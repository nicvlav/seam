//! An editor as an exporter drives it, for one of the host's views: open,
//! close, size, scale and tick, with what a host asks from inside another of
//! the editor's calls put off until that call returns.
//!
//! A toolkit may run the thread's message loop inside a call, a WebView's
//! start-up inside `open` among them, and the host's messages then arrive
//! inside it: a resize, a removal, even a tick. The editor is borrowed for
//! each call, so a call that finds it borrowed never borrows it twice. A tick
//! skips it. A size or a scale is kept and told once the call returns, and a
//! close is kept and done then.
//!
//! An open keeps the plugin's module loaded until the process ends, since a
//! toolkit may leave the host something that points into it and a host may
//! unload the module and load it again elsewhere.

use std::cell::{Cell, RefCell};
use std::error::Error;
use std::panic::{self, AssertUnwindSafe};

use super::timer::keep_module_loaded;
use crate::{Editor, ParentWindow, Size};

/// Where an editor is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Not open: before the first open, after a close, or after an open
    /// that failed.
    Closed,
    /// Inside the editor's `open`.
    Opening,
    Open,
}

/// What an attempt to open the editor came to.
#[derive(Debug)]
pub(crate) enum Opened {
    /// It opened. A close the host asked for while it opened has closed it
    /// again since.
    Yes,
    /// It was open or opening already.
    Already,
    /// The editor refused, with its reason. It is closed.
    Failed(Box<dyn Error>),
    /// The editor panicked inside `open`. It is closed.
    Panicked,
}

/// One view's editor and what the host last gave it.
pub(crate) struct EditorSlot {
    editor: RefCell<Box<dyn Editor>>,
    phase: Cell<Phase>,
    /// A close the host asked for while the editor was in a call.
    closing: Cell<bool>,
    /// The size and scale the host gave last.
    size: Cell<Size>,
    scale: Cell<f64>,
    /// What the editor was last told of them; `None` before it was told.
    told_size: Cell<Option<Size>>,
    told_scale: Cell<Option<f64>>,
}

impl EditorSlot {
    /// Holds `editor`, closed, at the size it opens at and a scale of 1.
    pub(crate) fn new(editor: Box<dyn Editor>) -> Self {
        let size = editor.size();
        EditorSlot {
            editor: RefCell::new(editor),
            phase: Cell::new(Phase::Closed),
            closing: Cell::new(false),
            size: Cell::new(size),
            scale: Cell::new(1.0),
            told_size: Cell::new(None),
            told_scale: Cell::new(None),
        }
    }

    /// The size the host gave last, or the one it opens at.
    pub(crate) fn size(&self) -> Size {
        self.size.get()
    }

    /// The scale the host gave last, 1 until it gives one.
    pub(crate) fn scale(&self) -> f64 {
        self.scale.get()
    }

    /// Whether it is open, or opening.
    pub(crate) fn is_open(&self) -> bool {
        self.phase.get() != Phase::Closed
    }

    /// Opens the editor inside `parent`, after telling it the size and
    /// scale the host gave so far. A panic inside the editor's `open` is
    /// caught and answered as [`Opened::Panicked`].
    ///
    /// Before any of the editor's code runs, it keeps the module loaded until
    /// the process ends, whatever the open comes to: on Windows, what an
    /// editor registers with the system stays valid for the life of the
    /// process.
    pub(crate) fn open(&self, parent: &ParentWindow) -> Opened {
        if self.phase.get() != Phase::Closed {
            return Opened::Already;
        }
        let Ok(mut editor) = self.editor.try_borrow_mut() else {
            return Opened::Already;
        };
        // Before the editor is told anything or opened: an open that fails or
        // panics may still have left the host something that points into the
        // module.
        keep_module_loaded();
        self.phase.set(Phase::Opening);
        self.closing.set(false);
        let opened = panic::catch_unwind(AssertUnwindSafe(|| {
            self.tell(&mut **editor);
            editor.open(parent)
        }));
        match opened {
            Ok(Ok(())) => {
                self.phase.set(Phase::Open);
                self.catch_up(&mut **editor);
                Opened::Yes
            }
            Ok(Err(error)) => {
                self.phase.set(Phase::Closed);
                Opened::Failed(error)
            }
            Err(_) => {
                self.phase.set(Phase::Closed);
                Opened::Panicked
            }
        }
    }

    /// Closes the editor if it is open, or once the call it is in returns.
    pub(crate) fn close(&self) {
        if self.phase.get() == Phase::Closed {
            return;
        }
        match self.editor.try_borrow_mut() {
            Ok(mut editor) if self.phase.get() == Phase::Open => {
                self.phase.set(Phase::Closed);
                editor.close();
            }
            _ => self.closing.set(true),
        }
    }

    /// Takes the size the host gave, telling an open editor now or once the
    /// call it is in returns, and a closed one at its next open.
    pub(crate) fn set_size(&self, size: Size) {
        self.size.set(size);
        self.tell_now();
    }

    /// Takes the scale the host gave, as [`set_size`](Self::set_size) takes
    /// a size. Returns whether it changed.
    pub(crate) fn set_scale(&self, scale: f64) -> bool {
        let changed = self.scale.replace(scale) != scale;
        self.tell_now();
        changed
    }

    /// Ticks the editor if it is open and in no call.
    pub(crate) fn tick(&self) {
        if self.phase.get() != Phase::Open {
            return;
        }
        if let Ok(mut editor) = self.editor.try_borrow_mut() {
            editor.tick();
            self.catch_up(&mut **editor);
        }
    }

    /// Tells an open editor in no call what the host gave.
    fn tell_now(&self) {
        if self.phase.get() != Phase::Open {
            return;
        }
        if let Ok(mut editor) = self.editor.try_borrow_mut() {
            self.catch_up(&mut **editor);
        }
    }

    /// Does what the host asked for while the editor was in a call: a
    /// close, or a size and a scale it was not told yet. Each of these
    /// calls may bring another, so it goes on until none is left.
    fn catch_up(&self, editor: &mut dyn Editor) {
        loop {
            if self.closing.take() {
                self.phase.set(Phase::Closed);
                editor.close();
                return;
            }
            if !self.tell(editor) {
                return;
            }
        }
    }

    /// Tells the editor the scale, then the size, where it was not told
    /// them yet. Returns whether it told it anything.
    fn tell(&self, editor: &mut dyn Editor) -> bool {
        let (scale, size) = (self.scale.get(), self.size.get());
        let mut told = false;
        if self.told_scale.get() != Some(scale) {
            self.told_scale.set(Some(scale));
            editor.set_scale(scale);
            told = true;
        }
        if self.told_size.get() != Some(size) {
            self.told_size.set(Some(size));
            editor.set_size(size);
            told = true;
        }
        told
    }
}

impl std::fmt::Debug for EditorSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditorSlot")
            .field("phase", &self.phase.get())
            .field("size", &self.size.get())
            .field("scale", &self.scale.get())
            .finish_non_exhaustive()
    }
}
