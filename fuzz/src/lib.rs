//! A plugin for the fuzz targets that load saved state: parameters of every
//! kind and two session participants, one that takes strings only and one
//! that takes anything. The plugin keeps its session handle, and
//! [`Plugin::idle`] ticks it, so a target that ticks after a load makes the
//! restored sections reach the participants.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use seam_host::{
    Block, Category, Layout, Plugin, PluginInfo, ProcessSetup, Processor, SavedState, Status,
};
use seam_params::{Kind, ParamDecl, ParamSetBuilder};
use seam_session::{ApplyError, Participant, Registration, Section, Session, SessionUi};

const MODES: &[&str] = &["low", "band", "high"];

const fn decl(key: &'static str, kind: Kind, default: f32) -> ParamDecl {
    ParamDecl::new(key, key, kind, default)
}

/// A participant that takes a string and refuses anything else.
struct Notes(String);

impl Participant for Notes {
    fn capture(&self) -> Section {
        if self.0.is_empty() {
            Section::Null
        } else {
            Section::String(self.0.clone())
        }
    }

    fn apply(&mut self, section: Option<&Section>) -> Result<(), ApplyError> {
        self.0 = match section {
            None => String::new(),
            Some(Section::String(text)) => text.clone(),
            Some(_) => return Err(ApplyError::new("not a string")),
        };
        Ok(())
    }
}

/// A participant that takes whatever it is handed.
struct Anything(Section);

impl Participant for Anything {
    fn capture(&self) -> Section {
        self.0.clone()
    }

    fn apply(&mut self, section: Option<&Section>) -> Result<(), ApplyError> {
        self.0 = section.cloned().unwrap_or(Section::Null);
        Ok(())
    }
}

/// Passes audio through.
#[derive(Debug)]
pub struct Silent;

impl Processor for Silent {
    fn process(&mut self, _block: Block<'_, '_>) -> Status {
        Status::Continue
    }
}

/// A participant with the registration that keeps it in the session.
struct Held<P> {
    participant: Rc<RefCell<P>>,
    registration: Registration,
}

impl<P: Participant + 'static> Held<P> {
    fn new(ui: &SessionUi, key: &str, participant: P) -> Self {
        let participant = Rc::new(RefCell::new(participant));
        let registration = ui.register(key, &participant).expect("a free key");
        Self {
            participant,
            registration,
        }
    }
}

/// The fuzzed plugin.
pub struct Fuzzed {
    saved: Arc<SavedState>,
    /// Held so the registrations stay live, and ticked from `idle`.
    ui: SessionUi,
    notes: Held<Notes>,
    _anything: Held<Anything>,
}

impl Fuzzed {
    /// The text the notes participant holds.
    pub fn notes(&self) -> String {
        self.notes.participant.borrow().0.clone()
    }

    /// Sets the notes the way a plugin's own code does, and tells the
    /// session.
    pub fn set_notes(&self, text: &str) {
        self.notes.participant.borrow_mut().0 = text.to_owned();
        self.notes.registration.changed();
    }
}

impl Plugin for Fuzzed {
    const INFO: PluginInfo = PluginInfo {
        name: "Fuzzed",
        vendor: "Seam",
        url: "",
        email: "",
        description: "",
        version: "0.0.0",
        id: "dev.seam.fuzzed",
        vst3_class_id: [0; 16],
        au_codes: (*b"Fuzz", *b"Seam"),
        category: Category::Effect,
        features: &[],
        layouts: &[Layout::STEREO],
        midi_input: false,
        midi_output: false,
    };
    const PARAMS: &'static [ParamDecl] = &[
        decl(
            "freq",
            Kind::continuous(20.0, 20_000.0, 0.0, Some(1_000.0)),
            1_000.0,
        ),
        decl("gain", Kind::continuous(-24.0, 24.0, 0.0, None), 0.0),
        decl("on", Kind::Bool, 1.0),
        decl("mode", Kind::Choice(MODES), 1.0),
        decl("steps", Kind::continuous(0.0, 10.0, 3.0, None), 3.0),
    ];
    type Processor = Silent;

    fn new() -> Self {
        let mut builder = ParamSetBuilder::new();
        for &decl in Self::PARAMS {
            builder.add(decl).expect("valid");
        }
        let set = builder.build();
        let session = Session::new(&set);
        let ui = session.take_ui().expect("the first UI handle");
        Fuzzed {
            saved: Arc::new(SavedState::new(&set, &session)),
            notes: Held::new(&ui, "notes", Notes(String::new())),
            _anything: Held::new(&ui, "anything", Anything(Section::Null)),
            ui,
        }
    }

    fn saved(&self) -> Arc<SavedState> {
        Arc::clone(&self.saved)
    }

    fn idle(&mut self) {
        self.ui.tick();
    }

    fn activate(&mut self, _setup: &ProcessSetup) -> Silent {
        Silent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loaded_section_reaches_its_participant_after_a_tick() {
        let mut first = Fuzzed::new();
        first.set_notes("hello");
        first.idle();
        let block = first.saved().save();

        let mut second = Fuzzed::new();
        assert!(second.saved().load(&block).is_ok());
        second.idle();
        assert_eq!(second.notes(), "hello");
    }
}
