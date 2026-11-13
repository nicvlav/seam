//! The native side of the protocol: one page's conversation with the plugin,
//! driven by the messages the page sends and by the plugin's tick.
//!
//! A [`Dispatcher`] holds what the page shows: one [`Watch`] per listed
//! parameter, and the gestures the page opened. Each `ready` starts a page
//! session: it closes the gestures the last one left open, seeds every watch
//! again, and answers with a snapshot read from them. From then on the tick
//! posts each change the watches see, whatever made it. Every `set` and
//! `gestureEnd` is answered from the parameter's watch, polled and then read,
//! never from the store: while another surface holds a gesture open, the
//! store may hold a host write every surface is holding back.

use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};

use seam_params::{Gesture, ParamDecl, ParamId, UiParams, Watch};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value as Json;

use crate::{
    CallResult, NativeMessage, Outbox, PROTOCOL_VERSION, PageMessage, ParamState, Position,
};

/// One command as the registry keeps it: JSON arguments in, a JSON result
/// or a reason for a developer out.
type Command = Box<dyn FnMut(Json) -> Result<Json, String>>;

/// The commands a page may call by name, each with typed arguments and a
/// typed result.
///
/// A command's arguments are decoded from the JSON the page sent, and a page
/// that sends none sends `null`, which a command taking `()` accepts. Its
/// result is encoded back, `()` as `null`. An `Err` reaches the page as the
/// call's error. A command runs to its end inside the page's call, on the
/// thread the [`Dispatcher`] runs on, so it may capture what lives on that
/// thread.
///
/// # Examples
///
/// ```
/// use std::cell::RefCell;
/// use std::rc::Rc;
///
/// use seam_bridge::Commands;
///
/// let label = Rc::new(RefCell::new(String::from("Untitled")));
/// let (read, write) = (Rc::clone(&label), Rc::clone(&label));
/// let commands = Commands::new()
///     .command("getLabel", move |()| Ok(read.borrow().clone()))
///     .command("setLabel", move |name: String| {
///         if name.is_empty() {
///             return Err("a label is never empty".into());
///         }
///         *write.borrow_mut() = name;
///         Ok(())
///     });
/// assert_eq!(commands.names().count(), 2);
/// ```
#[derive(Default)]
pub struct Commands {
    by_name: HashMap<String, Command>,
}

impl Commands {
    /// Creates a registry with no command.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `command` under `name`: the page calls it with
    /// `useCommand(name)`. [`Commands`] shows two.
    ///
    /// # Panics
    ///
    /// When `name` was added already: give each command its own name.
    pub fn command<A, R>(
        mut self,
        name: &str,
        mut command: impl FnMut(A) -> Result<R, String> + 'static,
    ) -> Self
    where
        A: DeserializeOwned,
        R: Serialize,
    {
        assert!(
            !self.by_name.contains_key(name),
            "the command {name:?} is added twice: give each command its own name"
        );
        let label = name.to_owned();
        let call = move |args: Json| {
            let args = A::deserialize(args)
                .map_err(|error| format!("the arguments of {label:?} do not fit it: {error}"))?;
            let result = command(args)?;
            serde_json::to_value(result)
                .map_err(|error| format!("the result of {label:?} is not JSON: {error}"))
        };
        self.by_name.insert(name.to_owned(), Box::new(call));
        self
    }

    /// Returns the names of the commands, in no particular order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.by_name.keys().map(String::as_str)
    }

    /// Runs the command `name` on `args`. `None` for a name nothing was
    /// added under; `Some(Err(None))` for a command that panicked.
    fn call(&mut self, name: &str, args: Json) -> Option<Result<Json, Option<String>>> {
        let command = self.by_name.get_mut(name)?;
        Some(
            match panic::catch_unwind(AssertUnwindSafe(|| command(args))) {
                Ok(result) => result.map_err(Some),
                Err(_) => Err(None),
            },
        )
    }
}

