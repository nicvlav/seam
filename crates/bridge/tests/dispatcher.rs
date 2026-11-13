//! The native side of the protocol, driven as a page drives it: each rule the
//! `Dispatcher` keeps, with a buffer standing in for the page and the host's
//! event drain and an undo history showing what the plugin's other sides see.

use std::sync::Arc;

use seam_bridge::{
    CallResult, Commands, Dispatcher, NativeMessage, PROTOCOL_VERSION, ParamState, Refusals,
};
use seam_params::{
    HostEvent, HostEventDrain, Kind, ParamDecl, ParamId, ParamSet, UiParams, host_id,
};
use seam_undo::{Bound, History};
use serde_json::{Value, json};

const MODES: [&str; 3] = ["Clean", "Warm", "Hot"];

/// A plugin's side of one page: its parameters, the handle its page writes
/// through, the host's drain, an undo history of both parameters, and the
/// native side under test.
struct Plugin {
    set: Arc<ParamSet>,
    ui: UiParams,
    host: HostEventDrain,
    history: History,
    native: Dispatcher<Vec<String>>,
}

impl Plugin {
    fn new() -> Self {
        Self::with(Commands::new())
    }

    fn with(commands: Commands) -> Self {
        let set = ParamSet::from_decls([
            ParamDecl::new(
                "gain",
                "Gain",
                Kind::continuous(-60.0, 12.0, 0.1, None),
                0.0,
            )
            .with_unit("dB"),
            ParamDecl::new("mode", "Mode", Kind::Choice(&MODES), 0.0),
        ])
        .expect("valid declarations");
        let ui = set.take_ui().expect("the first UI handle");
        let host = set.take_host_event_drain().expect("the first drain");
        let history = History::new(&ui, Bound::default()).expect("the first history");
        for id in set.ids() {
            history.enroll_param(id).expect("a listed parameter");
        }
        let native = Dispatcher::new(ui.clone(), commands, Vec::new());
        Plugin {
            set,
            ui,
            host,
            history,
            native,
        }
    }

    fn gain(&self) -> ParamId {
        self.set.id("gain").expect("declared")
    }

    /// Hands the native side what the page sent.
    fn page_sends(&mut self, message: Value) {
        self.native.receive(&message.to_string());
    }

    /// What the page was sent since the last call.
    fn page_got(&mut self) -> Vec<NativeMessage> {
        std::mem::take(self.native.outbox_mut())
            .iter()
            .map(|text| serde_json::from_str(text).expect("a native message of this protocol"))
            .collect()
    }

    /// A page that is listening: its `ready` answered, and the answer taken.
    fn listening() -> Self {
        let mut plugin = Self::new();
        plugin.page_sends(ready(PROTOCOL_VERSION));
        assert!(matches!(
            plugin.page_got()[..],
            [NativeMessage::Snapshot { .. }]
        ));
        plugin
    }

    /// What the host was told since the last call.
    fn host_heard(&mut self) -> Vec<HostEvent> {
        let mut heard = Vec::new();
        self.host.drain(|event| heard.push(event));
        heard
    }
}

fn ready(protocol: u32) -> Value {
    json!({ "type": "ready", "protocol": protocol })
}

fn begin(param: &str) -> Value {
    json!({ "type": "gestureBegin", "param": param })
}

fn set(param: &str, normalized: f64) -> Value {
    json!({ "type": "set", "param": param, "normalized": normalized })
}

fn end(param: &str) -> Value {
    json!({ "type": "gestureEnd", "param": param })
}

fn invoke(call: u32, command: &str, args: Value) -> Value {
    json!({ "type": "invoke", "call": call, "command": command, "args": args })
}

/// The state of `param` the page was sent, as `normalized` and `text`.
fn param(param: &str, normalized: f64, text: &str) -> NativeMessage {
    let state: ParamState = serde_json::from_value(json!({
        "param": param, "normalized": normalized, "text": text,
    }))
    .expect("a parameter state");
    NativeMessage::Param { state }
}

fn begins_and_ends(heard: &[HostEvent]) -> (usize, usize) {
    let begins = heard
        .iter()
        .filter(|event| matches!(event, HostEvent::Begin(_)))
        .count();
    let ends = heard
        .iter()
        .filter(|event| matches!(event, HostEvent::End(_)))
        .count();
    (begins, ends)
}

