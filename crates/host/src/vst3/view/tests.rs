//! The editor's view through its COM interfaces, with a stub editor that
//! records each call the exporter makes of it, and a host that calls back in
//! from inside the editor's own `open`.

use std::cell::{Cell, RefCell};
use std::error::Error;
use std::ffi::c_void;

use ::vst3::Steinberg::Vst::IEditControllerTrait;
use ::vst3::Steinberg::{
    IPlugView, IPlugViewTrait, ViewRect, kInternalError, kPlatformTypeHWND, kResultFalse,
};
use ::vst3::{ComPtr, ComRef};

use crate::faults::written;
use crate::vst3::{Refused, TestHost, TestView};
use crate::{
    AuCodes, Block, Category, Editor, Layout, ParentWindow, Parts, Plugin, PluginInfo,
    ProcessSetup, Processor, Size, Status,
};

/// What the stub editor was asked to do, in order.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Call {
    Scale(f64),
    Size(u32, u32),
    Open,
    Close,
    Tick,
}

/// How the stub's `open` goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Opens {
    Fine,
    Fails,
    Panics,
}

thread_local! {
    static CALLS: RefCell<Vec<Call>> = const { RefCell::new(Vec::new()) };
    static OPENS: Cell<Opens> = const { Cell::new(Opens::Fine) };
    /// What the host does from inside the editor's `open`, as a message
    /// loop run there would deliver it.
    static DURING_OPEN: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
    static EDITORS_DROPPED: Cell<u32> = const { Cell::new(0) };
    static PLUGINS_DROPPED: Cell<u32> = const { Cell::new(0) };
}

fn calls() -> Vec<Call> {
    CALLS.with(|calls| std::mem::take(&mut *calls.borrow_mut()))
}

fn record(call: Call) {
    CALLS.with(|calls| calls.borrow_mut().push(call));
}

/// Runs `during` from inside the next `open`.
fn during_open(during: impl FnOnce() + 'static) {
    DURING_OPEN.with(|slot| *slot.borrow_mut() = Some(Box::new(during)));
}

struct Stub;

impl Editor for Stub {
    fn size(&self) -> Size {
        Size {
            width: 320,
            height: 200,
        }
    }

    fn open(&mut self, _parent: &ParentWindow) -> Result<(), Box<dyn Error>> {
        record(Call::Open);
        if let Some(during) = DURING_OPEN.with(|slot| slot.borrow_mut().take()) {
            during();
        }
        match OPENS.with(Cell::get) {
            Opens::Fine => Ok(()),
            Opens::Fails => Err("no WebView runtime".into()),
            Opens::Panics => panic!("the editor's bug"),
        }
    }

    fn close(&mut self) {
        record(Call::Close);
    }

    fn set_size(&mut self, size: Size) {
        record(Call::Size(size.width, size.height));
    }

    fn set_scale(&mut self, scale: f64) {
        record(Call::Scale(scale));
    }

    fn tick(&mut self) {
        record(Call::Tick);
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        EDITORS_DROPPED.with(|count| count.set(count.get() + 1));
    }
}

struct Through;

impl Processor for Through {
    fn process(&mut self, _block: Block<'_, '_>) -> Status {
        Status::Continue
    }
}

const fn info(name: &'static str) -> PluginInfo {
    PluginInfo {
        name,
        vendor: "Example",
        url: "",
        email: "",
        description: "",
        version: "0.1.0",
        id: "dev.example.view",
        vst3_class_id: [7; 16],
        au_codes: AuCodes {
            subtype: *b"view",
            manufacturer: *b"Exmp",
        },
        category: Category::Effect,
        features: &[],
        layouts: &[Layout::STEREO],
        midi_input: false,
        midi_output: false,
    }
}

/// A plugin with the stub editor.
struct WithEditor;

impl Plugin for WithEditor {
    const INFO: PluginInfo = info("With editor");
    type Params = ();
    type Processor = Through;

    fn new(_params: (), _parts: Parts) -> Self {
        WithEditor
    }

    fn activate(&mut self, _setup: &ProcessSetup) -> Through {
        Through
    }

    fn editor(&mut self) -> Option<Box<dyn Editor>> {
        Some(Box::new(Stub))
    }
}

