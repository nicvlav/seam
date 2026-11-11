//! The example gain: a gain in dB, a bypass, a mode with three names, a label
//! that is saved and undoable, and a gain that a MIDI controller can be
//! learned to.
//!
//! It shows a plugin that uses the parts Seam builds for it: parameters,
//! saved state, undo, the session, learn and the VST3 export. The guide
//! `docs/first-plugin.md` walks through it.
//!
//! Open: no exporter delivers MIDI events yet, so learn works only when a
//! test drives the processor directly.

#[cfg(feature = "vst3")]
seam_host::export_vst3!(Gain);

use std::cell::RefCell;
use std::rc::Rc;

use seam_host::{
    AuCodes, Block, Category, Layout, Parts, Plugin, PluginInfo, ProcessSetup, Processor, Status,
};
use seam_learn::{Config, LearnAudio, LearnError, Takes};
use seam_params::{
    Bool, BoolDecl, Changes, Choice, ChoiceDecl, Choices, Declarer, Float, FloatDecl, Params,
    UiParams, Value,
};
use seam_session::{ApplyError, Participant, Registration, Section};
use seam_undo::{Bound, History, HistoryError};

const LABEL_KEY: &str = "label";
const DEFAULT_LABEL: &str = "Untitled";

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

impl Choices for Mode {
    const LABELS: &'static [&'static str] = &["Clean", "Warm", "Hot"];

    fn from_index(index: usize) -> Self {
        match index {
            0 => Mode::Clean,
            1 => Mode::Warm,
            _ => Mode::Hot,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// The parameters, in the order hosts list them, each declared where it is
/// held.
#[derive(Debug, Clone)]
pub struct GainParams {
    gain: Float,
    mode: Choice<Mode>,
    bypass: Bool,
}

impl Params for GainParams {
    fn declare(p: &mut Declarer) -> Self {
        GainParams {
            gain: p.add(
                FloatDecl::new("gain", "Gain", -60.0..=12.0, 0.0)
                    .with_step(0.1)
                    .with_unit("dB"),
            ),
            mode: p.add(ChoiceDecl::new("mode", "Mode", Mode::Clean)),
            bypass: p.add(BoolDecl::new("bypass", "Bypass", false).as_bypass()),
        }
    }
}

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
    parts: Parts,
    params: GainParams,
    history: History,
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
        self.parts.params()
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
    /// As [`Learn::arm`](seam_learn::Learn::arm).
    pub fn learn_gain(&mut self) -> Result<(), LearnError> {
        let gain = self.params.gain.id();
        self.parts.learn().arm(gain, Takes::CONTROLLER)
    }

    /// Whether a controller is learned to the gain.
    pub fn gain_is_learned(&self) -> bool {
        self.parts.learn().mapping(self.params.gain.id()).is_some()
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
        au_codes: AuCodes {
            subtype: *b"SmGn",
            manufacturer: *b"Seam",
        },
        category: Category::Effect,
        features: &[],
        layouts: &[Layout::STEREO],
        midi_input: true,
        midi_output: false,
    };

    type Params = GainParams;

    type Processor = GainProcessor;

    fn new(params: GainParams, mut parts: Parts) -> Self {
        let history = parts.undo(Bound::default());
        parts
            .midi_learn(Config::default())
            .expect("learn's key is free");

        let label = Value::new(DEFAULT_LABEL.to_owned(), parts.set());
        history.enroll(&label, "Rename").expect("a new value");
        let label_section = Rc::new(RefCell::new(LabelSection(label.clone())));
        let label_registration = parts
            .session()
            .register(LABEL_KEY, &label_section)
            .expect("a free key");

        Gain {
            parts,
            params,
            history,
            label_seen: label.changes(),
            label,
            _label_section: label_section,
            label_registration,
        }
    }

    fn activate(&mut self, _setup: &ProcessSetup) -> GainProcessor {
        GainProcessor::new(self.params.clone(), self.parts.learn_audio())
    }

    fn idle(&mut self) {
        self.parts.tick();
        self.label_moved();
    }
}

/// The audio-thread half: a gain and a mode over the first two channels.
#[derive(Debug)]
pub struct GainProcessor {
    learn: LearnAudio,
    params: GainParams,
    /// What the fields below were last read from.
    changes: Changes,
    gain: f32,
    mode: Mode,
    bypassed: bool,
}

impl GainProcessor {
    fn new(params: GainParams, learn: LearnAudio) -> Self {
        let mut processor = GainProcessor {
            learn,
            params,
            changes: Changes::new(),
            gain: 1.0,
            mode: Mode::Clean,
            bypassed: false,
        };
        processor.retune();
        processor
    }

    /// Reads the parameters again when one moved. Atomics only.
    fn retune(&mut self) {
        let GainParams { gain, mode, bypass } = &self.params;
        if !self.changes.moved([gain, mode, bypass]) {
            return;
        }
        self.gain = 10.0_f32.powf(gain.get() / 20.0);
        self.mode = mode.get();
        self.bypassed = bypass.get();
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
