//! What a save writes and a load holds for the parameters a plugin makes at
//! run time.
//!
//! Each is saved under its path, its owner's scope and its key
//! (`group/17/volume`), its value read at the save as a listed parameter's
//! is, and its learned mapping beside it. The parameter declared last under
//! a path is the one a save writes.
//!
//! A load holds its entries, for the declares its pass makes, until a pass
//! that began after the load stored its sections has finished. The restore
//! clock's stored count orders the two: the entries record it after the
//! load's parameter writes, the session's store moves it, and the session
//! says which count its last finished pass covered. When that pass has
//! finished, the tick assigns the load in one step: each parameter declared
//! before it takes its value, and every live one its mapping.
//!
//! The registry counts loads under its own lock, and each name records the
//! count its declare saw, so a parameter declared before a load is told
//! from one declared since, a declare the load landed inside included.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use seam_learn::{Mapping, decode_mapping, encode_mapping};
use seam_params::{HeldValues, ParamRef, WeakParamRef};
use serde_json::{Map, Value as Json};

use super::{VALUE_BYTES, json_bytes, key_bytes, value_json, value_number};

/// The key the block keeps them under.
pub(crate) const KEY: &str = "runTime";

/// `held_at` with no entries held.
const NONE_HELD: u64 = u64::MAX;

/// The fewest names a sweep waits for.
const FIRST_SWEEP: usize = 64;

/// Returns `scope/key`, or the key alone in the empty scope.
pub(crate) fn path(scope: &str, key: &str) -> String {
    if scope.is_empty() {
        key.to_owned()
    } else {
        format!("{scope}/{key}")
    }
}

/// Every parameter made at run time that has a name, and the entries the
/// last load holds. Shared by the main thread, which names parameters and
/// ticks, and whichever thread saves or loads.
#[derive(Debug)]
pub(crate) struct RunTimeSaved {
    inner: Mutex<Inner>,
    /// The stored count the held entries recorded, or [`NONE_HELD`], so a
    /// tick knows without the lock whether a finished pass leaves it work.
    /// A hint: written under the lock, and confirmed under it.
    held_at: AtomicU64,
    /// Two parameters the plugin still holds were declared under one path.
    shared_path: AtomicBool,
    /// A save left entries out for want of room.
    no_room: AtomicBool,
}

#[derive(Debug, Default)]
struct Inner {
    /// Each path's parameter: the one declared there last.
    paths: BTreeMap<Arc<str>, u64>,
    /// Every named parameter not swept yet, by its store's serial, one a
    /// later declare took its path from included.
    named: HashMap<u64, Named>,
    /// `named`'s size at which the next new name sweeps it.
    sweep_at: usize,
    /// The learned mapping of each path's parameter, as a save writes it.
    learned: HashMap<Arc<str>, Json>,
    /// What the last load held, until the pass that applies it finishes.
    held: Option<Held>,
    /// How many loads have held entries.
    loads: u64,
}

#[derive(Debug)]
struct Named {
    path: Arc<str>,
    param: WeakParamRef,
    /// How many loads its declare saw: below the held entries' count, it
    /// was declared before their load.
    loads: u64,
    /// The restore clock's count of finished passes, when it was declared
    /// inside a restore's apply.
    applied: Option<u64>,
}

impl Named {
    /// The parameter, while the plugin holds it.
    fn held(&self) -> Option<ParamRef> {
        self.param.upgrade().filter(ParamRef::held)
    }

    /// Whether a declare that saw `loads` loads, in an apply at `applied`,
    /// takes this one's path in a way that is no fault: in a restore's
    /// apply, from a parameter not declared in an apply of the same pass.
    /// A tree coming back declares the paths of the tree it replaces, while
    /// the plugin still holds that tree.
    fn replaced_by_a_restore(&self, loads: u64, applied: Option<u64>) -> bool {
        applied.is_some() && (self.applied, self.loads) != (applied, loads)
    }
}

/// A load's entries, from its `runTime` object.
#[derive(Debug, Clone, Default)]
pub(crate) struct Loaded {
    entries: BTreeMap<String, Entry>,
}

#[derive(Debug, Clone)]
struct Held {
    /// The restore clock's stored count after the load's parameter writes.
    at: u64,
    /// The registry's count of loads, this one included.
    loads: u64,
    entries: Arc<BTreeMap<String, Entry>>,
}

