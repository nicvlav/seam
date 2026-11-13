//! Parameters a plugin makes while it runs: declaring them under a scope,
//! and the tick's steps for them around a load.

use std::rc::Weak;
use std::sync::Arc;

use seam_params::{Declaration, Declarer, ParamRef, Params};

use super::Ticked;
use crate::SavedState;
use crate::state::Assigned;

/// Makes parameters while a plugin runs, as [`Parts::declare`] and
/// [`Parts::add`] do, for code that cannot reach the [`Parts`]: a session
/// participant whose apply brings a tree of owners back after a load.
///
/// [`Parts::maker`] makes one. Clones share one instance's parts. It stays
/// on the main thread, and once the plugin's parts are dropped it still
/// makes parameters, with nothing to learn them.
///
/// # Examples
///
/// A tree saves its owners' ids, and its apply declares each again: the
/// parameters come back holding their saved values.
///
/// ```
#[doc = include_str!("../../doc/gain.rs")]
/// use std::cell::RefCell;
/// use std::rc::Rc;
///
/// use seam_host::{Driver, Maker};
/// use seam_session::{ApplyError, Participant, Section};
/// use serde_json::json;
///
/// struct Group {
///     volume: Float,
/// }
///
/// impl Params for Group {
///     fn declare(p: &mut Declarer) -> Self {
///         Group { volume: p.add(FloatDecl::new("volume", "Volume", -60.0..=12.0, 0.0)) }
///     }
/// }
///
/// /// The groups by id; each group's parameters are saved under its scope.
/// struct Tree {
///     groups: Vec<(u64, Group)>,
///     maker: Maker,
/// }
///
/// impl Participant for Tree {
///     fn capture(&self) -> Section {
///         json!(self.groups.iter().map(|(id, _)| *id).collect::<Vec<_>>())
///     }
///
///     fn apply(&mut self, section: Option<&Section>) -> Result<(), ApplyError> {
///         let ids = section.and_then(Section::as_array).map_or(&[][..], Vec::as_slice);
///         self.groups = ids
///             .iter()
///             .filter_map(serde_json::Value::as_u64)
///             .map(|id| (id, self.maker.declare(&format!("group/{id}"))))
///             .collect();
///         Ok(())
///     }
/// }
///
/// let make = || -> Result<_, Box<dyn std::error::Error>> {
///     let (_params, parts) = Parts::for_plugin::<Gain>()?;
///     let tree = Rc::new(RefCell::new(Tree { groups: Vec::new(), maker: parts.maker() }));
///     let registration = parts.session().register("tree", &tree)?;
///     Ok((parts, tree, registration))
/// };
///
/// let (parts, tree, registration) = make()?;
/// let group: Group = parts.declare("group/7");
/// parts.ui().write(&group.volume, -6.0);
/// tree.borrow_mut().groups.push((7, group));
/// registration.changed();
/// let block = parts.saved().save();
///
/// // A second instance loads the block; its tick applies the tree.
/// let (loaded, tree, _registration) = make()?;
/// loaded.saved().load(&block)?;
/// Driver::of(&loaded).tick();
/// let (id, group) = &tree.borrow().groups[0];
/// assert_eq!((*id, group.volume.get()), (7, -6.0));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// [`Parts`]: super::Parts
/// [`Parts::declare`]: super::Parts::declare
/// [`Parts::add`]: super::Parts::add
/// [`Parts::maker`]: super::Parts::maker
#[derive(Debug, Clone)]
pub struct Maker {
    ticked: Weak<Ticked>,
    saved: Arc<SavedState>,
}

impl Maker {
    pub(super) fn new(ticked: Weak<Ticked>, saved: Arc<SavedState>) -> Maker {
        Maker { ticked, saved }
    }

