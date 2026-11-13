//! The editor on Windows: a WebView2 child window inside the host's, made
//! by wry.
//!
//! wry's start-up runs the thread's message loop until the WebView exists,
//! so the host's messages, a tick among them, arrive inside `open`; the
//! exporter puts off what they ask of this editor until `open` returns. The
//! page's messages arrive later, from the message loop, through the IPC
//! handler, outside any call of the exporter's: a panic there is caught,
//! since it would unwind into WebView2.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::error::Error;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Instant;

use seam_bridge::{Dispatcher, Outbox};
use seam_host::{Editor, ParentWindow, Size};
use wry::dpi::{PhysicalPosition, PhysicalSize};
use wry::http::header::{CONTENT_TYPE, HeaderValue};
use wry::http::{Response, StatusCode};
use wry::{Rect, WebContext, WebView, WebViewBuilder, WebViewBuilderExtWindows};

use crate::Page;
use crate::assets::{SCHEME, data_folder, dev_url, page_url, serve};

/// The page's side of the conversation: what the dispatcher posts goes to
/// the WebView open now, in order, and nowhere while none is.
struct ToPage {
    name: &'static str,
    webview: Rc<RefCell<Weak<WebView>>>,
    /// When the editor opened, until the page's first `ready` of that open
    /// is answered.
    opened: Rc<Cell<Option<Instant>>>,
}

impl Outbox for ToPage {
    fn post(&mut self, message: &str) {
        if message.starts_with(r#"{"type":"snapshot""#)
            && let Some(opened) = self.opened.take()
        {
            seam_host::report_line(&format!(
                "seam: {}: the page answered its first ready {} ms after the editor opened.",
                self.name,
                opened.elapsed().as_millis()
            ));
        }
        let webview = self.webview.borrow().upgrade();
        if let Some(webview) = webview {
            // A message the page cannot take is lost with the page, which
            // asks again with its next `ready`.
            let _ = webview.evaluate_script(&script(message));
        }
    }
}

/// The script that hands `message` to the page's transport, which listens
/// as `window.__seam`: the message as a JavaScript string, which JSON's
/// string syntax is.
fn script(message: &str) -> String {
    let text = serde_json::Value::String(message.to_owned()).to_string();
    format!("window.__seam?.receive({text})")
}

/// The editor: one WebView at a time, made at each open.
pub(crate) struct PageEditor {
    name: &'static str,
    folder: PathBuf,
    assets: &'static [crate::Asset],
    size: Size,
    scale: f64,
    native: Rc<RefCell<Dispatcher<ToPage>>>,
    /// Messages the page sent while the dispatcher was in a call of its own;
    /// the next tick hands them over, in order.
    held: Rc<RefCell<VecDeque<String>>>,
    target: Rc<RefCell<Weak<WebView>>>,
    opened: Rc<Cell<Option<Instant>>>,
    webview: Option<Rc<WebView>>,
}

impl PageEditor {
    pub(crate) fn new(page: Page) -> Self {
        let target = Rc::new(RefCell::new(Weak::new()));
        let opened = Rc::new(Cell::new(None));
        let outbox = ToPage {
            name: page.name,
            webview: Rc::clone(&target),
            opened: Rc::clone(&opened),
        };
        let base = std::env::var_os("LOCALAPPDATA").map_or_else(std::env::temp_dir, PathBuf::from);
        PageEditor {
            name: page.name,
            folder: data_folder(&base, page.id),
            assets: page.assets,
            size: page.size,
            scale: 1.0,
            native: Rc::new(RefCell::new(Dispatcher::new(
                page.ui,
                page.commands,
                outbox,
            ))),
            held: Rc::new(RefCell::new(VecDeque::new())),
            target,
            opened,
            webview: None,
        }
    }

    /// The WebView's place in the host's window: all of it, in the screen's
    /// pixels.
    fn bounds(&self) -> Rect {
        let pixels = |logical: u32| (f64::from(logical) * self.scale).round() as u32;
        Rect {
            position: PhysicalPosition::new(0, 0).into(),
            size: PhysicalSize::new(pixels(self.size.width), pixels(self.size.height)).into(),
        }
    }