impl std::fmt::Debug for Commands {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut names: Vec<&str> = self.names().collect();
        names.sort_unstable();
        f.debug_struct("Commands").field("names", &names).finish()
    }
}

/// What a [`Dispatcher`] refused since it was made: messages it dropped
/// unanswered, and commands that panicked.
///
/// A page built from the plugin's own declarations and commands moves none
/// of them, so a count that moved is a page out of step with its plugin, or
/// a bug.
///
/// # Examples
///
/// ```
/// # use seam_params::{Kind, ParamDecl, ParamSet};
/// use seam_bridge::{Commands, Dispatcher, Refusals};
///
/// # let set = ParamSet::from_decls([ParamDecl::new("gain", "Gain", Kind::Bool, 0.0)])?;
/// # let ui = set.take_ui().expect("the first UI handle");
/// let mut native = Dispatcher::new(ui, Commands::new(), Vec::<String>::new());
/// native.receive("not a message");
/// native.receive(r#"{"type":"set","param":"volume","normalized":0.5}"#);
/// assert_eq!(
///     native.refusals(),
///     Refusals { malformed: 1, unknown_params: 1, panicked_commands: 0 },
/// );
/// # Ok::<(), seam_params::DeclError>(())
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Refusals {
    /// Messages that did not decode ([`PageMessage::decode`]), dropped
    /// unanswered.
    pub malformed: u64,
    /// Gestures and writes that name a key the plugin does not list, dropped
    /// unanswered: no parameter has a state to answer with.
    pub unknown_params: u64,
    /// Commands that panicked, each answered with an error.
    pub panicked_commands: u64,
}

/// One listed parameter as the page sees it.
#[derive(Debug)]
struct Listed {
    id: ParamId,
    watch: Watch,
    /// The gestures the page opened on it in this session, one per begin,
    /// so a page's two controls on one parameter each close their own.
    gestures: Vec<Gesture>,
}

