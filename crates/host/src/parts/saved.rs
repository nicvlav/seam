//! A plain value saved with the session: the participant registered for it,
//! and the check each tick makes for a change to capture.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use seam_params::{Origin, Value};
use seam_session::{ApplyError, Participant, Registration, Section, SessionError, SessionUi};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// A saved value as the parts keep it, behind one type whatever its `T`.
pub(super) trait Synced: fmt::Debug {
    /// Captures the value when it moved since the last call.
    fn sync(&mut self);
}

/// Captures a value as JSON, and applies a restored section to it, or its
/// default when the restored project has no section.
struct ValueSection<T> {
    value: Value<T>,
    default: T,
    /// The change count its last apply's write left, which the session's
    /// apply pass captured; `None` before one, or when a write from another
    /// thread came after it.
    applied: Option<u64>,
}

impl<T> Participant for ValueSection<T>
where
    T: Clone + Serialize + DeserializeOwned,
{
    fn capture(&self) -> Section {
        let section = serde_json::to_value(self.value.get());
        debug_assert!(
            section.is_ok(),
            "serde cannot write a saved value as JSON: {section:?}"
        );
        section.unwrap_or(Section::Null)
    }

    fn apply(&mut self, section: Option<&Section>) -> Result<(), ApplyError> {
        let value = match section {
            Some(section) => {
                T::deserialize(section).map_err(|error| ApplyError::new(error.to_string()))?
            }
            None => self.default.clone(),
        };
        // Written while the restore applies, so undo records none of it.
        self.value.set(value);
        let reading = self.value.read();
        self.applied = (reading.origin == Origin::Restore).then_some(reading.changes);
        Ok(())
    }
}

/// One value [`Parts::saved_value`](super::Parts::saved_value) made.
pub(super) struct SavedValue<T> {
    value: Value<T>,
    /// The value's change count when the session last had it.
    seen: u64,
    registration: Registration,
    /// The session holds its participant weakly, so this keeps it alive.
    section: Rc<RefCell<ValueSection<T>>>,
}

impl<T> SavedValue<T>
where
    T: Clone + Serialize + DeserializeOwned + 'static,
{
    /// Registers `value` with `session` under `key`. A section waiting under
    /// the key applies before this returns.
    pub(super) fn register(
        session: &SessionUi,
        key: &str,
        value: &Value<T>,
        default: T,
    ) -> Result<SavedValue<T>, SessionError> {
        let section = Rc::new(RefCell::new(ValueSection {
            value: value.clone(),
            default,
            applied: None,
        }));
        let registration = session.register(key, &section)?;
        Ok(SavedValue {
            value: value.clone(),
            seen: value.changes(),
            registration,
            section,
        })
    }
}

impl<T> Synced for SavedValue<T>
where
    T: Clone,
{
    fn sync(&mut self) {
        let changes = self.value.changes();
        if changes == self.seen {
            return;
        }
        self.seen = changes;
        // The session's apply pass captured what this value's own apply
        // wrote, so a load is captured once. Any other write is captured,
        // one another participant's apply made after it included. The
        // section is borrowed only while it applies, which never syncs.
        let applied = self
            .section
            .try_borrow()
            .ok()
            .and_then(|section| section.applied);
        if applied != Some(changes) {
            self.registration.changed();
        }
    }
}

impl<T> fmt::Debug for SavedValue<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SavedValue")
            .field("key", &self.registration.key())
            .field("seen", &self.seen)
            .finish_non_exhaustive()
    }
}