    fn place(&self) {
        if let Some(webview) = &self.webview {
            // A WebView whose window went with the host's takes no bounds;
            // the close that follows drops it.
            let _ = webview.set_bounds(self.bounds());
        }
    }
}

impl Editor for PageEditor {
    fn size(&self) -> Size {
        self.size
    }

    fn open(&mut self, parent: &ParentWindow) -> Result<(), Box<dyn Error>> {
        self.opened.set(Some(Instant::now()));
        let mut context = WebContext::new(Some(self.folder.clone()));
        let (assets, name) = (self.assets, self.name);
        let (native, held) = (Rc::clone(&self.native), Rc::clone(&self.held));
        let built = WebViewBuilder::new_with_web_context(&mut context)
            .with_url(page_url(dev_url()))
            .with_custom_protocol(SCHEME.to_owned(), move |_, request| {
                respond(assets, request.uri().path())
            })
            .with_ipc_handler(move |request| deliver(name, &native, &held, request.into_body()))
            .with_bounds(self.bounds())
            // The page takes the keyboard when it is clicked, not as the
            // window opens, so the host keeps it until then.
            .with_focused(false)
            // A release build shows no browser menu and takes no browser
            // shortcut, such as a reload.
            .with_default_context_menus(cfg!(debug_assertions))
            .with_browser_accelerator_keys(cfg!(debug_assertions))
            .build_as_child(parent);
        match built {
            Ok(webview) => {
                let webview = Rc::new(webview);
                *self.target.borrow_mut() = Rc::downgrade(&webview);
                self.webview = Some(webview);
                Ok(())
            }
            Err(error) => {
                self.opened.set(None);
                Err(format!("the WebView did not start: {error}").into())
            }
        }
    }

    fn close(&mut self) {
        if let Ok(mut native) = self.native.try_borrow_mut() {
            native.end_session();
        }
        self.held.borrow_mut().clear();
        self.opened.set(None);
        *self.target.borrow_mut() = Weak::new();
        // Dropping it closes the WebView and destroys its window.
        self.webview = None;
    }

    fn set_size(&mut self, size: Size) {
        self.size = size;
        self.place();
    }

    fn set_scale(&mut self, scale: f64) {
        self.scale = scale;
        self.place();
    }

    fn tick(&mut self) {
        let Ok(mut native) = self.native.try_borrow_mut() else {
            return;
        };
        loop {
            let next = self.held.borrow_mut().pop_front();
            match next {
                Some(message) => native.receive(&message),
                None => break,
            }
        }
        native.tick();
    }
}

/// Hands a message the page posted to the dispatcher, or holds it for the
/// next tick while the dispatcher is in a call. It runs inside WebView2's
/// callback, so a panic is caught here and the message dropped.
fn deliver(
    name: &'static str,
    native: &RefCell<Dispatcher<ToPage>>,
    held: &RefCell<VecDeque<String>>,
    message: String,
) {
    let handled = panic::catch_unwind(AssertUnwindSafe(|| match native.try_borrow_mut() {
        Ok(mut native) => native.receive(&message),
        Err(_) => held.borrow_mut().push_back(message),
    }));
    if handled.is_err() {
        seam_host::report_line(&format!(
            "seam: {name}: a message from the page panicked and was dropped."
        ));
    }
}

/// The scheme's answer to a request for `path`, built from the page's
/// files with no copy.
fn respond(assets: &'static [crate::Asset], path: &str) -> Response<Cow<'static, [u8]>> {
    let served = serve(assets, path);
    let mut response = Response::new(Cow::Borrowed(served.body));
    *response.status_mut() = StatusCode::from_u16(served.status).unwrap_or(StatusCode::NOT_FOUND);
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(served.content_type));
    response
}

impl std::fmt::Debug for PageEditor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PageEditor")
            .field("plugin", &self.name)
            .field("size", &self.size)
            .field("open", &self.webview.is_some())
            .finish_non_exhaustive()
    }
}