impl Held {
    /// The mapping held for `path`.
    fn learned(&self, path: &str) -> Option<Mapping> {
        self.entries.get(path)?.learned
    }
}

/// One path's entry as a load read it.
#[derive(Debug, Clone)]
struct Entry {
    /// A bool or a number, as loaded.
    value: Option<Json>,
    learned: Option<Mapping>,
}

impl Entry {
    /// The value a parameter is born holding, before its declaration makes
    /// it legal.
    fn number(&self) -> Option<f32> {
        self.value.as_ref().and_then(value_number)
    }
}

/// What a save writes, as [`RunTimeSaved::to_write`] copies it.
struct ToWrite {
    held: Option<Held>,
    live: Vec<Live>,
}

/// A parameter a save writes live.
struct Live {
    path: Arc<str>,
    param: ParamRef,
    /// The mapping the held entries give it when their pass finishes.
    held: Option<Mapping>,
    /// Its mapping as the last tick captured it.
    learned: Option<Json>,
}

/// The bytes the section's frame takes: `,"runTime":{` and its `}`.
const FRAME_BYTES: usize = 13;

/// The most bytes one entry takes in the section, with its value at the
/// longest a save writes, and the `,` after it.
fn entry_bytes(path: &str, learned: Option<&Json>) -> usize {
    // `"path":{"value":` and the value, `,"learned":` and the mapping, `}`.
    let learned = learned.map_or(0, |learned| 11 + json_bytes(learned));
    key_bytes(path) + 10 + VALUE_BYTES + learned + 2
}

impl Loaded {
    /// Reads a block's `runTime` object. A value that is not a bool or a
    /// number, and a mapping that does not read, are dropped; an entry left
    /// with neither is too.
    pub(crate) fn read(section: Option<Json>) -> Loaded {
        let Some(Json::Object(section)) = section else {
            return Loaded::default();
        };
        let entries = section
            .into_iter()
            .filter_map(|(path, entry)| {
                let entry = entry.as_object()?;
                let value = entry
                    .get("value")
                    .filter(|value| value.is_boolean() || value.is_number())
                    .cloned();
                let learned = entry
                    .get("learned")
                    .and_then(decode_mapping)
                    .map(|(_, mapping)| mapping);
                (value.is_some() || learned.is_some()).then_some((path, Entry { value, learned }))
            })
            .collect();
        Loaded { entries }
    }

    /// The most bytes these entries take when a save writes them back.
    pub(crate) fn bytes(&self) -> usize {
        if self.entries.is_empty() {
            return 0;
        }
        let entries: usize = self
            .entries
            .iter()
            .map(|(path, entry)| {
                let learned = entry
                    .learned
                    .as_ref()
                    .map(|mapping| encode_mapping(path, mapping));
                entry_bytes(path, learned.as_ref())
            })
            .sum();
        FRAME_BYTES + entries
    }
}

impl Default for RunTimeSaved {
    fn default() -> Self {
        RunTimeSaved {
            inner: Mutex::default(),
            held_at: AtomicU64::new(NONE_HELD),
            shared_path: AtomicBool::new(false),
            no_room: AtomicBool::new(false),
        }
    }
}

impl RunTimeSaved {
    /// Never poisoned for good: no plugin code runs under it.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The values the last load holds under `scope`, for a declare there,
    /// and the count of loads they come from, which the declare's
    /// [`name`](Self::name) records. The lookup owns what it reads, since
    /// the plugin's code runs while the declarer holds it.
    pub(crate) fn held_values(&self, scope: &str) -> (HeldValues, u64) {
        let inner = self.lock();
        let loads = inner.loads;
        let Some(held) = inner.held.clone() else {
            return (Box::new(|_| None), loads);
        };
        drop(inner);
        let scope = scope.to_owned();
        let lookup = move |key: &str| held.entries.get(&path(&scope, key))?.number();
        (Box::new(lookup), loads)
    }