impl Drop for WithEditor {
    fn drop(&mut self) {
        PLUGINS_DROPPED.with(|count| count.set(count.get() + 1));
    }
}

/// A plugin with no editor.
struct Bare;

impl Plugin for Bare {
    const INFO: PluginInfo = info("Bare");
    type Params = ();
    type Processor = Through;

    fn new(_params: (), _parts: Parts) -> Self {
        Bare
    }

    fn activate(&mut self, _setup: &ProcessSetup) -> Through {
        Through
    }
}

/// A host with a view of the stub, and the calls before the view's taken.
fn viewed() -> (TestHost<WithEditor>, TestView) {
    OPENS.with(|opens| opens.set(Opens::Fine));
    let host = TestHost::<WithEditor>::new();
    let view = host.view().expect("an editor");
    calls();
    (host, view)
}

#[test]
fn a_plugin_with_no_editor_offers_no_view() {
    let host = TestHost::<Bare>::new();
    assert!(host.view().is_none());
}

#[test]
fn a_view_of_another_name_is_not_offered() {
    let (host, _view) = viewed();
    let (_, _, controller) = host.interfaces();
    for name in [c"settings".as_ptr(), std::ptr::null()] {
        // SAFETY: the component is alive while the host holds it; the name
        // is null or a terminated string.
        let view = unsafe { controller.createView(name) };
        assert!(view.is_null());
    }
}

#[test]
fn the_view_attaches_to_a_windows_window_only() {
    let (_host, view) = viewed();
    assert_eq!(view.supports(c"HWND"), cfg!(windows));
    assert!(!view.supports(c"NSView"));
    assert!(view.can_resize());
}

#[test]
fn attaching_tells_the_editor_its_scale_and_size_then_opens_it() {
    let (_host, view) = viewed();
    assert_eq!(view.size(), Some((320, 200)));
    assert_eq!(view.attach(1), Ok(()));
    assert_eq!(
        calls(),
        [Call::Scale(1.0), Call::Size(320, 200), Call::Open]
    );
    // Attached twice with no removal between: refused, the editor untouched.
    assert_eq!(view.attach(1), Err(Refused(kResultFalse)));
    assert!(calls().is_empty());
    assert_eq!(view.remove(), Ok(()));
    assert_eq!(calls(), [Call::Close]);
}

#[test]
fn a_null_window_is_refused_and_the_editor_not_opened() {
    let (_host, view) = viewed();
    assert!(view.attach(0).is_err());
    assert!(calls().is_empty());
}

#[test]
fn a_resize_reaches_the_editor_in_logical_pixels() {
    let (_host, view) = viewed();
    view.attach(1).expect("opens");
    calls();
    view.resize(640, 480).expect("resizes");
    assert_eq!(calls(), [Call::Size(640, 480)]);
    assert_eq!(view.size(), Some((640, 480)));
    // At 150 %, the window's pixels are the screen's.
    view.set_scale(1.5).expect("scales");
    calls();
    view.resizes();
    view.resize(900, 600).expect("resizes");
    assert_eq!(calls(), [Call::Size(600, 400)]);
    assert_eq!(view.size(), Some((900, 600)));
}

#[test]
fn a_scale_before_opening_sizes_the_window_and_asks_for_nothing() {
    let (_host, view) = viewed();
    view.set_scale(2.0).expect("scales");
    assert!(view.resizes().is_empty());
    assert_eq!(view.size(), Some((640, 400)));
    view.attach(1).expect("opens");
    assert_eq!(
        calls(),
        [Call::Scale(2.0), Call::Size(320, 200), Call::Open]
    );
}

#[test]
fn a_new_scale_while_open_asks_the_frame_for_the_editors_size_there() {
    let (_host, view) = viewed();
    view.attach(1).expect("opens");
    calls();
    view.set_scale(1.25).expect("scales");
    // The frame passes the size straight back to `onSize`, as a host does;
    // it is the same logical size, so the editor hears only the scale.
    assert_eq!(view.resizes(), [(400, 250)]);
    assert_eq!(calls(), [Call::Scale(1.25)]);
    view.set_scale(1.25).expect("the same scale");
    assert!(view.resizes().is_empty());
    assert!(view.set_scale(0.0).is_err());
    assert!(view.set_scale(f32::NAN).is_err());
}