#[test]
fn a_ready_of_this_version_is_answered_with_every_parameter_in_order() {
    let mut plugin = Plugin::new();
    plugin.ui.write(plugin.gain(), -12.0);
    plugin.page_sends(ready(PROTOCOL_VERSION));
    let expected: NativeMessage = serde_json::from_value(json!({
        "type": "snapshot",
        "protocol": PROTOCOL_VERSION,
        "params": [
            { "param": "gain", "normalized": 48.0 / 72.0, "text": "-12 dB" },
            { "param": "mode", "normalized": 0.0, "text": "Clean" },
        ],
    }))
    .expect("a snapshot");
    assert_eq!(plugin.page_got(), [expected]);
}

#[test]
fn a_ready_of_another_version_is_rejected_with_this_sides_version() {
    for protocol in [PROTOCOL_VERSION - 1, PROTOCOL_VERSION + 1, u32::MAX] {
        let mut plugin = Plugin::new();
        plugin.page_sends(ready(protocol));
        let [
            NativeMessage::Reject {
                protocol: native,
                message,
            },
        ] = &plugin.page_got()[..]
        else {
            panic!("a reject for protocol {protocol}");
        };
        assert_eq!(*native, PROTOCOL_VERSION);
        assert!(message.contains(&protocol.to_string()), "{message}");
        // A rejected page is not listening: no change is posted to it.
        plugin.set.audio().host_write(plugin.gain(), 1.0);
        plugin.native.tick();
        assert!(plugin.page_got().is_empty());
    }
}

#[test]
fn a_reject_ends_the_session_before_it_too() {
    let mut plugin = Plugin::listening();
    plugin.page_sends(begin("gain"));
    plugin.page_sends(ready(PROTOCOL_VERSION + 1));
    assert!(!plugin.ui.is_open(plugin.gain()));
    let (begins, ends) = begins_and_ends(&plugin.host_heard());
    assert_eq!((begins, ends), (1, 1));
}

#[test]
fn every_set_and_gesture_end_is_answered_changed_or_not() {
    let mut plugin = Plugin::listening();
    plugin.page_sends(set("gain", 0.75));
    plugin.page_sends(set("gain", 0.75));
    plugin.page_sends(end("gain")); // with no gesture open
    let answer = param("gain", 0.75, "-6 dB");
    assert_eq!(plugin.page_got(), [answer.clone(), answer.clone(), answer]);
}

#[test]
fn a_set_is_snapped_and_answered_with_the_snapped_value() {
    let mut plugin = Plugin::listening();
    // 0.4 of the mode's three choices rounds to the second, at 0.5.
    plugin.page_sends(set("mode", 0.4));
    assert_eq!(plugin.page_got(), [param("mode", 0.5, "Warm")]);
    let mode = plugin.set.id("mode").expect("declared");
    assert_eq!(plugin.set.value(mode), 1.0);
}

#[test]
fn a_set_outside_a_drag_is_one_host_gesture_and_one_undo_step() {
    let mut plugin = Plugin::listening();
    plugin.page_sends(set("gain", 0.75));
    let heard = plugin.host_heard();
    let gain = plugin.gain();
    assert_eq!(
        heard,
        [
            HostEvent::Begin(gain),
            HostEvent::Value {
                id: gain,
                normalized: 0.75
            },
            HostEvent::End(gain),
        ]
    );
    plugin.history.tick();
    assert_eq!(plugin.history.len(), 1);
    // A set that changes nothing opens no gesture and records nothing.
    plugin.page_sends(set("gain", 0.75));
    assert!(plugin.host_heard().is_empty());
    plugin.history.tick();
    assert_eq!(plugin.history.len(), 1);
}

#[test]
fn a_drag_is_one_host_gesture_and_one_undo_step_however_many_sets() {
    let mut plugin = Plugin::listening();
    plugin.page_sends(begin("gain"));
    for normalized in [0.6, 0.7, 0.75] {
        plugin.page_sends(set("gain", normalized));
    }
    assert!(plugin.ui.is_open(plugin.gain()), "open until the end");
    plugin.page_sends(end("gain"));
    let (begins, ends) = begins_and_ends(&plugin.host_heard());
    assert_eq!((begins, ends), (1, 1));
    plugin.history.tick();
    assert_eq!(plugin.history.len(), 1);
    assert_eq!(plugin.set.value(plugin.gain()), -6.0);
    // The undo replays to before the drag, and the next tick tells the page.
    assert_eq!(plugin.history.undo(), Ok(true));
    plugin.page_got();
    plugin.native.tick();
    assert_eq!(plugin.page_got(), [param("gain", 60.0 / 72.0, "0 dB")]);
}

