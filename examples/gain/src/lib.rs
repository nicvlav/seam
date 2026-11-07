//! The example gain: a gain in dB, a bypass, a mode with three names, a label
//! that is saved and undoable, and a gain that a MIDI controller can be
//! learned to.
//!
//! It shows how a plugin joins Seam's parts: parameters, saved state, undo,
//! the session, learn and the VST3 export. The guide `docs/first-plugin.md`
//! walks through it.
//!
//! Open: no exporter delivers MIDI events yet, so learn works only when a
//! test drives the processor directly.

#[cfg(feature = "vst3")]
seam_host::export_vst3!(Gain);

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use seam_host::{
    Block, Category, Layout, Plugin, PluginInfo, ProcessSetup, Processor, SavedState, Status,
};
use seam_learn::{Config, Learn, LearnAudio, LearnError, Takes};
use seam_params::{AudioParams, Kind, ParamDecl, ParamId, ParamSet, UiParams, Value};
use seam_session::{ApplyError, Participant, Registration, Section, Session, SessionUi};
use seam_undo::{Bound, History, HistoryError};

/// The gain parameter's key.
pub const GAIN: &str = "gain";
/// The mode parameter's key.
pub const MODE: &str = "mode";
/// The bypass parameter's key.
pub const BYPASS: &str = "bypass";

const LABEL_KEY: &str = "label";
const DEFAULT_LABEL: &str = "Untitled";

/// Every parameter, in the order hosts list them.
const PARAMS: &[ParamDecl] = &[
    ParamDecl::new(GAIN, "Gain", Kind::continuous(-60.0, 12.0, 0.1, None), 0.0).with_unit("dB"),
    ParamDecl::new(MODE, "Mode", Kind::Choice(&["Clean", "Warm", "Hot"]), 0.0),
    ParamDecl::new(BYPASS, "Bypass", Kind::Bool, 0.0).as_bypass(),
];

/// The label as a session participant: it saves and restores the value.
#[derive(Debug)]
struct LabelSection(Value<String>);

impl Participant for LabelSection {
    fn capture(&self) -> Section {
        Section::String(self.0.get())
    }

    fn apply(&mut self, section: Option<&Section>) -> Result<(), ApplyError> {
        let label = match section {
            Some(section) => section
                .as_str()
                .ok_or_else(|| ApplyError::new("the label is not a string"))?,
            None => DEFAULT_LABEL,
        };
        self.0.set(label.to_owned());
        Ok(())
    }
}

/// The plugin, on the main thread.
#[derive(Debug)]
pub struct Gain {
    saved: Arc<SavedState>,
    ui: UiParams,
    session: SessionUi,
    history: History,
    learn: Rc<RefCell<Learn>>,
    label: Value<String>,
    /// The session holds its participant weakly, so the plugin keeps it alive.
    _label_section: Rc<RefCell<LabelSection>>,
    label_registration: Registration,
    label_seen: u64,
}

impl Gain {
    /// The label now.
    pub fn label(&self) -> String {
        self.label.get()
    }

    /// Renames the instance: one undo step, saved with the plugin.
    pub fn set_label(&mut self, label: &str) {
        self.label.set(label.to_owned());
        self.label_moved();
    }

    /// The parameter handle for an editor's writes, which undo records.
    pub fn ui(&self) -> &UiParams {
        &self.ui
    }

    /// Undoes the last step; false when there was none.
    ///
    /// # Errors
    ///
    /// As [`History::undo`]: refused while a gesture is open or a replay runs.
    pub fn undo(&mut self) -> Result<bool, HistoryError> {
        let undone = self.history.undo()?;
        self.label_moved();
        Ok(undone)
    }

    /// Redoes the last undone step; false when there was none.
    ///
    /// # Errors
    ///
    /// As [`History::redo`].
    pub fn redo(&mut self) -> Result<bool, HistoryError> {
        let redone = self.history.redo()?;
        self.label_moved();
        Ok(redone)
    }

    /// Arms the gain for MIDI learn: the next controller the processor sees
    /// drives it, once the next tick commits it.
    ///
    /// # Errors
    ///
    /// As [`Learn::arm`].
    pub fn learn_gain(&mut self) -> Result<(), LearnError> {
        let gain = declared(self.saved.set(), GAIN);
        self.learn.borrow_mut().arm(gain, Takes::CONTROLLER)
    }

    /// Whether a controller is learned to the gain.
    pub fn gain_is_learned(&self) -> bool {
        let gain = declared(self.saved.set(), GAIN);
        self.learn.borrow().mapping(gain).is_some()
    }

    /// Tells the session the label moved, so the next save writes it.
    fn label_moved(&mut self) {
        let changes = self.label.changes();
        if changes != self.label_seen {
            self.label_seen = changes;
            self.label_registration.changed();
        }
    }
}

fn declared(set: &ParamSet, key: &str) -> ParamId {
    set.id(key).expect("declared in PARAMS")
}