    /// Makes `P`'s parameters under `scope`, as
    /// [`Parts::declare`](super::Parts::declare) says.
    ///
    /// Thread: main.
    ///
    /// # Panics
    ///
    /// As [`Parts::declare`](super::Parts::declare) says.
    pub fn declare<P: Params>(&self, scope: &str) -> P {
        let (held, loads) = self.saved.run_time().held_values(scope);
        let (params, made) = Declarer::run_time::<P>(self.saved.set(), held);
        self.name(scope, &made, loads);
        params
    }

    /// Makes one parameter under `scope`, as
    /// [`Parts::add`](super::Parts::add) says.
    ///
    /// Thread: main.
    ///
    /// # Panics
    ///
    /// As [`Parts::declare`](super::Parts::declare) says.
    pub fn add<D: Declaration>(&self, scope: &str, decl: D) -> D::Param {
        let (held, loads) = self.saved.run_time().held_values(scope);
        let (param, made) = Declarer::run_time_one(self.saved.set(), held, decl);
        self.name(scope, &made, loads);
        param
    }

    /// Names each parameter made under its path, its declare having seen
    /// `loads` loads. Learn drops the mapping of each parameter whose path a
    /// new one took. A load's mapping for a new one waits for the load's
    /// pass to finish, as its other mappings do.
    fn name(&self, scope: &str, made: &[ParamRef], loads: u64) {
        // Borrowed first, so a declare while the plugin holds learn panics
        // whether or not a path is taken.
        let ticked = self.ticked.upgrade();
        let mut learn = ticked
            .as_ref()
            .and_then(|ticked| ticked.learn.get())
            .map(|learn| learn.borrow_mut());
        let clock = self.saved.set().clock().reader();
        let applied = clock.applying().then(|| clock.stamp().completed());
        let replaced = self.saved.run_time().name(scope, made, loads, applied);
        if let Some(learn) = learn.as_deref_mut() {
            for param in &replaced {
                learn.unmap(param);
            }
        }
    }
}

impl Ticked {
    /// Once a pass that began after a load stored its sections has finished,
    /// between the session's tick and the history's, assigns the load in one
    /// step. Each parameter made at run time that was declared before the
    /// load and is still held takes the load's value for its path, or its
    /// default, as a restore. Every one still held takes the load's mapping
    /// for its path; one declared before it with none held is unmapped. Then
    /// the load's entries go.
    ///
    /// The load itself wrote none of them: on the host's thread, they are the
    /// tree the load replaces, still playing. Learn's restore in that pass
    /// cleared their mappings, whichever of the plugin's tree and learn
    /// applied first, so no mapping here is refused as full for one of the
    /// tree the load replaced.
    pub(super) fn assign_held(&self) {
        let Some(assigned) = self.saved.run_time().settle(self.session.covered()) else {
            return;
        };
        // Made at the first value, since making it marks a restore stored.
        let mut load = None;
        let mut learn = self.learn.get().map(|learn| learn.borrow_mut());
        for Assigned {
            param,
            value,
            learned,
            ..
        } in assigned
        {
            if let Some(value) = value {
                load.get_or_insert_with(|| self.saved.set().program().load())
                    .restore_run_time(&param, value);
            }
            let Some(learn) = learn.as_deref_mut() else {
                continue;
            };
            match learned {
                // Refused only when learn is full: the mapping is lost, as
                // one in learn's own section would be.
                Some(mapping) => {
                    let _ = learn.map(&param, mapping);
                }
                None => {
                    learn.unmap(&param);
                }
            }
        }
    }

    /// At the end of a tick, when learn's mappings moved since the last
    /// time: hands a save the mappings of the parameters made at run time.
    /// Captured at the tick, as a saved value is, and its cost is the
    /// mappings, only when they moved.
    pub(super) fn capture_learned(&self) {
        let Some(learn) = self.learn.get() else {
            return;
        };
        let learn = learn.borrow();
        if self.learned_at.get() == Some(learn.revision()) {
            return;
        }
        self.learned_at.set(Some(learn.revision()));
        self.saved
            .run_time()
            .set_all_learned(learn.run_time_mappings());
    }
}