/// The native side of one page: it answers what the page sends, and tells it
/// of every change to the plugin's listed parameters.
///
/// It is driven by two calls on the UI thread: [`receive`](Self::receive)
/// for each message the page sent, and [`tick`](Self::tick) once per main
/// thread tick. It knows nothing of how text travels; what it posts goes to
/// its [`Outbox`] in order. The rules it keeps:
///
/// - **`ready`.** Answered with a snapshot of every listed parameter, in
///   declaration order, when the page speaks [`PROTOCOL_VERSION`], and with a
///   `reject` otherwise. Either way it ends the page session before it: the
///   gestures that session left open close, once. A snapshot starts the
///   next.
/// - **Writes.** `gestureBegin`, `set` and `gestureEnd` reach the parameter
///   through [`UiParams`], as a user's: one undo step per drag, and the host
///   hears one gesture however many controls drag one parameter. A `set`
///   outside a drag is a gesture of its own. Every `set` and `gestureEnd` is
///   answered with the parameter's state, changed or not.
/// - **Calls.** An `invoke` runs the command of its name from [`Commands`]
///   and is answered with its result, or with an error for a name nothing
///   was added under, for arguments that do not fit, and for a command that
///   panicked. A command runs to its end inside the call, so no call outlives
///   the page session that made it.
/// - **Changes.** While a session is live, each tick posts the state of each
///   listed parameter that moved since the last post, whatever moved it: the
///   host, undo, a load, learn or another surface. Nothing moved costs one
///   check per parameter and no allocation.
/// - **Refusals.** A message that does not decode, and a write that names a
///   key the plugin does not list, are dropped unanswered and counted in
///   [`refusals`](Self::refusals). Nothing the page sends makes it panic.
///
/// [`end_session`](Self::end_session), or dropping it, ends the page
/// session, closing the gestures still open, as an editor does when it
/// closes.
///
/// # Examples
///
/// A page drags the gain; the host's automation then moves it, and the next
/// tick tells the page.
///
/// ```
/// use seam_bridge::{Commands, Dispatcher, NativeMessage};
/// use seam_params::{Kind, ParamDecl, ParamSet};
///
/// let set = ParamSet::from_decls([
///     ParamDecl::new("gain", "Gain", Kind::continuous(-60.0, 12.0, 0.1, None), 0.0)
///         .with_unit("dB"),
/// ])?;
/// let ui = set.take_ui().expect("the first UI handle");
/// let mut native = Dispatcher::new(ui, Commands::new(), Vec::<String>::new());
/// let mut answers = |native: &mut Dispatcher<Vec<String>>| {
///     std::mem::take(native.outbox_mut())
///         .iter()
///         .map(|text| serde_json::from_str::<NativeMessage>(text))
///         .collect::<Result<Vec<_>, _>>()
/// };
///
/// native.receive(r#"{"type":"ready","protocol":2}"#);
/// assert!(matches!(answers(&mut native)?[..], [NativeMessage::Snapshot { .. }]));
///
/// // A drag: its set is answered with the snapped value, as text in its unit.
/// native.receive(r#"{"type":"gestureBegin","param":"gain"}"#);
/// native.receive(r#"{"type":"set","param":"gain","normalized":0.75}"#);
/// native.receive(r#"{"type":"gestureEnd","param":"gain"}"#);
/// let gain = set.id("gain").expect("declared");
/// assert_eq!(set.value(gain), -6.0);
/// let [NativeMessage::Param { state }, NativeMessage::Param { .. }] = &answers(&mut native)?[..]
/// else {
///     return Err("an answer to the set and one to the end".into());
/// };
/// assert_eq!(state.text, "-6 dB");
///
/// // Automation moves the gain: the next tick tells the page, once.
/// set.audio().host_write(gain, 1.0);
/// native.tick();
/// native.tick();
/// assert_eq!(answers(&mut native)?.len(), 1);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct Dispatcher<O> {
    ui: UiParams,
    commands: Commands,
    outbox: O,
    params: Box<[Listed]>,
    /// Whether a snapshot answered the last `ready`: a page session is live,
    /// and the tick tells it of changes.
    live: bool,
    refusals: Refusals,
}

impl<O: Outbox> Dispatcher<O> {
    /// Creates the native side of a page over the plugin's listed
    /// parameters, as `ui` reaches them, with `commands` for the page to call
    /// and `outbox` to post to. It posts nothing until the page's first
    /// `ready`.
    ///
    /// `ui` is a clone of the plugin's own handle (`Parts::ui` in
    /// `seam-host`), so the page's drags are a user's: undo records them, and
    /// they share gestures with every other surface.
    ///
    /// Thread: UI, the thread `ui` lives on. [`Dispatcher`] shows one.
    pub fn new(ui: UiParams, commands: Commands, outbox: O) -> Self {
        let params = ui
            .set()
            .ids()
            .map(|id| Listed {
                id,
                watch: ui.watch(id),
                gestures: Vec::new(),
            })
            .collect();
        Dispatcher {
            ui,
            commands,
            outbox,
            params,
            live: false,
            refusals: Refusals::default(),
        }
    }

    /// Handles one message the page sent, as the rules on [`Dispatcher`]
    /// say, posting whatever answers it.
    ///
    /// Thread: UI. [`Dispatcher`] shows each kind of message.
    pub fn receive(&mut self, text: &str) {
        let Ok(message) = PageMessage::decode(text) else {
            self.refusals.malformed += 1;
            return;
        };
        match message {
            PageMessage::Ready { protocol } => self.ready(protocol),
            PageMessage::GestureBegin { param } => {
                if let Some(index) = self.listed(&param) {
                    let gesture = self.ui.begin(self.params[index].id);
                    self.params[index].gestures.push(gesture);
                }
            }
            PageMessage::Set { param, normalized } => {
                if let Some(index) = self.listed(&param) {
                    self.set(index, normalized);
                    self.answer(index);
                }
            }
            PageMessage::GestureEnd { param } => {
                if let Some(index) = self.listed(&param) {
                    // Dropping the token closes it; the last one closes the
                    // gesture for the host.
                    drop(self.params[index].gestures.pop());
                    self.answer(index);
                }
            }
            PageMessage::Invoke {
                call,
                command,
                args,
            } => self.invoke(call, &command, args),
        }
    }