impl Plugin for Gain {
    const INFO: PluginInfo = PluginInfo {
        name: "Seam Example Gain",
        vendor: "Seam",
        url: "",
        email: "",
        description: "A gain with three modes, a label and MIDI learn",
        version: "0.1.0",
        id: "dev.seam.example-gain",
        vst3_class_id: 0x5b0a_d271_3e96_4c18_a37f_e024_8db5_6a19_u128.to_be_bytes(),
        au_codes: (*b"SmGn", *b"Seam"),
        category: Category::Effect,
        features: &[],
        layouts: &[Layout::STEREO],
        midi_input: true,
        midi_output: false,
    };

    const PARAMS: &'static [ParamDecl] = PARAMS;

    type Processor = GainProcessor;

    fn new() -> Self {
        let set = ParamSet::from_decls(PARAMS.iter().copied()).expect("valid declarations");
        let session = Session::new(&set);
        let saved = Arc::new(SavedState::new(&set, &session));
        let session_ui = session.take_ui().expect("the first session UI handle");
        let ui = set.take_ui().expect("the first UI handle");

        let history = History::new(&ui, Bound::default()).expect("the first history");
        history.enroll_params();
        let label = Value::new(DEFAULT_LABEL.to_owned(), &set);
        history.enroll(&label, "Rename").expect("a new value");
        let label_section = Rc::new(RefCell::new(LabelSection(label.clone())));
        let label_registration = session_ui
            .register(LABEL_KEY, &label_section)
            .expect("a free key");

        let (learn, _) = Learn::new(&set, Config::default());
        let learn = Rc::new(RefCell::new(learn));
        Learn::attach(&learn, &session_ui).expect("a free key");

        Gain {
            saved,
            ui,
            session: session_ui,
            history,
            learn,
            label_seen: label.changes(),
            label,
            _label_section: label_section,
            label_registration,
        }
    }

    fn saved(&self) -> Arc<SavedState> {
        Arc::clone(&self.saved)
    }

    fn activate(&mut self, _setup: &ProcessSetup) -> GainProcessor {
        let learn = self.learn.borrow_mut().replace_audio();
        GainProcessor::new(self.saved.set(), learn)
    }

    fn idle(&mut self) {
        // Learn first: a mapping its poll commits reaches the session at the
        // session's next tick, which is this one.
        let _learned = self.learn.borrow_mut().poll();
        self.session.tick();
        self.history.tick();
        self.label_moved();
    }
}

/// What each mode does to the sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Gain only.
    Clean,
    /// Gain, then a soft clip.
    Warm,
    /// Gain with three times the drive, then a soft clip.
    Hot,
}

/// The audio-thread half: a gain and a mode over the first two channels.
#[derive(Debug)]
pub struct GainProcessor {
    params: AudioParams,
    learn: LearnAudio,
    gain_id: ParamId,
    mode_id: ParamId,
    bypass_id: ParamId,
    /// The sum of the parameters' change counters when it last tuned.
    seen: Option<u64>,
    gain: f32,
    mode: Mode,
    bypassed: bool,
}

impl GainProcessor {
    fn new(set: &Arc<ParamSet>, learn: LearnAudio) -> Self {
        let mut processor = GainProcessor {
            params: set.audio(),
            learn,
            gain_id: declared(set, GAIN),
            mode_id: declared(set, MODE),
            bypass_id: declared(set, BYPASS),
            seen: None,
            gain: 1.0,
            mode: Mode::Clean,
            bypassed: false,
        };
        processor.retune();
        processor
    }

    /// Reads the parameters again when one moved. Atomics only.
    fn retune(&mut self) {
        let set = self.params.set();
        let seen = set
            .changes(self.gain_id)
            .wrapping_add(set.changes(self.mode_id))
            .wrapping_add(set.changes(self.bypass_id));
        if self.seen == Some(seen) {
            return;
        }
        self.seen = Some(seen);
        self.gain = 10.0_f32.powf(set.value(self.gain_id) / 20.0);
        self.mode = match set.value(self.mode_id) {
            mode if mode < 0.5 => Mode::Clean,
            mode if mode < 1.5 => Mode::Warm,
            _ => Mode::Hot,
        };
        self.bypassed = set.value(self.bypass_id) >= 0.5;
    }
}

impl Processor for GainProcessor {
    fn process(&mut self, block: Block<'_, '_>) -> Status {
        // Learned controllers write the gain first, so they apply in this
        // block.
        self.learn.process(block.events);
        self.retune();
        if self.bypassed {
            return Status::Continue;
        }
        let (gain, mode) = (self.gain, self.mode);
        for channel in block.channels.iter_mut().take(2) {
            for sample in channel.iter_mut() {
                *sample = match mode {
                    Mode::Clean => *sample * gain,
                    Mode::Warm => (*sample * gain).tanh(),
                    Mode::Hot => (*sample * gain * 3.0).tanh(),
                };
            }
        }
        Status::Continue
    }
}
