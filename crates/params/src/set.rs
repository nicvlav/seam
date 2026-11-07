//! The parameter set: declarations checked once, then frozen into one value
//! cell and one change counter per parameter.

// Audio-path code: a panic here ends a host, so indexing, unwrap, expect,
// panic and unchecked arithmetic are refused, and a site that is provably
// safe says why in an `#[expect]`.
#![cfg_attr(
    not(test),
    deny(
        clippy::indexing_slicing,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::arithmetic_side_effects
    )
)]

#[cfg(loom)]
use loom::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
#[cfg(not(loom))]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use value_cell::{Packed, ValueCell};

use crate::host_events::{HostEventDrain, HostQueue};
use crate::program::ProgramParams;
use crate::ui::UiParams;
use crate::{
    Kind, Origin, ParamDecl, ParamId, RESERVED_HOST_IDS, Reading, RestoreClock, audio_thread,
    host_id,
};

/// The longest key, in bytes: AAX truncates longer ones, which would merge
/// two keys that share a prefix.
pub const MAX_KEY_LEN: usize = 31;

/// Why a declaration was refused.
///
/// # Examples
///
/// ```
/// use seam_params::{DeclError, Kind, ParamDecl, ParamSet};
///
/// let error = ParamSet::from_decls([ParamDecl::new("a/b", "A", Kind::Bool, 0.0)]).unwrap_err();
/// assert_eq!(error, DeclError::KeyHasSlash("a/b"));
/// assert_eq!(error.to_string(), r#"key "a/b" contains '/'"#);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeclError {
    /// The key is empty.
    EmptyKey,
    /// The key contains `/`, which separates group names.
    KeyHasSlash(&'static str),
    /// The key is longer than [`MAX_KEY_LEN`] bytes.
    KeyTooLong(&'static str),
    /// Another parameter already has this key.
    DuplicateKey(&'static str),
    /// A key and a group share this name: keys and group names share one
    /// namespace.
    KeyIsGroup(&'static str),
    /// The group path has an empty name in it: a leading, trailing or
    /// doubled `/`.
    EmptyGroupName {
        /// The parameter declared with it.
        key: &'static str,
        /// The path.
        group: &'static str,
    },
    /// A choice needs at least two labels.
    TooFewChoices(&'static str),
    /// A choice's label is empty or only whitespace, so no typed text reads
    /// as it.
    EmptyChoiceLabel(&'static str),
    /// A choice's label starts or ends with whitespace. Typed text is trimmed
    /// before it is read, so no typed text reads as it either.
    PaddedChoiceLabel {
        /// The choice.
        key: &'static str,
        /// The label.
        label: &'static str,
    },
    /// Two of a choice's labels are equal, or differ only in ASCII case:
    /// [`ParamDecl::parse_real`] reads a label in any case, so text that
    /// matches both reads as the first.
    DuplicateChoiceLabel {
        /// The choice.
        key: &'static str,
        /// The later of the two labels.
        label: &'static str,
    },
    /// Another parameter is already the bypass.
    SecondBypass(&'static str),
    /// The bypass is not a bool: VST3 drives it as a toggle.
    BypassNotBool(&'static str),
    /// The key's host id falls in the range the VST3 exporter keeps for its
    /// own proxy parameters. Rename the key.
    ReservedHostId(&'static str),
    /// Two keys share one host id, so a host could not tell them apart.
    /// Rename one of them.
    HostIdCollision {
        /// The parameter declared second.
        key: &'static str,
        /// The parameter declared first.
        other: &'static str,
    },
}

impl fmt::Display for DeclError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeclError::EmptyKey => write!(f, "a parameter key is empty"),
            DeclError::KeyHasSlash(key) => write!(f, "key {key:?} contains '/'"),
            DeclError::KeyTooLong(key) => {
                write!(f, "key {key:?} is longer than {MAX_KEY_LEN} bytes")
            }
            DeclError::DuplicateKey(key) => write!(f, "key {key:?} is declared twice"),
            DeclError::KeyIsGroup(name) => {
                write!(f, "{name:?} names both a parameter and a group")
            }
            DeclError::EmptyGroupName { key, group } => {
                write!(f, "parameter {key:?} has an empty name in group {group:?}")
            }
            DeclError::TooFewChoices(key) => {
                write!(f, "choice {key:?} has fewer than two labels")
            }
            DeclError::EmptyChoiceLabel(key) => write!(f, "choice {key:?} has an empty label"),
            DeclError::PaddedChoiceLabel { key, label } => {
                write!(
                    f,
                    "choice {key:?} has the label {label:?}, with whitespace at an end"
                )
            }
            DeclError::DuplicateChoiceLabel { key, label } => {
                write!(
                    f,
                    "choice {key:?} has the label {label:?} twice, ignoring case"
                )
            }
            DeclError::SecondBypass(key) => {
                write!(f, "{key:?} would be a second bypass parameter")
            }
            DeclError::BypassNotBool(key) => write!(f, "bypass {key:?} is not a bool"),
            DeclError::ReservedHostId(key) => {
                write!(f, "key {key:?} has a host id the VST3 exporter reserves")
            }
            DeclError::HostIdCollision { key, other } => {
                write!(f, "keys {other:?} and {key:?} share one host id")
            }
        }
    }
}

impl std::error::Error for DeclError {}

/// Collects declarations and freezes them into a [`ParamSet`].
///
/// # Examples
///
/// ```
/// use seam_params::{Kind, ParamDecl, ParamSetBuilder};
///
/// let gain = ParamDecl::new("gain", "Gain", Kind::continuous(-60.0, 12.0, 0.0, None), 0.0)
///     .with_unit("dB");
/// let bypass = ParamDecl::new("bypass", "Bypass", Kind::Bool, 0.0).as_bypass();
///
/// let mut builder = ParamSetBuilder::new();
/// builder.add(gain)?.add(bypass)?;
/// let set = builder.build();
/// assert_eq!(set.len(), 2);
/// # Ok::<(), seam_params::DeclError>(())
/// ```
#[derive(Debug, Default)]
pub struct ParamSetBuilder {
    decls: Vec<ParamDecl>,
    keys: HashSet<&'static str>,
    groups: HashSet<&'static str>,
    host_ids: HashMap<u32, &'static str>,
    bypass: bool,
}

impl ParamSetBuilder {
    /// Creates a builder with no declarations.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one parameter after checking everything about it.
    ///
    /// On `Err` nothing changed, in every build, so later declarations go on
    /// as if this one had never been made.
    ///
    /// # Errors
    ///
    /// Returns the [`DeclError`] naming the first rule the declaration breaks;
    /// its variants are the rules.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{DeclError, Kind, ParamDecl, ParamSetBuilder};
    ///
    /// const GAIN: ParamDecl = ParamDecl::new(
    ///     "gain",
    ///     "Gain",
    ///     Kind::continuous(-60.0, 12.0, 0.1, None),
    ///     0.0,
    /// )
    ///     .with_unit("dB");
    ///
    /// let mut builder = ParamSetBuilder::new();
    /// assert!(builder.add(GAIN).is_ok());
    /// // A second "gain" is refused and the builder is unchanged.
    /// assert!(matches!(builder.add(GAIN), Err(DeclError::DuplicateKey("gain"))));
    /// assert_eq!(builder.build().len(), 1);
    /// ```
    pub fn add(&mut self, decl: ParamDecl) -> Result<&mut Self, DeclError> {
        self.check(&decl)?;
        self.keys.insert(decl.key);
        self.host_ids.insert(host_id(decl.key), decl.key);
        self.groups.extend(group_names(decl.group));
        self.bypass |= decl.bypass;
        self.decls.push(decl);
        Ok(self)
    }

    fn check(&self, decl: &ParamDecl) -> Result<(), DeclError> {
        let key = decl.key;
        if key.is_empty() {
            return Err(DeclError::EmptyKey);
        }
        if key.contains('/') {
            return Err(DeclError::KeyHasSlash(key));
        }
        if key.len() > MAX_KEY_LEN {
            return Err(DeclError::KeyTooLong(key));
        }
        if self.keys.contains(key) {
            return Err(DeclError::DuplicateKey(key));
        }
        if self.groups.contains(key) {
            return Err(DeclError::KeyIsGroup(key));
        }
        let id = host_id(key);
        if RESERVED_HOST_IDS.contains(&id) {
            return Err(DeclError::ReservedHostId(key));
        }
        if let Some(&other) = self.host_ids.get(&id) {
            return Err(DeclError::HostIdCollision { key, other });
        }
        for name in group_names(decl.group) {
            if name.is_empty() {
                return Err(DeclError::EmptyGroupName {
                    key,
                    group: decl.group,
                });
            }
            if name == key || self.keys.contains(name) {
                return Err(DeclError::KeyIsGroup(name));
            }
        }
        if let Kind::Choice(labels) = decl.kind {
            check_labels(key, labels)?;
        }
        if decl.bypass && !decl.kind.is_boolean() {
            return Err(DeclError::BypassNotBool(key));
        }
        if decl.bypass && self.bypass {
            return Err(DeclError::SecondBypass(key));
        }
        Ok(())
    }

    /// Freezes the declarations into a set, in declaration order.
    ///
    /// The order is groups depth-first, a group's own parameters before its
    /// subgroups, each in the order declared. Every store starts at its
    /// parameter's snapped default.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{Kind, ParamDecl, ParamSetBuilder};
    ///
    /// let mut builder = ParamSetBuilder::new();
    /// builder.add(ParamDecl::new("late", "Late", Kind::Bool, 0.0).with_group("Fx"))?;
    /// builder.add(ParamDecl::new("early", "Early", Kind::Bool, 0.0))?;
    ///
    /// // Declaration order lists the top level first, then each group.
    /// let set = builder.build();
    /// let keys: Vec<_> = set.ids().map(|id| set.decl(id).key).collect();
    /// assert_eq!(keys, ["early", "late"]);
    /// # Ok::<(), seam_params::DeclError>(())
    /// ```
    pub fn build(self) -> Arc<ParamSet> {
        let order = declaration_order(&self.decls);
        let decls: Box<[ParamDecl]> = order
            .iter()
            .filter_map(|&i| self.decls.get(i).copied())
            .collect();
        let by_key = decls
            .iter()
            .enumerate()
            .map(|(index, decl)| (decl.key, ParamId::from_index(index)))
            .collect();
        let bypass = decls
            .iter()
            .position(|decl| decl.bypass)
            .map(ParamId::from_index);
        let values = decls
            .iter()
            .map(|decl| ValueCell::new(Packed::new(decl.default_value(), Origin::Restore)))
            .collect();
        let changes = decls.iter().map(|_| AtomicU64::new(0)).collect();
        let host_ids: Box<[u32]> = decls.iter().map(|decl| host_id(decl.key)).collect();
        let by_host_id = host_ids
            .iter()
            .enumerate()
            .map(|(index, &id)| (id, ParamId::from_index(index)))
            .collect();
        let host = HostQueue::new(decls.len());
        Arc::new(ParamSet {
            decls,
            values,
            changes,
            by_key,
            host_ids,
            by_host_id,
            bypass,
            host,
            ui_taken: AtomicBool::new(false),
            clock: RestoreClock::new(),
        })
    }
}

/// The entry of `cells` that `id` names. An id from another set is the
/// caller's bug that [`ParamSet`] documents.
#[expect(
    clippy::indexing_slicing,
    reason = "an id from the owning set is in range; a foreign one is the caller bug ParamSet documents"
)]
pub(crate) fn cell<T>(cells: &[T], id: ParamId) -> &T {
    &cells[id.index()]
}

/// Checks a choice's labels against the [`DeclError`] rules for them.
fn check_labels(key: &'static str, labels: &'static [&'static str]) -> Result<(), DeclError> {
    if labels.len() < 2 {
        return Err(DeclError::TooFewChoices(key));
    }
    for (index, &label) in labels.iter().enumerate() {
        if label.trim().is_empty() {
            return Err(DeclError::EmptyChoiceLabel(key));
        }
        if label.trim() != label {
            return Err(DeclError::PaddedChoiceLabel { key, label });
        }
        let mut earlier = labels.iter().take(index);
        if earlier.any(|other| other.eq_ignore_ascii_case(label)) {
            return Err(DeclError::DuplicateChoiceLabel { key, label });
        }
    }
    Ok(())
}

/// The names along a group path; none for the top level.
fn group_names(group: &'static str) -> impl Iterator<Item = &'static str> {
    group.split('/').filter(move |_| !group.is_empty())
}

/// Each group along a path, named by the path up to it: `Mix`, then
/// `Mix/Sides`. None for the top level.
fn group_paths(group: &'static str) -> impl Iterator<Item = &'static str> {
    let ends = group.match_indices('/').map(|(slash, _)| slash);
    ends.chain([group.len()])
        .filter(move |_| !group.is_empty())
        .filter_map(move |end| group.get(..end))
}

/// The declaration indices in declaration order.
#[expect(
    clippy::indexing_slicing,
    reason = "runs once, on the main thread, when a set is built; the indices are ones it just made"
)]
fn declaration_order(decls: &[ParamDecl]) -> Vec<usize> {
    #[derive(Default)]
    struct Group {
        params: Vec<usize>,
        children: Vec<usize>,
    }
    let mut groups = vec![Group::default()];
    let mut by_path: HashMap<&str, usize> = HashMap::new();
    for (index, decl) in decls.iter().enumerate() {
        let mut node = 0;
        for path in group_paths(decl.group) {
            node = *by_path.entry(path).or_insert_with(|| {
                let child = groups.len();
                groups.push(Group::default());
                groups[node].children.push(child);
                child
            });
        }
        groups[node].params.push(index);
    }
    let mut order = Vec::with_capacity(decls.len());
    let mut stack = vec![0];
    while let Some(node) = stack.pop() {
        order.extend(&groups[node].params);
        // Pushed in reverse so the first child is visited first.
        stack.extend(groups[node].children.iter().rev());
    }
    order
}

/// The frozen parameters and their stores: one value cell in real units and
/// one change counter each, which every thread reads without locks.
///
/// A [`ParamId`] names a parameter of the set that minted it. Passing one from
/// another set is a bug: a method panics when the index is out of range and
/// otherwise names another parameter.
///
/// # Examples
///
/// ```
/// use seam_params::{Kind, ParamDecl, ParamSet};
///
/// let set = ParamSet::from_decls([
///     ParamDecl::new("bypass", "Bypass", Kind::Bool, 1.0).as_bypass(),
/// ])?;
///
/// let id = set.id("bypass").expect("declared above");
/// assert_eq!(set.value(id), 1.0);
/// assert_eq!(set.bypass(), Some(id));
/// # Ok::<(), seam_params::DeclError>(())
/// ```
#[derive(Debug)]
pub struct ParamSet {
    decls: Box<[ParamDecl]>,
    values: Box<[ValueCell]>,
    /// Moved by every write after its store; see
    /// [`store_then_publish`](Self::store_then_publish).
    changes: Box<[AtomicU64]>,
    by_key: HashMap<&'static str, ParamId>,
    host_ids: Box<[u32]>,
    by_host_id: HashMap<u32, ParamId>,
    bypass: Option<ParamId>,
    host: HostQueue,
    ui_taken: AtomicBool,
    clock: Arc<RestoreClock>,
}

impl ParamSet {
    /// Declares a whole plugin's parameters at once: checks each, in order, as
    /// [`ParamSetBuilder::add`] does, and freezes them.
    ///
    /// This is the usual way to make a set; [`ParamSetBuilder`] adds
    /// declarations from several places.
    ///
    /// # Errors
    ///
    /// Returns the [`DeclError`] of the first declaration refused.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{DeclError, Kind, ParamDecl, ParamSet};
    ///
    /// const PARAMS: &[ParamDecl] = &[
    ///     ParamDecl::new("on", "On", Kind::Bool, 1.0),
    ///     ParamDecl::new("gain", "Gain", Kind::continuous(-24.0, 24.0, 0.1, None), 0.0)
    ///         .with_unit("dB"),
    /// ];
    ///
    /// let set = ParamSet::from_decls(PARAMS.iter().copied())?;
    /// assert_eq!(set.len(), 2);
    ///
    /// let twice = [PARAMS[0], PARAMS[0]];
    /// assert_eq!(
    ///     ParamSet::from_decls(twice).unwrap_err(),
    ///     DeclError::DuplicateKey("on")
    /// );
    /// # Ok::<(), DeclError>(())
    /// ```
    pub fn from_decls(
        decls: impl IntoIterator<Item = ParamDecl>,
    ) -> Result<Arc<ParamSet>, DeclError> {
        let mut builder = ParamSetBuilder::new();
        for decl in decls {
            builder.add(decl)?;
        }
        Ok(builder.build())
    }

    /// Returns how many parameters there are.
    pub fn len(&self) -> usize {
        self.decls.len()
    }

    /// Returns whether there are none.
    pub fn is_empty(&self) -> bool {
        self.decls.is_empty()
    }

    /// Returns every parameter, in declaration order.
    pub fn ids(&self) -> impl ExactSizeIterator<Item = ParamId> + use<> {
        (0..self.len()).map(ParamId::from_index)
    }

    /// Returns the parameter declared under this key, or `None` when there is
    /// none. Look an id up once and keep it.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{Kind, ParamDecl, ParamSet};
    ///
    /// let set = ParamSet::from_decls([ParamDecl::new("gain", "Gain", Kind::Bool, 0.0)])?;
    ///
    /// assert!(set.id("gain").is_some());
    /// // Only the key finds it: the display name does not.
    /// assert!(set.id("Gain").is_none());
    /// # Ok::<(), seam_params::DeclError>(())
    /// ```
    pub fn id(&self, key: &str) -> Option<ParamId> {
        self.by_key.get(key).copied()
    }

    /// Returns the id hosts know the parameter by, the
    /// [`host_id`](fn@crate::host_id) of its key.
    /// [`by_host_id`](Self::by_host_id) reads it back.
    pub fn host_id(&self, id: ParamId) -> u32 {
        *cell(&self.host_ids, id)
    }

    /// Returns the parameter a host id names, or `None` for an id no
    /// parameter has. It only reads, so the audio thread may call it.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{Kind, ParamDecl, ParamSet, host_id};
    ///
    /// let set = ParamSet::from_decls([ParamDecl::new("gain", "Gain", Kind::Bool, 0.0)])?;
    /// let gain = set.id("gain").ok_or("declared")?;
    ///
    /// // The host id is a hash of the key, so it does not depend on the order.
    /// assert_eq!(set.host_id(gain), host_id("gain"));
    /// assert_eq!(set.by_host_id(host_id("gain")), Some(gain));
    /// assert_eq!(set.by_host_id(host_id("other")), None);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]
    pub fn by_host_id(&self, host_id: u32) -> Option<ParamId> {
        self.by_host_id.get(&host_id).copied()
    }

    /// Returns the parameter at a host's index, its declaration-order
    /// position, or `None` past the last.
    pub fn id_at(&self, index: usize) -> Option<ParamId> {
        (index < self.len()).then(|| ParamId::from_index(index))
    }

    /// Returns a parameter's declaration.
    pub fn decl(&self, id: ParamId) -> &ParamDecl {
        cell(&self.decls, id)
    }

    /// Returns the bypass parameter, if one is declared.
    pub fn bypass(&self) -> Option<ParamId> {
        self.bypass
    }

    /// Returns the current value, in real units. The
    /// [crate overview](crate#examples) shows it read on the audio thread.
    ///
    /// A reader that keeps a count, as a [`Watch`](crate::Watch) does, reads
    /// both through [`read`](Self::read).
    #[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]
    pub fn value(&self, id: ParamId) -> f32 {
        cell(&self.values, id).load(Ordering::Relaxed).value()
    }

    /// Returns the change count, then the value and the origin of the write
    /// that stored it, from one load, as a [`Reading`].
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{Kind, Origin, ParamDecl, ParamSet};
    ///
    /// let set = ParamSet::from_decls([
    ///     ParamDecl::new("gain", "Gain", Kind::continuous(-60.0, 12.0, 0.0, None), 0.0),
    /// ])?;
    /// let gain = set.id("gain").ok_or("declared")?;
    ///
    /// // Before any write: the default, which counts as a restore's.
    /// let first = set.read(gain);
    /// assert_eq!((first.changes, first.value, first.origin), (0, 0.0, Origin::Restore));
    ///
    /// set.audio().host_write(gain, 1.0);
    /// let second = set.read(gain);
    /// assert_eq!((second.changes, second.value, second.origin), (1, 12.0, Origin::Host));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]
    pub fn read(&self, id: ParamId) -> Reading<f32> {
        // The count first, as `store_then_publish` explains; then the value
        // and its origin from one word.
        let changes = self.changes(id);
        let word = cell(&self.values, id).load(Ordering::Relaxed);
        Reading {
            changes,
            value: word.value(),
            origin: word.origin(),
        }
    }

    /// Returns the current value as a host's 0..1, computed from the store
    /// each time, never cached.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{Kind, ParamDecl, ParamSet};
    ///
    /// let set = ParamSet::from_decls([
    ///     ParamDecl::new("gain", "Gain", Kind::continuous(-60.0, 12.0, 0.0, None), -24.0),
    /// ])?;
    /// let gain = set.id("gain").ok_or("declared")?;
    ///
    /// assert_eq!(set.value(gain), -24.0); // real units
    /// assert_eq!(set.normalized(gain), 0.5); // the host's 0..1
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]
    pub fn normalized(&self, id: ParamId) -> f64 {
        self.decl(id).kind.normalize(self.value(id))
    }

    /// Returns how many writes have reached this parameter, a repeated value
    /// included. Compare it with the count you last saw; the
    /// [crate overview](crate#examples) shows the pattern.
    #[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]
    pub fn changes(&self, id: ParamId) -> u64 {
        // Acquire pairs with the release in `store_then_publish`.
        cell(&self.changes, id).load(Ordering::Acquire)
    }

    /// Returns whether a sync has asked the host to re-read every value
    /// since the last call, clearing the request.
    ///
    /// The exporter calls it where VST3's restart and CLAP's rescan may be
    /// made.
    ///
    /// Thread: main.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{Kind, ParamDecl, ParamSet};
    ///
    /// let set = ParamSet::from_decls([
    ///     ParamDecl::new("gain", "Gain", Kind::continuous(-60.0, 12.0, 0.1, None), 0.0),
    /// ])?;
    /// let gain = set.id("gain").ok_or("declared")?;
    ///
    /// assert!(!set.take_rescan()); // nothing asked yet
    /// set.program().sync(gain, 3.0); // a preset recall asks the host to re-read
    /// assert!(set.take_rescan());
    /// assert!(!set.take_rescan()); // taking it cleared the request
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn take_rescan(&self) -> bool {
        self.host.take_rescan()
    }

    /// Returns the handle the audio thread holds, which can make only
    /// host-shaped writes.
    pub fn audio(self: &Arc<Self>) -> AudioParams {
        AudioParams {
            set: Arc::clone(self),
        }
    }

    /// Returns the handle for writes from any thread but audio: syncs and
    /// restores.
    pub fn program(self: &Arc<Self>) -> ProgramParams {
        ProgramParams::new(Arc::clone(self))
    }

    /// Takes the UI thread's handle: user writes, program edits and undo
    /// replays, with the gesture count they share.
    ///
    /// There is one per set, since two would count gestures apart; `None`
    /// while one, or a clone of it, is alive. Make it once and clone it.
    ///
    /// Thread: UI.
    ///
    /// # Panics
    ///
    /// In a build with `debug_assertions`, while the thread holds an
    /// [`audio_thread`] mark: making the handle allocates.
    ///
    /// # Examples
    ///
    /// ```
    /// # let set = seam_params::ParamSet::from_decls([])?;
    /// let ui = set.take_ui().expect("the first UI handle");
    /// assert!(set.take_ui().is_none()); // one per set, clones included
    ///
    /// let also = ui.clone();
    /// drop(ui);
    /// assert!(set.take_ui().is_none()); // a clone is alive
    /// drop(also);
    /// assert!(set.take_ui().is_some());
    /// # Ok::<(), seam_params::DeclError>(())
    /// ```
    pub fn take_ui(self: &Arc<Self>) -> Option<UiParams> {
        audio_thread::assert_not_here("ParamSet::take_ui");
        let taken = self.ui_taken.swap(true, Ordering::AcqRel);
        (!taken).then(|| UiParams::new(Arc::clone(self), self.clock.reader()))
    }

    /// The plugin's one clock, made with the set.
    #[doc(hidden)]
    pub fn clock(&self) -> &Arc<RestoreClock> {
        &self.clock
    }

    /// Takes the exporter's drain of what edits tell the host. There is one
    /// per set; `None` while one is alive. [`HostEventDrain`] shows how it is
    /// used.
    ///
    /// # Panics
    ///
    /// In a build with `debug_assertions`, while the thread holds an
    /// [`audio_thread`] mark: making the drain allocates.
    pub fn take_host_event_drain(self: &Arc<Self>) -> Option<HostEventDrain> {
        audio_thread::assert_not_here("ParamSet::take_host_event_drain");
        HostEventDrain::new(self)
    }

    pub(crate) fn host(&self) -> &HostQueue {
        &self.host
    }

    pub(crate) fn release_ui(&self) {
        self.ui_taken.store(false, Ordering::Release);
    }

    /// [`store_then_publish`](Self::store_then_publish) with nothing between.
    pub(crate) fn store(&self, id: ParamId, value: f32, origin: Origin) {
        self.store_then_publish(id, value, origin, || {});
    }

    /// Every write, in its one order: the value and its origin in one store,
    /// then `between`, then the publish. The value is legal.
    ///
    /// The publish is the count's release, which pairs with the acquire in
    /// [`changes`](Self::changes): a reader that loads the count and then the
    /// value never sees a value older than the count. The store comes before
    /// `between` because `between` may tell the host, which can read the
    /// value back at once; nothing depends on `between` coming before the
    /// publish. A [`value`](Self::value) call may see the write sooner.
    pub(crate) fn store_then_publish(
        &self,
        id: ParamId,
        value: f32,
        origin: Origin,
        between: impl FnOnce(),
    ) {
        // Relaxed: a release after this in program order makes the word
        // visible to a reader that acquires it. That is the count's below for
        // a watch, and for the host's drain the release `between` makes when
        // it tells the host.
        cell(&self.values, id).store(Packed::new(value, origin), Ordering::Relaxed);
        between();
        cell(&self.changes, id).fetch_add(1, Ordering::Release);
    }
}

/// The audio thread's handle to the parameters.
///
/// Its writes are host-shaped, as [`Origin`] says, and constant-time: none
/// blocks or allocates. Each stores and publishes whether or not the value
/// changed; watches deduplicate. Drop it off the audio thread: if it holds the
/// set's last reference, dropping it frees the set.
///
/// # Examples
///
/// ```
/// use seam_params::{Kind, ParamDecl, ParamSet};
///
/// let set = ParamSet::from_decls([
///     ParamDecl::new("mix", "Mix", Kind::continuous(0.0, 100.0, 0.0, None), 100.0)
///         .with_unit("%"),
/// ])?;
/// let mix = set.id("mix").expect("declared above");
///
/// // The host automates the parameter to the middle of its range.
/// let audio = set.audio();
/// audio.host_write(mix, 0.5);
/// assert_eq!(audio.value(mix), 50.0);
/// # Ok::<(), seam_params::DeclError>(())
/// ```
///
/// # Panics
///
/// In a build with `debug_assertions`, dropping it panics while the thread
/// holds an [`audio_thread`] mark.
#[derive(Debug, Clone)]
pub struct AudioParams {
    set: Arc<ParamSet>,
}

impl Drop for AudioParams {
    fn drop(&mut self) {
        audio_thread::assert_not_here("dropping AudioParams");
    }
}

impl AudioParams {
    /// Returns the set these writes reach.
    pub fn set(&self) -> &ParamSet {
        &self.set
    }

    /// Returns the current value, in real units, as [`ParamSet::value`] does.
    #[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]
    pub fn value(&self, id: ParamId) -> f32 {
        self.set.value(id)
    }

    /// Returns the change count, as [`ParamSet::changes`] does. The
    /// [crate overview](crate#examples) shows a processor comparing it.
    #[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]
    pub fn changes(&self, id: ParamId) -> u64 {
        self.set.changes(id)
    }

    /// Returns the change count, the value and its origin, as
    /// [`ParamSet::read`] does.
    #[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]
    pub fn read(&self, id: ParamId) -> Reading<f32> {
        self.set.read(id)
    }

    /// Writes a host's automation value, as its 0..1, storing what
    /// [`ParamDecl::host_value`] makes of it.
    ///
    /// # Examples
    ///
    /// ```
    /// # use seam_params as sp;
    /// # let set = sp::ParamSet::from_decls([
    /// #     sp::ParamDecl::new("gain", "Gain", sp::Kind::continuous(-60.0, 12.0, 0.1, None), 0.0),
    /// # ])?;
    /// # let gain = set.id("gain").expect("declared");
    ///
    /// let audio = set.audio();
    ///
    /// audio.host_write(gain, 0.5); // the middle of -60..12, snapped
    /// assert_eq!(audio.value(gain), -24.0);
    /// audio.host_write(gain, f64::NAN); // not finite: the range start
    /// assert_eq!(audio.value(gain), -60.0);
    /// # Ok::<(), seam_params::DeclError>(())
    /// ```
    #[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]
    pub fn host_write(&self, id: ParamId, normalized: f64) {
        let value = self.set.decl(id).host_value(normalized);
        self.set.store(id, value, Origin::Host);
    }

    /// Writes a learned controller's value, as its 0..1 after the mapping's
    /// curve, storing what [`ParamDecl::learned_value`] makes of it: NaN
    /// writes nothing.
    ///
    /// # Examples
    ///
    /// ```
    /// # use seam_params as sp;
    /// # let set = sp::ParamSet::from_decls([
    /// #     sp::ParamDecl::new("gain", "Gain", sp::Kind::continuous(-60.0, 12.0, 0.1, None), 0.0),
    /// # ])?;
    /// # let gain = set.id("gain").expect("declared");
    ///
    /// let audio = set.audio();
    ///
    /// audio.learned_write(gain, 1.0); // a controller at its top
    /// assert_eq!(audio.value(gain), 12.0);
    /// audio.learned_write(gain, f64::NAN); // writes nothing
    /// assert_eq!(audio.value(gain), 12.0);
    /// # Ok::<(), seam_params::DeclError>(())
    /// ```
    #[cfg_attr(seam_rtsan, sanitize(realtime = "nonblocking"))]
    pub fn learned_write(&self, id: ParamId, normalized: f64) {
        if let Some(value) = self.set.decl(id).learned_value(normalized) {
            self.set.store(id, value, Origin::Learn);
        }
    }
}

#[cfg(test)]
mod tests;
mod value_cell;