    /// Posts the state of each listed parameter that moved since it was last
    /// posted, while a page session is live. Call it once per tick, after
    /// the plugin's own idle work, so the page hears what that tick changed.
    ///
    /// Thread: UI. [`Dispatcher`] shows one.
    pub fn tick(&mut self) {
        if !self.live {
            return;
        }
        for index in 0..self.params.len() {
            if self.params[index].watch.poll().is_some() {
                self.post_state(index);
            }
        }
    }

    /// Posts an event to the page: something that happened and has no
    /// current value to hold. Nothing is posted while no page session is
    /// live, since no page is listening.
    ///
    /// Thread: UI.
    ///
    /// # Examples
    ///
    /// ```
    /// # use seam_params::{Kind, ParamDecl, ParamSet};
    /// use seam_bridge::{Commands, Dispatcher};
    ///
    /// # let set = ParamSet::from_decls([ParamDecl::new("gain", "Gain", Kind::Bool, 0.0)])?;
    /// # let ui = set.take_ui().expect("the first UI handle");
    /// let mut native = Dispatcher::new(ui, Commands::new(), Vec::<String>::new());
    /// native.event("learned", serde_json::json!({ "controller": 74 }));
    /// assert!(native.outbox_mut().is_empty(), "no page yet");
    ///
    /// native.receive(r#"{"type":"ready","protocol":2}"#);
    /// native.event("learned", serde_json::json!({ "controller": 74 }));
    /// assert_eq!(
    ///     native.outbox_mut().last().map(String::as_str),
    ///     Some(r#"{"type":"event","name":"learned","payload":{"controller":74}}"#),
    /// );
    /// # Ok::<(), seam_params::DeclError>(())
    /// ```
    pub fn event(&mut self, name: &str, payload: Json) {
        if self.live {
            self.post(&NativeMessage::Event {
                name: name.to_owned(),
                payload,
            });
        }
    }

    /// Ends the page session: the gestures the page left open close, once,
    /// and nothing is posted until the page's next `ready`.
    ///
    /// A shell calls it when its page goes away and the dispatcher stays, as
    /// an editor's does when its window closes and may open again: the
    /// commands stay with the dispatcher. Dropping the dispatcher does the
    /// same.
    ///
    /// Thread: UI.
    ///
    /// # Examples
    ///
    /// ```
    /// # use seam_params::{Kind, ParamDecl, ParamSet};
    /// use seam_bridge::{Commands, Dispatcher};
    ///
    /// # let set = ParamSet::from_decls([ParamDecl::new("gain", "Gain", Kind::Bool, 0.0)])?;
    /// # let gain = set.id("gain").expect("declared");
    /// # let ui = set.take_ui().expect("the first UI handle");
    /// let mut native = Dispatcher::new(ui.clone(), Commands::new(), Vec::<String>::new());
    /// native.receive(r#"{"type":"ready","protocol":2}"#);
    /// native.receive(r#"{"type":"gestureBegin","param":"gain"}"#);
    /// assert!(ui.is_open(gain));
    ///
    /// // The editor closes mid-drag: the drag ends for the host.
    /// native.end_session();
    /// assert!(!ui.is_open(gain));
    /// # Ok::<(), seam_params::DeclError>(())
    /// ```
    pub fn end_session(&mut self) {
        for listed in &mut self.params {
            listed.gestures.clear();
        }
        self.live = false;
    }

    /// Returns what it refused since it was made. [`Refusals`] shows them.
    pub fn refusals(&self) -> Refusals {
        self.refusals
    }

