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
        Ok(())
    }
}

/// One value [`Parts::save`](super::Parts::save) registered.
pub(super) struct SavedValue<T> {
    value: Value<T>,
    /// The value's change count when the session last had it.
    seen: u64,
    registration: Registration,
    /// The session holds its participant weakly, so this keeps it alive.
    _section: Rc<RefCell<ValueSection<T>>>,
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
        }));
        let registration = session.register(key, &section)?;
        Ok(SavedValue {
            value: value.clone(),
            seen: value.changes(),
            registration,
            _section: section,
        })
    }
}

impl<T> Synced for SavedValue<T>
where
    T: Clone,
{
    fn sync(&mut self) {
        if self.value.changes() == self.seen {
            return;
        }
        // The count and the origin from one lock, so a write after it moves
        // the count again and the next sync sees it.
        let reading = self.value.read();
        self.seen = reading.changes;
        // The session's apply pass captured a restore's write as it applied
        // it. Capturing it again would make a load look like an edit.
        if reading.origin != Origin::Restore {
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