#[test]
fn a_view_attaches_again_after_it_was_removed() {
    let (_host, view) = viewed();
    view.attach(1).expect("opens");
    view.remove().expect("removed");
    view.resize(400, 300).expect("resizes while closed");
    calls();
    view.attach(2).expect("opens again");
    assert_eq!(calls(), [Call::Size(400, 300), Call::Open]);
    // A removal of a closed view closes nothing.
    view.remove().expect("removed");
    view.remove().expect("removed again");
    assert_eq!(calls(), [Call::Close]);
}

#[test]
fn a_tick_reaches_an_open_editor_and_not_a_closed_one() {
    let (host, view) = viewed();
    host.tick();
    assert!(calls().is_empty(), "not opened yet");
    view.attach(1).expect("opens");
    calls();
    host.tick();
    host.tick();
    assert_eq!(calls(), [Call::Tick, Call::Tick]);
    view.remove().expect("removed");
    calls();
    host.tick();
    assert!(calls().is_empty(), "closed");
}

#[test]
fn two_views_each_have_an_editor_of_their_own() {
    let (host, first) = viewed();
    let second = host.view().expect("a second editor");
    first.attach(1).expect("opens");
    second.attach(2).expect("opens");
    calls();
    host.tick();
    assert_eq!(calls(), [Call::Tick, Call::Tick]);
    drop(second);
    assert_eq!(calls(), [Call::Close]);
    host.tick();
    assert_eq!(calls(), [Call::Tick]);
}

#[test]
fn a_view_released_while_attached_closes_its_editor() {
    let (host, view) = viewed();
    view.attach(1).expect("opens");
    calls();
    let dropped = EDITORS_DROPPED.with(Cell::get);
    drop(view);
    assert_eq!(calls(), [Call::Close]);
    assert_eq!(EDITORS_DROPPED.with(Cell::get), dropped + 1);
    host.tick();
    assert!(calls().is_empty());
}

#[test]
fn the_instance_released_before_its_view_lives_until_the_view_goes() {
    let (host, view) = viewed();
    view.attach(1).expect("opens");
    calls();
    let plugins = PLUGINS_DROPPED.with(Cell::get);
    // The host terminates and releases the instance with the view still
    // attached, against the SDK's order.
    drop(host);
    assert_eq!(
        PLUGINS_DROPPED.with(Cell::get),
        plugins,
        "the view holds it"
    );
    assert_eq!(view.size(), Some((320, 200)));
    view.resize(400, 300).expect("still answers");
    // Released at last without a removal: the editor closes, then the
    // instance goes.
    drop(view);
    assert_eq!(calls(), [Call::Size(400, 300), Call::Close]);
    assert_eq!(PLUGINS_DROPPED.with(Cell::get), plugins + 1);
}

#[test]
fn a_view_call_from_another_thread_is_refused_and_counted() {
    let (host, view) = viewed();
    let interface = view.interface().clone();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            let mut rect = ViewRect {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            // SAFETY: the view is alive while `view` holds it; `rect` is
            // valid for the call.
            let answer = unsafe { interface.getSize(&mut rect) };
            assert_eq!(answer, kInternalError);
            // SAFETY: as above.
            let answer = unsafe {
                interface.attached(std::ptr::dangling_mut::<c_void>(), kPlatformTypeHWND)
            };
            assert_eq!(answer, kResultFalse);
        });
    });
    host.expect_faults(2);
    assert!(calls().is_empty());
    assert_eq!(view.size(), Some((320, 200)), "still answers here");
}

#[test]
fn an_editor_that_fails_to_open_is_a_fault_with_its_reason() {
    let (host, view) = viewed();
    OPENS.with(|opens| opens.set(Opens::Fails));
    written();
    assert_eq!(view.attach(1), Err(Refused(kResultFalse)));
    host.expect_faults(1);
    let lines = written();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("editor failed to open"), "{}", lines[0]);
    assert!(lines[0].contains("no WebView runtime"), "{}", lines[0]);
    // Not open, so no tick and no close; a second failure writes no line.
    host.tick();
    view.remove().expect("removed");
    assert_eq!(
        calls(),
        [Call::Scale(1.0), Call::Size(320, 200), Call::Open]
    );
    assert!(view.attach(1).is_err());
    host.expect_faults(1);
    assert!(written().is_empty());
    assert_eq!(calls(), [Call::Open], "told its size and scale already");
    // It opens once the editor can.
    OPENS.with(|opens| opens.set(Opens::Fine));
    assert_eq!(view.attach(1), Ok(()));
}