    /// Returns the outbox it posts to, as a test reads back what the page
    /// was sent. [`Dispatcher`] shows one.
    pub fn outbox_mut(&mut self) -> &mut O {
        &mut self.outbox
    }

    /// Ends the page session before it, then answers: a snapshot that starts
    /// the next, or a reject.
    fn ready(&mut self, protocol: u32) {
        for listed in &mut self.params {
            listed.gestures.clear();
        }
        if protocol != PROTOCOL_VERSION {
            self.live = false;
            self.post(&NativeMessage::Reject {
                protocol: PROTOCOL_VERSION,
                message: format!(
                    "this plugin speaks protocol {PROTOCOL_VERSION} and the page {protocol}"
                ),
            });
            return;
        }
        // Seeded after the gestures closed, so each starts from the store.
        for listed in &mut self.params {
            listed.watch = self.ui.watch(listed.id);
        }
        self.live = true;
        let params = (0..self.params.len())
            .map(|index| self.state(index))
            .collect();
        self.post(&NativeMessage::Snapshot {
            protocol: PROTOCOL_VERSION,
            params,
        });
    }

    /// The index of the listed parameter `key` names, or `None`, counted, for
    /// a key the plugin does not list.
    fn listed(&mut self, key: &str) -> Option<usize> {
        let index = self.ui.set().id(key).map(ParamId::index);
        if index.is_none() {
            self.refusals.unknown_params += 1;
        }
        index
    }

    /// Writes the position the page set: inside the page's open gesture on
    /// it, or as a gesture of its own.
    fn set(&self, index: usize, normalized: Position) {
        let listed = &self.params[index];
        let value = self.decl(listed.id).host_value(normalized.get());
        match listed.gestures.last() {
            Some(gesture) => {
                gesture.write(value);
            }
            None => {
                self.ui.write(listed.id, value);
            }
        }
    }

    /// Answers a write with the state the parameter's watch shows once
    /// polled, changed or not.
    fn answer(&mut self, index: usize) {
        self.params[index].watch.poll();
        self.post_state(index);
    }

    fn invoke(&mut self, call: u32, command: &str, args: Json) {
        let result = match self.commands.call(command, args) {
            Some(Ok(value)) => CallResult::Ok { value },
            Some(Err(Some(message))) => CallResult::Err { message },
            Some(Err(None)) => {
                self.refusals.panicked_commands += 1;
                CallResult::Err {
                    message: format!("the command {command:?} panicked"),
                }
            }
            None => CallResult::Err {
                message: format!("the plugin has no command {command:?}"),
            },
        };
        self.post(&NativeMessage::Reply { call, result });
    }

    fn post_state(&mut self, index: usize) {
        let state = self.state(index);
        self.post(&NativeMessage::Param { state });
    }

    /// What the page shows for a listed parameter: its watch's value.
    fn state(&self, index: usize) -> ParamState {
        let listed = &self.params[index];
        let decl = self.decl(listed.id);
        let value = listed.watch.value();
        ParamState {
            param: decl.key.to_owned(),
            normalized: Position::clamped(decl.kind.normalize(value)),
            text: text(decl, value),
        }
    }

    fn decl(&self, id: ParamId) -> &ParamDecl {
        self.ui.set().decl(id)
    }

    fn post(&mut self, message: &NativeMessage) {
        self.outbox.post(&message.encode());
    }
}

/// A value as the page shows it: as the host formats it, then the unit.
fn text(decl: &ParamDecl, value: f32) -> String {
    let mut text = decl.format_real(value);
    if !decl.unit.is_empty() {
        text.push(' ');
        text.push_str(decl.unit);
    }
    text
}

impl<O> std::fmt::Debug for Dispatcher<O> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dispatcher")
            .field("live", &self.live)
            .field("refusals", &self.refusals)
            .finish_non_exhaustive()
    }
}
