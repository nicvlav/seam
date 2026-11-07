//! Learn's place in a session: the section it saves, how a saved section
//! lands, and the participant that wires the two to the session.

use std::cell::RefCell;
use std::rc::Rc;

use seam_session::{ApplyError, Participant, Section, SessionError, SessionUi};
use serde_json::Value as Json;

use super::{Custom, Learn};
use crate::saved::{SCHEMA, SESSION_KEY, decode, encode};

impl Learn {
    /// Returns the `midiLearn` section: schema 1, one entry per mapping and
    /// per held entry, sorted by target key, each held entry as it was saved.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_learn::{Config, Control, Learn, Mapping, Source};
    #[doc = include_str!("../../doc/setup.rs")]
    /// let (mut learn, _audio) = Learn::new(&set, Config::default());
    /// learn.register("fader", Box::new(Fader))?;
    /// learn.map(gain, Mapping::new(Source::any(Control::Cc(7))))?;
    ///
    /// let saved = learn.save();
    /// assert_eq!(saved["schema"], 1);
    /// assert_eq!(saved["mappings"][0]["target"], "gain");
    /// // The unmapped fader has no entry.
    /// assert_eq!(saved["mappings"].as_array().map(Vec::len), Some(1));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn save(&self) -> Section {
        let mut entries: Vec<(&str, Section)> = self
            .mappings
            .iter()
            .filter_map(|(&target, mapping)| {
                let key = self.key(target)?;
                Some((key, encode(key, mapping)))
            })
            .chain(
                self.held
                    .iter()
                    .map(|(key, entry)| (key.as_str(), entry.clone())),
            )
            .collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        // Keys inserted in order, as `encode` does.
        let mut section = serde_json::Map::new();
        section.insert(
            "mappings".into(),
            Json::Array(entries.into_iter().map(|(_, entry)| entry).collect()),
        );
        section.insert("schema".into(), SCHEMA.into());
        Json::Object(section)
    }

    /// Replaces every mapping and every held entry from a saved section, in one
    /// publish, and ends any arm.
    ///
    /// `None` clears them: a restore is a complete assignment. A restore
    /// routes and never writes a value. An entry this build cannot read drops
    /// on its own, and so does one the checks of [`map`](Self::map) refuse,
    /// the ceiling included. An entry for a target not registered yet is
    /// held, saved back unchanged and applied when that target registers.
    ///
    /// A section of another schema, a newer build's say, cannot be read. The
    /// mappings still clear, so the previous project's stop routing, and
    /// learn reports no change, so the session saves that section back.
    ///
    /// # Errors
    ///
    /// Returns [`ApplyError`] for a section of another schema, after
    /// clearing.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_learn::{Control, Mapping, Source};
    #[doc = include_str!("../../doc/setup.rs")]
    /// # use seam_learn::Config;
    /// # use seam_learn::Learn;
    /// # let (mut learn, mut audio) = Learn::new(&set, Config::default());
    /// learn.map(gain, Mapping::new(Source::any(Control::Cc(7))))?;
    /// let saved = learn.save();
    ///
    /// // A fresh instance of the same plugin loads what the first saved.
    /// let (mut loaded, _audio) = Learn::new(&set, Config::default());
    /// loaded.restore(Some(&saved))?;
    /// assert_eq!(loaded.mapping(gain), learn.mapping(gain));
    ///
    /// // A project saved without the section clears the mappings.
    /// loaded.restore(None)?;
    /// assert!(loaded.is_empty());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn restore(&mut self, section: Option<&Section>) -> Result<(), ApplyError> {
        let readable =
            section.is_none_or(|s| s.get("schema").and_then(Json::as_u64) == Some(SCHEMA));
        self.arm.disarm();
        self.mappings.clear();
        self.held.clear();
        for custom in self.customs.values_mut() {
            custom.staging = None;
        }
        let outcome = if readable {
            self.insert_entries(section);
            Ok(())
        } else {
            Err(ApplyError::new("a midiLearn section of another schema"))
        };
        self.publish();
        if outcome.is_ok() {
            self.saved_changed();
        }
        // Target code runs last, once the audio thread can take the new
        // table, so a panicking `end` leaves no old mapping routing there.
        self.customs.values_mut().for_each(Custom::close);
        outcome
    }

    /// Puts each entry of a readable section in the table, or holds it for a
    /// target not registered yet. Unpublished.
    fn insert_entries(&mut self, section: Option<&Section>) {
        let entries = section
            .and_then(|section| section.get("mappings"))
            .and_then(Json::as_array)
            .map_or(&[][..], Vec::as_slice);
        for entry in entries {
            let Some((key, mapping)) = decode(entry) else {
                continue;
            };
            match self.target_for(&key) {
                Some(target) => {
                    if self.check(target, &mapping).is_ok() {
                        self.insert(target, mapping);
                    }
                }
                None => {
                    self.held.insert(key, entry.clone());
                }
            }
        }
    }

    /// Registers `learn` with the session as its `midiLearn` participant.
    ///
    /// From then on every change to what [`save`](Self::save) writes
    /// reaches the session, on its next tick when made inside learn's own
    /// methods, and a session restore applies through
    /// [`restore`](Self::restore). A section already waiting applies now.
    ///
    /// # Panics
    ///
    /// When `session` was made from another set than learn's: the plugin's
    /// saved block would never hold the mappings. Make the session from the
    /// same set.
    ///
    /// # Errors
    ///
    /// Returns a [`SessionError`] when the session refuses the registration.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::cell::RefCell;
    /// use std::rc::Rc;
    ///
    /// use seam_learn::{Config, Control, Learn, Mapping, SESSION_KEY, Source};
    /// use seam_session::Session;
    #[doc = include_str!("../../doc/setup.rs")]
    /// let session = Session::new(&set);
    /// let ui = session.take_ui().ok_or("the first UI handle")?;
    /// let (learn, _audio) = Learn::new(&set, Config::default());
    /// let learn = Rc::new(RefCell::new(learn));
    /// Learn::attach(&learn, &ui)?;
    ///
    /// // From now on the session saves the mappings with the plugin's state.
    /// let mapping = Mapping::new(Source::any(Control::Cc(7)));
    /// learn.borrow_mut().map(gain, mapping)?;
    /// ui.tick();
    /// let saved = session.save().ok_or("a section to save")?;
    /// assert_eq!(saved[SESSION_KEY]["mappings"][0]["target"], "gain");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn attach(learn: &Rc<RefCell<Learn>>, session: &SessionUi) -> Result<(), SessionError> {
        assert!(
            session.uses(&learn.borrow().set),
            "the session was made from another ParamSet; make the Session from the same ParamSet as Learn"
        );
        let registration = session.register(SESSION_KEY, learn)?;
        learn.borrow_mut().registration = Some(registration);
        Ok(())
    }

    /// Tells the session what `save` writes has changed.
    pub(super) fn saved_changed(&self) {
        if let Some(registration) = &self.registration {
            registration.changed();
        }
    }
}

impl Participant for Learn {
    fn capture(&self) -> Section {
        self.save()
    }

    fn apply(&mut self, section: Option<&Section>) -> Result<(), ApplyError> {
        self.restore(section)
    }
}