#[test]
fn two_gestures_on_one_parameter_reach_the_host_as_one() {
    let mut plugin = Plugin::listening();
    // Two controls on the page drag the gain at once.
    plugin.page_sends(begin("gain"));
    plugin.page_sends(begin("gain"));
    plugin.page_sends(set("gain", 0.75));
    plugin.page_sends(end("gain"));
    assert!(
        plugin.ui.is_open(plugin.gain()),
        "the second control's drag"
    );
    plugin.page_sends(end("gain"));
    assert!(!plugin.ui.is_open(plugin.gain()));
    let (begins, ends) = begins_and_ends(&plugin.host_heard());
    assert_eq!((begins, ends), (1, 1));
    // Each end is answered.
    let answers = plugin.page_got();
    assert_eq!(answers.len(), 3, "{answers:?}");
}

#[test]
fn a_repeated_ready_mid_drag_closes_the_drag_once_and_answers_again() {
    let mut plugin = Plugin::listening();
    plugin.page_sends(begin("gain"));
    plugin.page_sends(set("gain", 0.75));
    plugin.page_got();
    // A reload: the new page announces itself mid-drag.
    plugin.page_sends(ready(PROTOCOL_VERSION));
    assert!(!plugin.ui.is_open(plugin.gain()));
    let got = plugin.page_got();
    let [NativeMessage::Snapshot { params, .. }] = &got[..] else {
        panic!("a snapshot, got {got:?}");
    };
    assert_eq!(params[0].text, "-6 dB", "the drag's value stands");
    // The old page's end comes late: it is answered, and closes nothing more.
    plugin.page_sends(end("gain"));
    assert_eq!(plugin.page_got(), [param("gain", 0.75, "-6 dB")]);
    let (begins, ends) = begins_and_ends(&plugin.host_heard());
    assert_eq!((begins, ends), (1, 1));
    plugin.history.tick();
    assert_eq!(plugin.history.len(), 1, "the drag is one step");
}

#[test]
fn a_load_that_lands_mid_drag_reaches_the_page_at_the_drags_end() {
    let mut plugin = Plugin::listening();
    let gain = plugin.gain();
    plugin.page_sends(begin("gain"));
    plugin.page_sends(set("gain", 0.75));
    plugin.page_got();
    // The host loads a project while the page drags.
    plugin.set.program().load().restore(gain, 3.0);
    assert_eq!(plugin.set.value(gain), 3.0);
    // The page keeps the drag: the tick holds the load back.
    plugin.native.tick();
    assert!(plugin.page_got().is_empty());
    // A set during the drag writes over the load, as a user's would.
    plugin.page_sends(set("gain", 0.5));
    assert_eq!(plugin.page_got(), [param("gain", 0.5, "-24 dB")]);
    assert_eq!(plugin.set.value(gain), -24.0);
    plugin.set.program().load().restore(gain, 3.0);
    // The end is answered with the store: the load's value.
    plugin.page_sends(end("gain"));
    assert_eq!(plugin.page_got(), [param("gain", 63.0 / 72.0, "3 dB")]);
    plugin.native.tick();
    assert!(plugin.page_got().is_empty(), "posted once");
}

#[test]
fn a_change_by_the_host_by_undo_and_by_a_load_reaches_the_page_at_the_next_tick() {
    let mut plugin = Plugin::listening();
    let gain = plugin.gain();
    plugin.native.tick();
    assert!(plugin.page_got().is_empty(), "nothing moved");

    plugin.set.audio().host_write(gain, 0.75);
    plugin.native.tick();
    assert_eq!(plugin.page_got(), [param("gain", 0.75, "-6 dB")]);

    plugin.ui.write(gain, -12.0); // another surface, recorded by undo
    plugin.history.tick();
    plugin.native.tick();
    assert_eq!(plugin.page_got(), [param("gain", 48.0 / 72.0, "-12 dB")]);
    assert_eq!(plugin.history.undo(), Ok(true));
    plugin.native.tick();
    assert_eq!(plugin.page_got(), [param("gain", 0.75, "-6 dB")]);

    plugin.set.program().load().restore(gain, 3.0);
    plugin.native.tick();
    assert_eq!(plugin.page_got(), [param("gain", 63.0 / 72.0, "3 dB")]);

    // Many writes between two ticks reach the page as the latest, once.
    for normalized in [0.1, 0.2, 0.3] {
        plugin.set.audio().host_write(gain, normalized);
    }
    plugin.native.tick();
    assert_eq!(plugin.page_got().len(), 1);
}