    /// Names each of `made` under `scope`, its declare having seen `loads`
    /// loads, inside a restore's apply when `applied` holds the clock's
    /// count of finished passes. Each takes its path from the parameter
    /// declared there before, which it hands back for learn to unmap.
    ///
    /// A path taken from a parameter the plugin still holds is the shared
    /// path fault, unless a restore's apply takes it from one not declared
    /// in an apply of the same pass: a tree coming back declares the paths
    /// of the tree it replaces.
    pub(crate) fn name(
        &self,
        scope: &str,
        made: &[ParamRef],
        loads: u64,
        applied: Option<u64>,
    ) -> Vec<ParamRef> {
        let mut replaced = Vec::new();
        let mut inner = self.lock();
        for param in made {
            let path: Arc<str> = path(scope, param.decl().key).into();
            if let Some(old) = inner.paths.get(&path).and_then(|old| inner.named.get(old))
                && let Some(old_param) = old.held()
            {
                if !old.replaced_by_a_restore(loads, applied) {
                    self.shared_path.store(true, Ordering::Relaxed);
                }
                replaced.push(old_param);
            }
            // Mapped when a load's pass finishes, or by the plugin: never
            // at its declare.
            inner.learned.remove(&path);
            inner.paths.insert(Arc::clone(&path), param.serial());
            inner.named.insert(
                param.serial(),
                Named {
                    path,
                    param: param.downgrade(),
                    loads,
                    applied,
                },
            );
        }
        if inner.named.len() >= inner.sweep_at {
            inner.sweep();
        }
        drop(inner);
        replaced
    }

    /// Replaces every learned mapping a save writes with `mappings`, those
    /// of parameters declared last under their paths.
    pub(crate) fn set_all_learned<'a>(
        &self,
        mappings: impl Iterator<Item = (&'a ParamRef, &'a Mapping)>,
    ) {
        let mut inner = self.lock();
        let learned = mappings
            .filter_map(|(param, mapping)| {
                let path = inner.path_of(param)?;
                let entry = encode_mapping(&path, mapping);
                Some((path, entry))
            })
            .collect();
        inner.learned = learned;
    }

    /// Holds `loaded`, replacing what an earlier load held, recorded at the
    /// stored count `at` and as one more load.
    pub(crate) fn hold(&self, loaded: Loaded, at: u64) {
        let mut inner = self.lock();
        inner.loads += 1;
        inner.held = Some(Held {
            at,
            loads: inner.loads,
            entries: Arc::new(loaded.entries),
        });
        self.held_at.store(at, Ordering::Release);
    }

    /// Lets the held entries go once a pass that covers a count past theirs
    /// has finished, and hands back the load to assign, path by path, for
    /// every parameter the plugin still holds:
    ///
    /// - one declared before the load takes the value held for its path, or
    ///   its default, and the mapping held, or none;
    /// - one declared since was born holding its value, and takes the
    ///   mapping held for its path, when there is one.
    ///
    /// A save from here on writes each path's mapping as handed back.
    pub(crate) fn settle(&self, covered: u64) -> Option<Vec<Assigned>> {
        let at = self.held_at.load(Ordering::Acquire);
        if at == NONE_HELD || covered <= at {
            return None;
        }
        let mut inner = self.lock();
        let held = inner.held.take_if(|held| covered > held.at)?;
        self.held_at.store(NONE_HELD, Ordering::Release);
        let assigned: Vec<Assigned> = inner
            .paths
            .iter()
            .filter_map(|(path, serial)| {
                let named = inner.named.get(serial)?;
                let param = named.held()?;
                let learned = held.learned(path);
                let value = if named.loads < held.loads {
                    let value = held.entries.get(&**path).and_then(Entry::number);
                    Some(value.unwrap_or_else(|| param.decl().default_value()))
                } else if learned.is_some() {
                    None
                } else {
                    return None;
                };
                Some(Assigned {
                    path: Arc::clone(path),
                    param,
                    value,
                    learned,
                })
            })
            .collect();
        for Assigned { path, learned, .. } in &assigned {
            match learned {
                Some(mapping) => {
                    let entry = encode_mapping(path, mapping);
                    inner.learned.insert(Arc::clone(path), entry);
                }
                None => {
                    inner.learned.remove(path);
                }
            }
        }
        drop(inner);
        Some(assigned)
    }

    /// Writes the section's entry into `block`, `,"runTime":{...}`, in at
    /// most `room` bytes, or nothing when it would hold no entry.
    ///
    /// With no load's entries held, it writes each path whose parameter the
    /// plugin holds, its value read now. While a load's are held, it writes
    /// each of them as loaded, and the live value only for a path declared
    /// since that load, with the mapping the load holds for it: a parameter
    /// declared before it still holds the state from before the load, and
    /// the load's pass maps each one when it finishes. Entries are taken in
    /// path order; one with no room is left out, and the room fault is
    /// flagged.
    pub(crate) fn write(&self, block: &mut String, room: usize) {
        let ToWrite { held, live } = self.to_write();
        // Formatted outside the lock, so a declare on the main thread waits
        // only while the parameters are copied.
        let mut entries: BTreeMap<&str, (Option<Json>, Option<Json>)> = BTreeMap::new();
        for (path, entry) in held.iter().flat_map(|held| held.entries.iter()) {
            let learned = entry
                .learned
                .as_ref()
                .map(|mapping| encode_mapping(path, mapping));
            entries.insert(path, (entry.value.clone(), learned));
        }
        for Live {
            path,
            param,
            held,
            learned,
        } in &live
        {
            let value = value_json(param.decl(), param.value());
            let learned = held
                .map(|mapping| encode_mapping(path, &mapping))
                .or_else(|| learned.clone());
            entries.insert(path, (Some(value), learned));
        }
        let mut used = FRAME_BYTES;
        let mut written = Vec::with_capacity(entries.len());
        for (path, (value, learned)) in entries {
            let bytes = entry_bytes(path, learned.as_ref());
            if used + bytes > room {
                self.no_room.store(true, Ordering::Relaxed);
                continue;
            }
            used += bytes;
            let mut entry = Map::new();
            if let Some(learned) = learned {
                entry.insert("learned".into(), learned);
            }
            if let Some(value) = value {
                entry.insert("value".into(), value);
            }
            written.push(format!("{}:{}", Json::from(path), Json::Object(entry)));
        }
        if written.is_empty() {
            return;
        }
        block.push_str(",\"");
        block.push_str(KEY);
        block.push_str("\":{");
        block.push_str(&written.join(","));
        block.push('}');
    }

    /// What a save writes, copied under the lock: the load's entries while
    /// they are held, and each path's parameter that the plugin holds and no
    /// held entry stands for, with its learned mapping.
    fn to_write(&self) -> ToWrite {
        let inner = self.lock();
        let held = inner.held.as_ref();
        let live = inner
            .paths
            .iter()
            .filter_map(|(path, serial)| {
                let named = inner.named.get(serial)?;
                if held.is_some_and(|held| named.loads < held.loads) {
                    return None;
                }
                Some(Live {
                    path: Arc::clone(path),
                    param: named.held()?,
                    held: held.and_then(|held| held.learned(path)),
                    learned: inner.learned.get(path).cloned(),
                })
            })
            .collect();
        ToWrite {
            held: inner.held.clone(),
            live,
        }
    }

    /// Takes the faults flagged since the last call: two parameters the
    /// plugin held declared under one path, and a save left short.
    pub(crate) fn take_faults(&self) -> (bool, bool) {
        (
            self.shared_path.swap(false, Ordering::Relaxed),
            self.no_room.swap(false, Ordering::Relaxed),
        )
    }
}

