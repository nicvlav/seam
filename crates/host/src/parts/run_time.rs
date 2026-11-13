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
        let held = self.saved.run_time().held_values(scope);
        let (params, made) = Declarer::run_time::<P>(self.saved.set(), held);
        self.name(scope, &made);
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
        let held = self.saved.run_time().held_values(scope);
        let (param, made) = Declarer::run_time_one(self.saved.set(), held, decl);
        self.name(scope, &made);
        param
    }

    /// Names each parameter made under its path, and learns it to the
    /// controller a load holds for that path. Learn drops the mapping of
    /// each parameter whose path a new one took.
    fn name(&self, scope: &str, made: &[ParamRef]) {
        let ticked = self.ticked.upgrade();
        let mut learn = ticked
            .as_ref()
            .and_then(|ticked| ticked.learn.get())
            .map(|learn| learn.borrow_mut());
        let clock = self.saved.set().clock();
        let now = clock.writer().stored_count();
        let applying = clock.reader().applying();
        let run_time = self.saved.run_time();
        let naming = run_time.name(scope, made, now, applying);
        let Some(learn) = learn.as_deref_mut() else {
            return;
        };
        for param in &naming.replaced {
            learn.unmap(param);
        }
        for (param, mapping) in &naming.learned {
            // Refused only when learn is full: the mapping is lost, as one
            // in learn's own section would be.
            if learn.map(param, *mapping).is_ok() {
                run_time.set_learned(param, mapping);
            }
        }
    }
}

impl Ticked {
    /// At the first tick that sees a new load, before the session's: learn
    /// drops the mapping of every parameter made at run time declared before
    /// that load, so no mapping the load's pass makes is refused as full
    /// for one of the tree it replaces.
    pub(super) fn forget_replaced(&self) {
        let run_time = self.saved.run_time();
        let Some(at) = run_time.take_loaded() else {
            return;
        };
        let Some(learn) = self.learn.get() else {
            return;
        };
        let mut learn = learn.borrow_mut();
        let old: Vec<ParamRef> = learn
            .run_time_mappings()
            .map(|(param, _)| param)
            .filter(|param| run_time.declared_by(param, at))
            .cloned()
            .collect();
        for param in &old {
            learn.unmap(param);
        }
    }

    /// Once a pass that began after a load stored its sections has finished,
    /// between the session's tick and the history's: each parameter made at
    /// run time that was declared before the load and is still held takes
    /// the load's value for its path, or its default, as a restore, and the
    /// load's mapping for it, or none. Then the load's entries go.
    ///
    /// The load itself wrote none of them: on the host's thread, they are the
    /// tree the load replaces, still playing.
    pub(super) fn assign_held(&self) {
        let Some(assigned) = self.saved.run_time().settle(self.session.covered()) else {
            return;
        };
        if assigned.is_empty() {
            return;
        }
        let load = self.saved.set().program().load();
        let mut learn = self.learn.get().map(|learn| learn.borrow_mut());
        for Assigned {
            param,
            value,
            learned,
        } in assigned
        {
            load.restore_run_time(
                &param,
                value.unwrap_or_else(|| param.decl().default_value()),
            );
            let Some(learn) = learn.as_deref_mut() else {
                continue;
            };
            match learned {
                // Refused only when learn is full, as in a declare.
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