#[test]
fn nothing_is_posted_before_the_first_ready() {
    let mut plugin = Plugin::new();
    plugin.set.audio().host_write(plugin.gain(), 1.0);
    plugin.native.tick();
    assert!(plugin.page_got().is_empty());
}

#[test]
fn a_write_naming_a_key_the_plugin_does_not_list_is_dropped_and_counted() {
    let mut plugin = Plugin::listening();
    plugin.page_sends(begin("volume"));
    plugin.page_sends(set("volume", 0.5));
    plugin.page_sends(end("volume"));
    assert!(plugin.page_got().is_empty());
    assert!(plugin.host_heard().is_empty());
    assert_eq!(
        plugin.native.refusals(),
        Refusals {
            unknown_params: 3,
            ..Refusals::default()
        }
    );
}

#[test]
fn dropping_it_closes_the_gestures_the_page_left_open() {
    let mut plugin = Plugin::listening();
    plugin.page_sends(begin("gain"));
    plugin.page_sends(set("gain", 0.75));
    assert!(plugin.ui.is_open(plugin.gain()));
    let native = std::mem::replace(
        &mut plugin.native,
        Dispatcher::new(plugin.ui.clone(), Commands::new(), Vec::new()),
    );
    drop(native); // the editor closed mid-drag
    assert!(!plugin.ui.is_open(plugin.gain()));
    let (begins, ends) = begins_and_ends(&plugin.host_heard());
    assert_eq!((begins, ends), (1, 1));
    plugin.history.tick();
    assert_eq!(plugin.history.len(), 1);
}

#[test]
fn an_ended_session_closes_its_gestures_and_posts_nothing_until_a_ready() {
    let mut plugin = Plugin::listening();
    plugin.page_sends(begin("gain"));
    plugin.page_sends(set("gain", 0.75));
    plugin.page_got();
    // The editor closes, and may open again with the same dispatcher.
    plugin.native.end_session();
    assert!(!plugin.ui.is_open(plugin.gain()));
    let (begins, ends) = begins_and_ends(&plugin.host_heard());
    assert_eq!((begins, ends), (1, 1));
    plugin.set.audio().host_write(plugin.gain(), 1.0);
    plugin.native.tick();
    assert!(plugin.page_got().is_empty(), "no page is listening");
    // The page of the next open starts a session of its own.
    plugin.page_sends(ready(PROTOCOL_VERSION));
    let got = plugin.page_got();
    let [NativeMessage::Snapshot { params, .. }] = &got[..] else {
        panic!("a snapshot, got {got:?}");
    };
    assert_eq!(params[0].text, "12 dB");
    plugin.native.tick();
    assert!(plugin.page_got().is_empty(), "the snapshot showed it");
}

/// The commands the fixture's calls name, and two more.
fn commands() -> Commands {
    Commands::new()
        .command("rename", |args: Value| {
            Ok(json!({ "renamed": args["to"] == "Lead" }))
        })
        .command("undo", |()| Ok(()))
        .command("refuse", |()| Err::<(), _>("refused".to_owned()))
        .command("break", |()| -> Result<(), String> { panic!("a bug") })
}

fn reply(plugin: &mut Plugin) -> (u32, CallResult) {
    let got = plugin.page_got();
    let [NativeMessage::Reply { call, result }] = &got[..] else {
        panic!("one reply, got {got:?}");
    };
    (*call, result.clone())
}

#[test]
fn a_call_is_answered_with_its_commands_result_under_its_call_id() {
    let mut plugin = Plugin::with(commands());
    plugin.page_sends(ready(PROTOCOL_VERSION));
    plugin.page_got();
    plugin.page_sends(invoke(7, "rename", json!({ "to": "Lead" })));
    let ok = CallResult::Ok {
        value: json!({ "renamed": true }),
    };
    assert_eq!(reply(&mut plugin), (7, ok));
    plugin.page_sends(json!({ "type": "invoke", "call": 8, "command": "undo" }));
    let nothing = CallResult::Ok { value: Value::Null };
    assert_eq!(reply(&mut plugin), (8, nothing));
}