/// What a load assigns a parameter when its pass finishes.
#[derive(Debug)]
pub(crate) struct Assigned {
    path: Arc<str>,
    pub(crate) param: ParamRef,
    /// The value it takes as a restore: the one held for its path, or its
    /// default. `None` for one declared since the load, born holding it.
    pub(crate) value: Option<f32>,
    /// The mapping held for its path, or `None` for none.
    pub(crate) learned: Option<Mapping>,
}

impl Inner {
    /// `param`'s path, while it is the parameter declared there.
    fn path_of(&self, param: &ParamRef) -> Option<Arc<str>> {
        let named = self.named.get(&param.serial())?;
        let current = self.paths.get(&named.path)?;
        (*current == param.serial()).then(|| Arc::clone(&named.path))
    }

    /// Drops every name whose parameter the plugin holds no longer, and
    /// each path left with none. Run as a new name doubles them since the
    /// last sweep, so its cost is shared among the names, never paid by a
    /// tick.
    fn sweep(&mut self) {
        self.named.retain(|_, named| named.held().is_some());
        let named = &self.named;
        self.paths.retain(|_, serial| named.contains_key(serial));
        let paths = &self.paths;
        self.learned.retain(|path, _| paths.contains_key(path));
        self.sweep_at = (self.named.len() * 2).max(FIRST_SWEEP);
    }
}

#[cfg(test)]
mod tests;
