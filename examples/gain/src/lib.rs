//! The example gain: a gain in dB, a bypass, a mode with three names, a label
//! that is saved and undoable, and a gain that a MIDI controller can be
//! learned to.
//!
//! It shows a plugin that uses the parts Seam builds for it: parameters,
//! saved state, undo, the session, learn and the VST3 export. The guide
//! `docs/first-plugin.md` walks through it.
//!
//! Open: no exporter delivers MIDI events yet, so learn works only in a test
//! that runs learn's audio side itself.

#[cfg(feature = "vst3")]
seam_host::export_vst3!(Gain);

use seam_host::{
    AuCodes, Block, Category, Layout, Parts, Plugin, PluginInfo, ProcessSetup, Processor, Status,
};
use seam_learn::{Config, LearnError, Takes};
use seam_params::{Bool, Changes, Choice, Choices, Float, Params, UiParams, Value};
use seam_undo::{Bound, History};

const LABEL_KEY: &str = "label";
const DEFAULT_LABEL: &str = "Untitled";

/// What each mode does to the sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Choices)]
enum Mode {
    /// Gain only.
    Clean,
    /// Gain, then a soft clip.
    Warm,
    /// Gain with three times the drive, then a soft clip.
    Hot,
}

/// The parameters, in the order hosts list them, each declared where it is
/// held.
#[derive(Debug, Clone, Params)]
pub struct GainParams {
    #[param(
        "gain", "Gain", -60.0..=12.0, 0.0,
        step = 0.1, unit = "dB",
    )]
    gain: Float,
    #[param("mode", "Mode", Mode::Clean)]
    mode: Choice<Mode>,
    #[param("bypass", "Bypass", false, bypass)]
    bypass: Bool,
}

/// The plugin, on the main thread.
#[derive(Debug)]
pub struct Gain {
    parts: Parts,
    params: GainParams,
    history: History,
    label: Value<String>,
}

impl Gain {
    /// The label now.
    pub fn label(&self) -> String {
        self.label.get()
    }

    /// Renames the instance: one undo step, saved with the plugin from the
    /// next tick.
    pub fn set_label(&self, label: &str) {
        self.label.set(label.to_owned());
    }

    /// The parameter handle for an editor's writes, which undo records.
    pub fn ui(&self) -> &UiParams {
        self.parts.ui()
    }

    /// The undo history of the parameters and the label.
    pub fn history(&self) -> &History {
        &self.history
    }

    /// Arms the gain for MIDI learn: the next controller a block brings
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
        parts.midi_learn(Config::default());

        let label = parts
            .saved_value(LABEL_KEY, DEFAULT_LABEL.to_owned())
            .expect("a free key");
        history.enroll(&label, "Rename").expect("a new value");

        Gain {
            parts,
            params,
            history,
            label,
        }
    }

    fn activate(&mut self, _setup: &ProcessSetup) -> GainProcessor {
        GainProcessor::new(self.params.clone())
    }
}

/// The audio-thread half: a gain and a mode over the first two channels.
#[derive(Debug)]
pub struct GainProcessor {
    params: GainParams,
    /// What the fields below were last read from.
    changes: Changes,
    gain: f32,
    mode: Mode,
    bypassed: bool,
}

impl GainProcessor {
    fn new(params: GainParams) -> Self {
        let mut processor = GainProcessor {
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
