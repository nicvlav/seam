//! The main-thread timer, for a shell whose host gives it no tick of its
//! own: VST3 gives a component no idle call without an editor. One per
//! instance, on the thread bound as main, until the instance goes.
//!
//! - Windows: a message-only window, made on the main thread and ticking
//!   through that thread's message loop. The timer asks for 15 ms, which
//!   the system clock's 15.6 ms tick rounds to: about 64 Hz.
//! - macOS: a Core Foundation run loop timer on the main thread's run loop,
//!   in its common modes, about 60 times a second.
//! - Elsewhere there is no timer, and the VST3 exporter refuses to build.
//!
//! Each timer runs its tick inside the panic guard, and only on the thread
//! that started it. Dropping it on that thread stops it at once. A timer
//! that must leak instead, its instance released inside a tick or on
//! another thread, is [`abandon`]ed: from any thread, it calls its tick no
//! more.

#[cfg(all(test, feature = "vst3"))]
pub(crate) use imp::detached;
#[cfg(all(any(test, feature = "test-host"), feature = "vst3"))]
pub(crate) use imp::pump;
#[cfg(feature = "vst3")]
pub(crate) use imp::unregister;
pub(crate) use imp::{Handle, Timer, abandon};

#[cfg(all(windows, feature = "vst3", not(miri)))]
mod imp {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};

    use windows_sys::Win32::Foundation::{
        ERROR_CLASS_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM,
    };
    use windows_sys::Win32::System::LibraryLoader::{
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN,
        GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleHandleExW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetWindowLongPtrW,
        HWND_MESSAGE, KillTimer, RegisterClassW, SetTimer, SetWindowLongPtrW, UnregisterClassW,
        WM_TIMER, WNDCLASSW,
    };
    use windows_sys::core::PCWSTR;

    use crate::boundary::guard;

    /// The window class's name, a static UTF-16 string with its null. The
    /// class is local to this module's DLL.
    const CLASS: PCWSTR = windows_sys::w!("SeamTimer");
    static REGISTERED: AtomicBool = AtomicBool::new(false);
    const ID: usize = 1;
    const PERIOD_MS: u32 = 15;

    /// This DLL's module handle: window classes are local to it, so two
    /// plugins in one host each register their own.
    fn module() -> HINSTANCE {
        module_with(GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT)
    }

    /// This DLL's module handle, found by `flags` and the address of one of
    /// its own functions.
    fn module_with(flags: u32) -> HINSTANCE {
        let mut module: HINSTANCE = std::ptr::null_mut();
        // SAFETY: the address is this module's own function, `flags` either
        // adds no reference or pins the module, and `module` is room for the
        // handle.
        unsafe {
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | flags,
                window_procedure as *const c_void as *const u16,
                &mut module,
            )
        };
        module
    }

    fn register() -> bool {
        if REGISTERED.load(Ordering::Acquire) {
            return true;
        }
        let class = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(window_procedure),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: module(),
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: CLASS,
        };
        // SAFETY: a filled class description whose strings are static.
        let registered = unsafe { RegisterClassW(&class) } != 0
            // SAFETY: reads this thread's last error.
            || unsafe { GetLastError() } == ERROR_CLASS_ALREADY_EXISTS;
        REGISTERED.store(registered, Ordering::Release);
        registered
    }

    /// Unregisters the window class, when the host unloads the module and
    /// every instance is gone.
    pub(crate) fn unregister() {
        if REGISTERED.swap(false, Ordering::AcqRel) {
            // SAFETY: the class this module registered. A dropped timer has
            // destroyed its window; an abandoned one's is never destroyed.
            // While one is left, the call fails and unregisters nothing,
            // which is harmless: that window's module is pinned, so its class
            // and procedure stay mapped.
            unsafe { UnregisterClassW(CLASS, module()) };
        }
    }

    type Tick = Box<dyn Fn()>;

    /// Calls its tick on the thread that started it, through that thread's
    /// message loop, until dropped there.
    pub(crate) struct Timer {
        window: HWND,
        tick: *mut Tick,
    }

    /// A timer's window, which any thread may [`abandon`].
    #[derive(Debug, Clone, Copy)]
    pub(crate) struct Handle(usize);

    /// Detaches a timer that will never be dropped from its tick, from any
    /// thread: for an instance whose main-thread state, timer included, is
    /// leaked.
    ///
    /// The window stops the timer at its next message, which finds no tick,
    /// on the thread that set it. The window itself is never destroyed, and
    /// the module stays loaded until the process ends. Otherwise, once the
    /// host unloads it, the host's message loop could call a window procedure
    /// no longer mapped. That pinning cannot be tested in-process: a test
    /// binary is never unloaded.
    pub(crate) fn abandon(handle: Handle) {
        // The pin's answer goes unchecked. The address is the running
        // module's own function, which the loader finds, and were the pin
        // refused, no safer step is left that works from any thread.
        module_with(GET_MODULE_HANDLE_EX_FLAG_PIN);
        // SAFETY: a window this module made and never destroys; any thread
        // of the process may set its user data, and the procedure reads zero
        // as no tick.
        unsafe { SetWindowLongPtrW(handle.0 as HWND, GWLP_USERDATA, 0) };
    }

    /// Whether the timer's window takes no more of its messages: none is
    /// due after four periods. On the window's thread; for tests.
    #[cfg(test)]
    pub(crate) fn stopped(handle: Handle) -> bool {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, PM_REMOVE, PeekMessageW};

        std::thread::sleep(std::time::Duration::from_millis(u64::from(4 * PERIOD_MS)));
        // SAFETY: all zeros is a valid message, which `PeekMessageW`
        // overwrites.
        let mut message: MSG = unsafe { std::mem::zeroed() };
        // SAFETY: takes a timer message for a window this thread made, if
        // one is due.
        unsafe {
            PeekMessageW(
                &mut message,
                handle.0 as HWND,
                WM_TIMER,
                WM_TIMER,
                PM_REMOVE,
            ) == 0
        }
    }

    /// Whether the timer's window procedure finds no tick: the timer was
    /// dropped, which destroyed its window, or abandoned; for tests.
    #[cfg(test)]
    pub(crate) fn detached(handle: Handle) -> bool {
        // SAFETY: reads the user data of a window this module made, from any
        // thread; a destroyed window's read answers 0, as an abandoned one's.
        unsafe { GetWindowLongPtrW(handle.0 as HWND, GWLP_USERDATA) == 0 }
    }

    impl Timer {
        /// Starts ticking `tick`. `None` when the system refuses a window or
        /// a timer: the instance then never ticks.
        pub(crate) fn start(tick: Tick) -> Option<Timer> {
            Timer::make(tick)
        }

        /// The handle that detaches it.
        pub(crate) fn handle(&self) -> Handle {
            Handle(self.window as usize)
        }

        fn make(tick: Tick) -> Option<Timer> {
            if !register() {
                return None;
            }
            // SAFETY: a message-only window of the class just registered,
            // with no name, parent or menu.
            let window = unsafe {
                CreateWindowExW(
                    0,
                    CLASS,
                    std::ptr::null(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    HWND_MESSAGE,
                    std::ptr::null_mut(),
                    module(),
                    std::ptr::null(),
                )
            };
            if window.is_null() {
                return None;
            }
            let timer = Timer {
                window,
                tick: Box::into_raw(Box::new(tick)),
            };
            // SAFETY: the window just made; its user data is the tick, which
            // lives until the window is destroyed in `drop`.
            unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, timer.tick as isize) };
            // SAFETY: a timer on the window just made, posting `WM_TIMER`.
            if unsafe { SetTimer(window, ID, PERIOD_MS, None) } == 0 {
                return None;
            }
            Some(timer)
        }
    }

    impl Drop for Timer {
        fn drop(&mut self) {
            // SAFETY: the window this timer made, on the thread that made it
            // (the timer lives in main-thread state). After these calls no
            // message reaches the window, so the tick can be freed.
            unsafe {
                KillTimer(self.window, ID);
                SetWindowLongPtrW(self.window, GWLP_USERDATA, 0);
                DestroyWindow(self.window);
                drop(Box::from_raw(self.tick));
            }
        }
    }

    impl std::fmt::Debug for Timer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Timer").finish_non_exhaustive()
        }
    }

    unsafe extern "system" fn window_procedure(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == WM_TIMER && wparam == ID {
            // SAFETY: the window's user data is its timer's tick, or zero
            // once the timer is being dropped or was abandoned.
            let tick = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *const Tick;
            if tick.is_null() {
                // SAFETY: the timer set on this window, on this thread, which
                // made it: an abandoned timer stops here.
                unsafe { KillTimer(window, ID) };
            } else {
                // SAFETY: a non-zero user data is the timer's tick, which
                // lives until the timer's drop clears it. Windows runs a
                // window's procedure only on the thread that made it: the
                // instance's main thread, which the tick needs.
                guard((), || unsafe { (*tick)() });
            }
            return 0;
        }
        // SAFETY: every other message goes to the default procedure.
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }

    /// Runs this thread's message loop for `period`, as a host's would, so
    /// a timer started here ticks; for tests.
    #[cfg(any(test, feature = "test-host"))]
    pub(crate) fn pump(period: std::time::Duration) {
        use std::time::{Duration, Instant};

        use windows_sys::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, MSG, PM_REMOVE, PeekMessageW,
        };

        let end = Instant::now() + period;
        while Instant::now() < end {
            // SAFETY: all zeros is a valid message, which `PeekMessageW`
            // overwrites.
            let mut message: MSG = unsafe { std::mem::zeroed() };
            // SAFETY: reads and dispatches this thread's own messages.
            while unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0
            {
                // SAFETY: a message just taken from the queue.
                unsafe { DispatchMessageW(&message) };
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

#[cfg(all(target_os = "macos", feature = "vst3", not(miri)))]
mod imp {
    use std::ffi::c_void;

    use crate::boundary::guard;

    type CFTypeRef = *const c_void;
    type CFRunLoopRef = *mut c_void;
    type CFRunLoopTimerRef = *mut c_void;
    type CFStringRef = *const c_void;
    type CFAllocatorRef = *const c_void;
    type CFIndex = isize;
    type CFOptionFlags = usize;
    type Callout = unsafe extern "C" fn(CFRunLoopTimerRef, *mut c_void);

    /// `CFRunLoopTimerContext`, as `CFRunLoop.h` lays it out.
    #[repr(C)]
    struct TimerContext {
        version: CFIndex,
        info: *mut c_void,
        retain: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
        release: Option<unsafe extern "C" fn(*const c_void)>,
        copy_description: Option<unsafe extern "C" fn(*const c_void) -> CFStringRef>,
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFRunLoopCommonModes: CFStringRef;
        fn CFAbsoluteTimeGetCurrent() -> f64;
        fn CFRunLoopGetCurrent() -> CFRunLoopRef;
        fn CFRunLoopTimerCreate(
            allocator: CFAllocatorRef,
            fire_date: f64,
            interval: f64,
            flags: CFOptionFlags,
            order: CFIndex,
            callout: Callout,
            context: *mut TimerContext,
        ) -> CFRunLoopTimerRef;
        fn CFRunLoopAddTimer(run_loop: CFRunLoopRef, timer: CFRunLoopTimerRef, mode: CFStringRef);
        fn CFRunLoopTimerInvalidate(timer: CFRunLoopTimerRef);
        fn CFRelease(object: CFTypeRef);
    }

    /// Seconds between ticks.
    const PERIOD: f64 = 1.0 / 60.0;

    type Tick = Box<dyn Fn()>;

    /// Calls its tick on the run loop of the thread that started it, until
    /// dropped there.
    pub(crate) struct Timer {
        timer: CFRunLoopTimerRef,
        tick: *mut Tick,
    }

    /// A timer's run loop timer, which any thread may [`abandon`].
    #[derive(Debug, Clone, Copy)]
    pub(crate) struct Handle(usize);

    /// Stops a timer that will never be dropped, from any thread: for an
    /// instance whose main-thread state, timer included, is leaked.
    ///
    /// Its run loop calls the tick no more, so a host may unload the bundle;
    /// the tick a callout is running finishes, and the tick's box leaks.
    pub(crate) fn abandon(handle: Handle) {
        // SAFETY: the run loop timer this module made, kept alive by the
        // leaked timer's own reference; Core Foundation's run loop timers
        // may be invalidated from any thread.
        unsafe { CFRunLoopTimerInvalidate(handle.0 as CFRunLoopTimerRef) };
    }

    /// Whether the run loop calls the tick no more: the timer was
    /// invalidated, as an abandoned one is. For tests of a leaked timer,
    /// whose run loop timer is still alive to ask.
    #[cfg(test)]
    pub(crate) fn detached(handle: Handle) -> bool {
        #[link(name = "CoreFoundation", kind = "framework")]
        unsafe extern "C" {
            fn CFRunLoopTimerIsValid(timer: CFRunLoopTimerRef) -> u8;
        }
        // SAFETY: a run loop timer this module made, which a leaked timer's
        // own reference keeps alive; any thread may ask.
        unsafe { CFRunLoopTimerIsValid(handle.0 as CFRunLoopTimerRef) == 0 }
    }

    impl Timer {
        /// Starts ticking `tick` on the calling thread's run loop. `None`
        /// when the system refuses a timer: the instance then never ticks.
        pub(crate) fn start(tick: Tick) -> Option<Timer> {
            let tick = Box::into_raw(Box::new(tick));
            let mut context = TimerContext {
                version: 0,
                info: tick.cast(),
                retain: None,
                release: None,
                copy_description: None,
            };
            // SAFETY: a repeating timer with the default allocator; the
            // context is copied during the call, and its `info`, the tick,
            // lives until the timer's drop, after it is invalidated.
            let timer = unsafe {
                CFRunLoopTimerCreate(
                    std::ptr::null(),
                    CFAbsoluteTimeGetCurrent() + PERIOD,
                    PERIOD,
                    0,
                    0,
                    callout,
                    &mut context,
                )
            };
            if timer.is_null() {
                // SAFETY: the box made above, which no timer holds.
                drop(unsafe { Box::from_raw(tick) });
                return None;
            }
            // SAFETY: the calling thread's own run loop and the timer just
            // made; the common modes keep it ticking while a host's modal
            // loop runs.
            unsafe { CFRunLoopAddTimer(CFRunLoopGetCurrent(), timer, kCFRunLoopCommonModes) };
            Some(Timer { timer, tick })
        }

        /// The handle that stops it.
        pub(crate) fn handle(&self) -> Handle {
            Handle(self.timer as usize)
        }
    }

    impl Drop for Timer {
        fn drop(&mut self) {
            // SAFETY: the timer this one made, on the thread whose run loop
            // runs it (the timer lives in main-thread state). This is
            // outside any of its callouts: an instance released inside one
            // leaks instead. Once invalidated, no callout reaches the tick,
            // so it can be freed. This releases the reference `start` took.
            unsafe {
                CFRunLoopTimerInvalidate(self.timer);
                CFRelease(self.timer.cast_const());
                drop(Box::from_raw(self.tick));
            }
        }
    }

    impl std::fmt::Debug for Timer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Timer").finish_non_exhaustive()
        }
    }

    unsafe extern "C" fn callout(_timer: CFRunLoopTimerRef, info: *mut c_void) {
        let tick = info.cast_const().cast::<Tick>();
        // SAFETY: `info` is the timer's tick, which lives until the timer's
        // drop invalidates it first. The run loop calls this only on the
        // thread that started it: the instance's main thread, which the tick
        // needs.
        guard((), || unsafe { (*tick)() });
    }

    /// Nothing to undo: the timers hold no class.
    pub(crate) fn unregister() {}

    /// Runs this thread's run loop for `period`, as a host's would, so a
    /// timer started here ticks; for tests.
    #[cfg(any(test, feature = "test-host"))]
    pub(crate) fn pump(period: std::time::Duration) {
        #[link(name = "CoreFoundation", kind = "framework")]
        unsafe extern "C" {
            static kCFRunLoopDefaultMode: CFStringRef;
            fn CFRunLoopRunInMode(mode: CFStringRef, seconds: f64, return_after_source: u8) -> i32;
        }
        // SAFETY: runs the calling thread's own run loop in its default
        // mode, one of the common modes the timers are added to.
        unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, period.as_secs_f64(), 0) };
    }
}