#[test]
fn a_call_that_fails_is_answered_with_why() {
    let mut plugin = Plugin::with(commands());
    let failed = |plugin: &mut Plugin, call: u32, command: &str, args: Value| {
        plugin.page_sends(invoke(call, command, args));
        match reply(plugin) {
            (answered, CallResult::Err { message }) if answered == call => message,
            other => panic!("an error for call {call}, got {other:?}"),
        }
    };
    let unknown = failed(&mut plugin, 1, "teleport", Value::Null);
    assert!(unknown.contains("teleport"), "{unknown}");
    let mismatched = failed(&mut plugin, 2, "undo", json!([1, 2]));
    assert!(mismatched.contains("arguments"), "{mismatched}");
    assert_eq!(failed(&mut plugin, 3, "refuse", Value::Null), "refused");
    assert_eq!(plugin.native.refusals(), Refusals::default());

    let panicked = failed(&mut plugin, 4, "break", Value::Null);
    assert!(panicked.contains("panicked"), "{panicked}");
    assert_eq!(plugin.native.refusals().panicked_commands, 1);
    // It still answers after a panic.
    assert_eq!(
        failed(&mut plugin, 5, "break", Value::Null),
        panicked,
        "the same answer"
    );
    assert_eq!(plugin.native.refusals().panicked_commands, 2);
}

#[test]
#[should_panic(expected = "is added twice")]
fn a_command_added_twice_panics() {
    let _ = Commands::new()
        .command("undo", |()| Ok(()))
        .command("undo", |()| Ok(()));
}

/// The messages both sides' tests share.
fn fixture() -> Value {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/messages.json");
    let text = std::fs::read_to_string(path).expect("the shared fixture");
    serde_json::from_str(&text).expect("the fixture is JSON")
}

fn entries<'a>(fixture: &'a Value, list: &str) -> &'a [Value] {
    let entries = fixture[list]
        .as_array()
        .unwrap_or_else(|| panic!("fixture list {list}"));
    assert!(!entries.is_empty(), "fixture list {list} is empty");
    entries
}

#[test]
fn the_fixtures_page_messages_are_answered_as_the_rules_say() {
    let fixture = fixture();
    let mut plugin = Plugin::with(commands());
    let mut answered = Vec::new();
    for entry in entries(&fixture, "page") {
        plugin.page_sends(entry["message"].clone());
        let got = plugin.page_got();
        let name = entry["name"].as_str().expect("a name");
        answered.push((name.to_owned(), got));
    }
    let kinds: Vec<(&str, Vec<&str>)> = answered
        .iter()
        .map(|(name, got)| {
            let kinds = got
                .iter()
                .map(|message| match message {
                    NativeMessage::Snapshot { .. } => "snapshot",
                    NativeMessage::Reject { .. } => "reject",
                    NativeMessage::Param { .. } => "param",
                    NativeMessage::Reply { .. } => "reply",
                    NativeMessage::Event { .. } => "event",
                })
                .collect();
            (name.as_str(), kinds)
        })
        .collect();
    assert_eq!(
        kinds,
        [
            ("ready", vec!["snapshot"]),
            ("gesture begin", vec![]),
            ("set", vec!["param"]),
            ("gesture end", vec!["param"]),
            ("invoke with arguments", vec!["reply"]),
            ("invoke with null arguments", vec!["reply"]),
        ]
    );
    assert_eq!(plugin.native.refusals(), Refusals::default());
    // The fixture's set reached the gain, through the host's own conversion.
    let gain = plugin.gain();
    assert_eq!(plugin.set.host_id(gain), host_id("gain"));
    assert_eq!(plugin.set.value(gain), -42.0);
}

#[test]
fn what_the_native_side_refuses_is_dropped_unanswered_and_counted() {
    let fixture = fixture();
    let refused = entries(&fixture, "refusedByNative");
    let mut plugin = Plugin::listening();
    for entry in refused {
        let text = entry["text"].as_str().expect("text");
        plugin.native.receive(text);
        assert!(plugin.page_got().is_empty(), "{}", entry["name"]);
    }
    assert!(plugin.host_heard().is_empty());
    assert_eq!(
        plugin.native.refusals(),
        Refusals {
            malformed: refused.len() as u64,
            ..Refusals::default()
        }
    );
}

#[test]
fn what_the_native_side_tolerates_is_handled() {
    let fixture = fixture();
    let mut plugin = Plugin::with(commands());
    for entry in entries(&fixture, "tolerated") {
        let text = entry["text"].as_str().expect("text");
        plugin.native.receive(text);
    }
    assert_eq!(plugin.native.refusals().malformed, 0);
}
