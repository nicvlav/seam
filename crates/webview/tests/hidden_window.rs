//! The page in a real WebView2, in a window of the test's own that is never
//! shown, through the VST3 exporter as a host drives it: the page loads from
//! the files built into the plugin, its `ready` is answered with a
//! snapshot, and its `set` reaches the parameter. Ignored in a normal run,
//! since it needs the WebView2 runtime and starts its browser processes; run
//! it by hand on Windows with `cargo test -p seam-webview --test
//! hidden_window -- --ignored`.
//!
//! It never shows a window, takes focus or sends input: the page acts by
//! itself, as its script says.

#![cfg(windows)]

use std::cell::RefCell;
use std::time::{Duration, Instant};

use seam_host::vst3::{HiddenWindow, TestHost};
use seam_host::{
    AuCodes, Block, Category, Editor, Layout, Parts, Plugin, PluginInfo, ProcessSetup, Processor,
    Size, Status,
};
use seam_params::{Declarer, Float, FloatDecl, Params};
use seam_webview::{Asset, Page};

/// A page that speaks the protocol by hand, as `seam-page`'s client does:
/// it listens, sends `ready`, and once the snapshot arrives reports it
/// through a command and sets the level to 0.75.
const PAGE: &[Asset] = &[Asset {
    path: "index.html",
    bytes: br#"<!doctype html>
<html><body><p>page</p><script>
  const send = (message) => window.ipc.postMessage(JSON.stringify(message));
  window.__seam = {
    receive(text) {
      const message = JSON.parse(text);
      if (message.type === "snapshot") {
        send({ type: "invoke", call: 1, command: "seen", args: message.params[0].text });
        send({ type: "set", param: "level", normalized: 0.75 });
      }
    },
  };
  send({ type: "ready", protocol: 2 });
</script></body></html>"#,
}];

thread_local! {
    /// What the page reported it saw in its snapshot.
    static SEEN: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

struct ProbeParams {
    #[expect(
        dead_code,
        reason = "declared for the host and the page; read through the host"
    )]
    level: Float,
}

impl Params for ProbeParams {
    fn declare(p: &mut Declarer) -> Self {
        ProbeParams {
            level: p.add(FloatDecl::new("level", "Level", 0.0..=100.0, 50.0).with_unit("%")),
        }
    }
}

struct Through;

impl Processor for Through {
    fn process(&mut self, _block: Block<'_, '_>) -> Status {
        Status::Continue
    }
}

struct Probe {
    parts: Parts,
}

impl Plugin for Probe {
    const INFO: PluginInfo = PluginInfo {
        name: "Seam WebView Probe",
        vendor: "Seam",
        url: "",
        email: "",
        description: "",
        version: "0.1.0",
        id: "dev.seam.webview-probe",
        vst3_class_id: [9; 16],
        au_codes: AuCodes {
            subtype: *b"wvpr",
            manufacturer: *b"Seam",
        },
        category: Category::Effect,
        features: &[],
        layouts: &[Layout::STEREO],
        midi_input: false,
        midi_output: false,
    };
    type Params = ProbeParams;
    type Processor = Through;

    fn new(_params: ProbeParams, parts: Parts) -> Self {
        Probe { parts }
    }

    fn activate(&mut self, _setup: &ProcessSetup) -> Through {
        Through
    }

    fn editor(&mut self) -> Option<Box<dyn Editor>> {
        Page::new(&Self::INFO, self.parts.ui().clone(), PAGE)
            .with_size(Size {
                width: 400,
                height: 300,
            })
            .command("seen", |text: String| {
                SEEN.with(|seen| seen.borrow_mut().push(text));
                Ok(())
            })
            .into_editor()
    }
}

/// Ticks `hosts` and runs this thread's message loop until `done` holds,
/// for at most `patience`; returns whether it holds.
fn until(hosts: &[TestHost<Probe>], patience: Duration, mut done: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + patience;
    while Instant::now() < end {
        for host in hosts {
            host.tick();
        }
        if done() {
            return true;
        }
        // Runs the message loop for about 10 ms, as a host's would: the
        // second ask ends it.
        let mut asked = false;
        hosts[0].pump_until(|| std::mem::replace(&mut asked, true));
    }
    done()
}

/// Keeps the editors open, ticking, for 20 seconds when `SEAM_HOLD_OPEN`
/// is set: time to read their processes and memory from outside.
fn hold_open(hosts: &[TestHost<Probe>]) {
    if std::env::var_os("SEAM_HOLD_OPEN").is_some() {
        until(hosts, Duration::from_secs(20), || false);
    }
}

fn level(host: &TestHost<Probe>) -> f64 {
    host.plain(
        seam_params::host_id("level"),
        host.normalized(seam_params::host_id("level")),
    )
}

#[test]
#[ignore = "opens a real WebView2 in a hidden window: run by hand on Windows"]
fn a_page_in_a_hidden_window_is_answered_and_its_set_reaches_the_parameter() {
    let host = TestHost::<Probe>::new();
    let window = HiddenWindow::new().expect("a window");
    let view = host.view().expect("an editor");
    let started = Instant::now();
    assert_eq!(view.attach(window.handle()), Ok(()));
    let opened = started.elapsed();
    let hosts = [host];
    let answered = until(&hosts, Duration::from_secs(30), || {
        level(&hosts[0]) == 75.0 && SEEN.with(|seen| !seen.borrow().is_empty())
    });
    println!(
        "open returned after {} ms; the page's set arrived after {} ms",
        opened.as_millis(),
        started.elapsed().as_millis()
    );
    assert!(answered, "the page's set never reached the parameter");
    hold_open(&hosts);
    assert_eq!(SEEN.with(|seen| seen.borrow().clone()), ["50.00 %"]);
    assert_eq!(view.remove(), Ok(()));
    // Closed: a tick reaches no page, and the view opens again.
    let reopened = view.attach(window.handle());
    assert_eq!(reopened, Ok(()));
    assert_eq!(view.remove(), Ok(()));
    drop(view);
}

#[test]
#[ignore = "opens sixteen real WebView2s in hidden windows: run by hand on Windows"]
fn sixteen_editors_open_in_one_process() {
    let hosts: Vec<TestHost<Probe>> = (0..16).map(|_| TestHost::<Probe>::new()).collect();
    let windows: Vec<HiddenWindow> = (0..16)
        .map(|_| HiddenWindow::new().expect("a window"))
        .collect();
    let views: Vec<_> = hosts
        .iter()
        .map(|host| host.view().expect("an editor"))
        .collect();
    let started = Instant::now();
    for (view, window) in views.iter().zip(&windows) {
        assert_eq!(view.attach(window.handle()), Ok(()));
    }
    let opened = started.elapsed();
    let answered = until(&hosts, Duration::from_secs(60), || {
        hosts.iter().all(|host| level(host) == 75.0)
    });
    println!(
        "sixteen opens returned after {} ms; every page's set arrived after {} ms",
        opened.as_millis(),
        started.elapsed().as_millis()
    );
    assert!(answered, "not every page's set reached its parameter");
    hold_open(&hosts);
    for view in &views {
        assert_eq!(view.remove(), Ok(()));
    }
}