#[cfg(not(any(
    all(windows, feature = "vst3", not(miri)),
    all(target_os = "macos", feature = "vst3", not(miri))
)))]
mod imp {
    /// No timer on this platform.
    #[derive(Debug)]
    pub(crate) struct Timer;

    /// Nothing to stop either.
    #[derive(Debug, Clone, Copy)]
    pub(crate) struct Handle;

    #[cfg(feature = "vst3")]
    impl Timer {
        pub(crate) fn start(_: Box<dyn Fn()>) -> Option<Timer> {
            None
        }

        pub(crate) fn handle(&self) -> Handle {
            Handle
        }
    }

    pub(crate) fn abandon(_: Handle) {}

    /// No timer reaches a tick.
    #[cfg(all(test, feature = "vst3"))]
    pub(crate) fn detached(_: Handle) -> bool {
        true
    }

    #[cfg(feature = "vst3")]
    pub(crate) fn unregister() {}

    /// Waits out `period`: there is no loop to run.
    #[cfg(all(any(test, feature = "test-host"), feature = "vst3"))]
    pub(crate) fn pump(period: std::time::Duration) {
        std::thread::sleep(period);
    }
}

#[cfg(all(test, any(windows, target_os = "macos"), feature = "vst3", not(miri)))]
mod tests;