#[test]
fn an_editor_that_panics_in_open_is_a_fault_and_stays_closed() {
    let (host, view) = viewed();
    OPENS.with(|opens| opens.set(Opens::Panics));
    assert_eq!(view.attach(1), Err(Refused(kResultFalse)));
    host.expect_faults(1);
    calls();
    host.tick();
    view.remove().expect("removed");
    assert!(calls().is_empty(), "never opened");
    OPENS.with(|opens| opens.set(Opens::Fine));
    assert_eq!(view.attach(1), Ok(()));
}

#[test]
fn a_resize_and_a_tick_from_inside_open_wait_until_it_returns() {
    let (host, view) = viewed();
    let interface = view.interface().clone();
    let host = std::rc::Rc::new(host);
    let ticker = std::rc::Rc::clone(&host);
    during_open(move || {
        ticker.tick(); // the timer, delivered by the toolkit's message loop
        let mut rect = ViewRect {
            left: 0,
            top: 0,
            right: 500,
            bottom: 400,
        };
        // SAFETY: the view is alive while `view` holds it; `rect` is valid
        // for the call.
        assert_eq!(unsafe { interface.onSize(&mut rect) }, 0);
    });
    assert_eq!(view.attach(1), Ok(()));
    // The tick passed the editor over; the size is told once `open`
    // returned.
    assert_eq!(
        calls(),
        [
            Call::Scale(1.0),
            Call::Size(320, 200),
            Call::Open,
            Call::Size(500, 400)
        ]
    );
    host.tick();
    assert_eq!(calls(), [Call::Tick]);
}

#[test]
fn a_removal_from_inside_open_closes_the_editor_once_it_returns() {
    let (host, view) = viewed();
    let interface = view.interface().clone();
    during_open(move || {
        let mut rect = ViewRect {
            left: 0,
            top: 0,
            right: 500,
            bottom: 400,
        };
        // SAFETY: the view is alive while `view` holds it; `rect` is valid
        // for the calls.
        unsafe {
            assert_eq!(interface.onSize(&mut rect), 0);
            assert_eq!(interface.removed(), 0);
        }
    });
    assert_eq!(view.attach(1), Ok(()));
    // Closed straight after `open`, with no size told to a closing editor.
    assert_eq!(
        calls(),
        [
            Call::Scale(1.0),
            Call::Size(320, 200),
            Call::Open,
            Call::Close
        ]
    );
    host.tick();
    assert!(calls().is_empty(), "closed");
    assert_eq!(view.attach(1), Ok(()), "and opens again");
    assert_eq!(calls(), [Call::Size(500, 400), Call::Open]);
}

#[test]
fn a_view_the_host_releases_from_inside_open_is_freed_after_it() {
    let (_host, view) = viewed();
    // The host's only reference, as a raw pointer the host releases from
    // inside `open`.
    let raw = ComPtr::into_raw(view.interface().clone());
    drop(view);
    let dropped = EDITORS_DROPPED.with(Cell::get);
    during_open(move || {
        // SAFETY: `raw` owns one reference, released here.
        drop(unsafe { ComPtr::<IPlugView>::from_raw(raw) });
        assert_eq!(EDITORS_DROPPED.with(Cell::get), dropped, "still held");
    });
    // SAFETY: `raw` is alive until the release inside `open`; the call holds
    // the view itself from then on.
    let answer = unsafe {
        ComRef::from_raw(raw)
            .expect("a view")
            .attached(std::ptr::dangling_mut::<c_void>(), kPlatformTypeHWND)
    };
    assert_eq!(answer, 0);
    // Released once the call returned: the editor closed and went.
    assert_eq!(
        calls(),
        [
            Call::Scale(1.0),
            Call::Size(320, 200),
            Call::Open,
            Call::Close
        ]
    );
    assert_eq!(EDITORS_DROPPED.with(Cell::get), dropped + 1);
}
