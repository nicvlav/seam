//! The saved block: the plugin's parameters and its session as one UTF-8
//! JSON object.

use std::borrow::Cow;
use std::error::Error;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use seam_params::{Kind, ParamDecl, ParamSet};
use seam_session::{Restored, Session};
use serde_json::{Map, Value as Json};

mod run_time;

pub(crate) use run_time::{Assigned, RunTimeSaved};
use run_time::{KEY as RUN_TIME, Loaded};

/// The version of the block's frame this build of Seam writes: the shape Seam
/// gives `parameters`, `extra` and the keys a load keeps. A plugin's own
/// schema has a version of its own (see [`SavedState::with_migration`]).
pub const STATE_VERSION: u64 = 1;

/// The key of the frame's version, [`STATE_VERSION`].
const SEAM_VERSION: &str = "seamStateVersion";
/// The key of the plugin's own schema version.
const PLUGIN_VERSION: &str = "pluginStateVersion";

/// A plugin's upgrade of an older block, in place, or why it cannot.
type Migrate =
    dyn Fn(u64, &mut Map<String, Json>) -> Result<(), Box<dyn Error + Send + Sync>> + Send + Sync;

/// The longest saved block a load reads, 64 MiB: a longer one is refused, so a
/// host's endless or corrupt stream cannot exhaust memory.
///
/// A save never writes a longer one. A participant's capture that would grow
/// the block past it is refused, the section saved before stays saved, and
/// [`Registration::refused`](seam_session::Registration::refused) says so,
/// whether the participant registered before the saved state was made or
/// after. A load refuses a block that would save back past it, whole: a block
/// from elsewhere can lengthen as it is written again, since `1e15` saves as
/// `1000000000000000.0` and a parameter it lacks is written in full. The room
/// a block leaves for its sections counts every declared parameter at its
/// longest value and the unknown keys the last load kept.
pub const MAX_STATE_BYTES: usize = seam_session::MAX_STATE_BYTES;

/// Why a saved block was not loaded. A refused load changes nothing.
///
/// # Examples
///
/// ```
/// use seam_host::{LoadError, SavedState};
/// use seam_params::{Kind, ParamDecl, ParamSet};
/// use seam_session::Session;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let gain = ParamDecl::new("gain", "Gain", Kind::continuous(0.0, 1.0, 0.0, None), 0.0);
/// let set = ParamSet::from_decls([gain])?;
/// let state = SavedState::new(&set, &Session::new(&set));
/// assert!(matches!(state.load(b"[]"), Err(LoadError::NotAnObject)));
/// assert!(matches!(state.load(b"{\"parameters\":{}}"), Err(LoadError::BadVersion)));
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub enum LoadError {
    /// The block is longer than [`MAX_STATE_BYTES`], or would save back
    /// longer.
    TooLarge,
    /// The block is not UTF-8 JSON, or has bytes after the object other
    /// than JSON whitespace and trailing NULs.
    NotJson,
    /// The block is JSON, but not an object.
    NotAnObject,
    /// `seamStateVersion` is missing, not a whole number, or below 1.
    BadVersion,
    /// `pluginStateVersion` is present but not a whole number from 0.
    BadPluginVersion,
    /// The plugin's migration could not upgrade the block's older plugin
    /// version, for the reason it holds, which is also its
    /// [`source`](Error::source).
    MigrationFailed(Box<dyn Error + Send + Sync>),
    /// `parameters` is missing or not an object.
    BadParameters,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LoadError::TooLarge => {
                "the saved block is longer than the limit, or would save back longer"
            }
            LoadError::NotJson => "the saved block is not one JSON value",
            LoadError::NotAnObject => "the saved block is not a JSON object",
            LoadError::BadVersion => {
                "the saved block's seamStateVersion is missing or not a whole number from 1"
            }
            LoadError::BadPluginVersion => {
                "the saved block's pluginStateVersion is not a whole number from 0"
            }
            LoadError::MigrationFailed(_) => "the plugin could not upgrade the saved block",
            LoadError::BadParameters => "the saved block's parameters are missing or not an object",
        })
    }
}

impl Error for LoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            LoadError::MigrationFailed(reason) => Some(&**reason),
            _ => None,
        }
    }
}

/// A plugin instance's saved state: what the host saves and restores.
///
/// [`save`](Self::save) and [`load`](Self::load) never interleave: a save
/// racing a load writes one block or the other, never a mix. Neither touches
/// the plugin: a save reads only the parameter stores and the session's saved
/// sections.
///
/// # Examples
///
/// A save, then a load into a second instance of the same plugin:
///
/// ```
/// use std::sync::Arc;
///
/// use seam_host::SavedState;
/// use seam_params::{Kind, ParamDecl, ParamSet};
/// use seam_session::Session;
///
/// let gain = ParamDecl::new("gain", "Gain", Kind::continuous(-24.0, 24.0, 0.0, None), 0.0)
///     .with_unit("dB");
/// let make = || {
///     let set = ParamSet::from_decls([gain]).expect("a valid declaration");
///     let session = Session::new(&set);
///     SavedState::new(&set, &session)
/// };
///
/// let first = make();
/// let id = first.set().id("gain").expect("declared");
/// first.set().program().load().restore(id, 6.0);
/// let bytes = first.save();
///
/// let second = make();
/// assert!(second.load(&bytes).is_ok());
/// assert_eq!(second.set().value(id), 6.0);
/// ```
pub struct SavedState {
    set: Arc<ParamSet>,
    session: Session,
    /// The plugin's own schema, when it has a migration; at 0 without.
    migration: Option<Migration>,
    /// The most bytes a block of this set takes besides the session's `extra`
    /// object and the unknown keys: its frame at the longest versions, and
    /// every declared parameter at its longest value.
    declared_bytes: usize,
    /// Held across a whole save or load, so the two never interleave.
    last: Mutex<LastLoad>,
    /// The parameters made at run time a save writes, and the entries a
    /// load holds for them.
    run_time: RunTimeSaved,
}

/// The plugin's schema version and its upgrade of a block of an older one.
struct Migration {
    version: u64,
    migrate: Box<Migrate>,
}

/// What the last load left for the saves after it.
#[derive(Debug, Default)]
struct LastLoad {
    /// The versions of the block it took, both 0 before any.
    versions: Versions,
    unknown: Unknown,
}

/// A block's two versions, each kept on its own: Seam's frame and the
/// plugin's schema.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Versions {
    /// `seamStateVersion`, from 1.
    seam: u64,
    /// `pluginStateVersion`, 0 when absent.
    plugin: u64,
}

impl Versions {
    /// Each version the higher of the two, so a block a newer build wrote
    /// keeps each number it carries.
    fn newest(self, other: Versions) -> Versions {
        Versions {
            seam: self.seam.max(other.seam),
            plugin: self.plugin.max(other.plugin),
        }
    }
}

/// The most bytes a declared parameter's value takes in the block.
///
/// A double from an `f32` has at most 17 significant digits, a sign, and a
/// point with four leading zeros or an exponent: 24 in all. A bool and a
/// choice's index take fewer.
const VALUE_BYTES: usize = 24;

/// What a block held that this build does not read, saved back so a newer
/// build's content survives the version a save keeps.
#[derive(Debug, Default)]
struct Unknown {
    /// Parameter keys this build does not declare.
    parameters: Map<String, Json>,
    /// Top-level keys other than the version, `parameters` and `extra`.
    root: Map<String, Json>,
}

impl Unknown {
    /// The bytes these keys take in a block. A parameter writes `"key":value`
    /// and a `,` after it, a top-level key a `,` before it, so each takes its
    /// key, its value and two more.
    fn bytes(&self) -> usize {
        let entry = |(key, value): (&String, &Json)| key_bytes(key) + json_bytes(value) + 2;
        self.parameters.iter().chain(&self.root).map(entry).sum()
    }
}

/// Counts the bytes handed to it, so a JSON text's length is known without
/// building the text.
struct Count(usize);

impl std::io::Write for Count {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The bytes of `value`'s JSON text, as `to_string` writes it.
fn json_bytes(value: &Json) -> usize {
    let mut count = Count(0);
    serde_json::to_writer(&mut count, value).expect("a JSON value always encodes");
    count.0
}

/// The bytes of `key` written as a JSON string, its quotes and escapes
/// included.
fn key_bytes(key: &str) -> usize {
    let mut count = Count(0);
    serde_json::to_writer(&mut count, key).expect("a string always encodes");
    count.0
}

/// A parameter's value as a block writes it: a bool `true` or `false`, a
/// choice its index as an integer, and anything else a double that holds
/// the stored `f32` exactly.
fn value_json(decl: &ParamDecl, value: f32) -> Json {
    match decl.kind {
        Kind::Bool => Json::Bool(value >= 0.5),
        Kind::Choice(_) => Json::from(value as u64),
        Kind::Continuous(_) => Json::from(f64::from(value)),
    }
}

/// A block's value as a parameter's write takes it: a bool as 1 or 0, a
/// number as the nearest `f32`, infinite outside its range. `None` for
/// anything else.
fn value_number(value: &Json) -> Option<f32> {
    match value {
        Json::Bool(on) => Some(if *on { 1.0 } else { 0.0 }),
        Json::Number(number) => number.as_f64().map(|value| value as f32),
        _ => None,
    }
}

/// One `"key":value` entry of a JSON object.
fn entry(key: &str, value: &Json) -> String {
    format!("{}:{value}", Json::from(key))
}

/// `map`'s entries in key order, so the same state always saves the same
/// bytes, whatever order the map keeps: any crate in a build may turn on
/// `serde_json`'s `preserve_order`, which keeps insertion order.
fn sorted(map: &Map<String, Json>) -> Vec<(&String, &Json)> {
    let mut entries: Vec<_> = map.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    entries
}

impl std::fmt::Debug for SavedState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SavedState")
            .field("plugin_version", &self.versions().plugin)
            .field("migrates", &self.migration.is_some())
            .finish_non_exhaustive()
    }
}

impl SavedState {
    /// Creates the saved state of `set` and `session`.
    ///
    /// # Panics
    ///
    /// When `session` was made from another set: a load would not mark its
    /// restores for the session's undo histories, and an undo could overwrite
    /// a loaded value. Make the [`Session`] from the same set.
    pub fn new(set: &Arc<ParamSet>, session: &Session) -> Self {
        assert!(
            session.uses(set),
            "the session was made from another ParamSet; make the Session from the same ParamSet"
        );
        let longest = u64::MAX;
        let frame = format!(
            "{{\"{SEAM_VERSION}\":{longest},\"{PLUGIN_VERSION}\":{longest},\
             \"parameters\":{{}},\"extra\":}}"
        );
        // Each parameter writes `"key":value` and a `,` or the `}` after it.
        let parameters: usize = set
            .ids()
            .map(|id| key_bytes(set.decl(id).key) + VALUE_BYTES + 2)
            .sum();
        let state = SavedState {
            set: Arc::clone(set),
            session: session.clone(),
            migration: None,
            declared_bytes: frame.len() + parameters,
            last: Mutex::default(),
            run_time: RunTimeSaved::default(),
        };
        state
            .session
            .set_budget(state.room_for_extra(&Unknown::default()));
        state
    }

    /// The room a block with `unknown` in it leaves for the session's `extra`
    /// object, so a save stays within [`MAX_STATE_BYTES`]. At 0 nothing fits,
    /// since the session counts an `extra` object's `{` even when it is empty.
    fn room_for_extra(&self, unknown: &Unknown) -> usize {
        MAX_STATE_BYTES.saturating_sub(self.declared_bytes + unknown.bytes())
    }

    /// Returns this saved state writing blocks of the plugin's own schema
    /// `version`, and upgrading a block of any older one with `migrate`
    /// before it loads.
    ///
    /// The plugin's version is its own, apart from Seam's frame
    /// ([`STATE_VERSION`]). A block carries it as `pluginStateVersion`, and a
    /// plugin with no migration is at 0, as is a block without the key. A
    /// plugin raises it when a block of the old shape would load wrong: a
    /// parameter renamed or rescaled, a section restructured. `migrate` gets
    /// the block's plugin version and the block, as a JSON object with its
    /// `parameters` and `extra`. `migrate` rewrites the block in place into
    /// the current shape, or returns why it cannot, and the load then fails
    /// with [`LoadError::MigrationFailed`] and changes nothing. A block of
    /// this version or a newer one loads as it is, so at 0 `migrate` never
    /// runs. The version keys `migrate` writes are ignored: a save writes
    /// the higher of the block's versions and this build's. `migrate` runs
    /// inside the load, on whichever thread the host loads from, so it
    /// touches only the block.
    ///
    /// A migration adds what the block lacks and never overwrites a key the
    /// block already holds. An older build keeps the keys it does not know
    /// and saves them back. So a key the block holds may be newer than
    /// anything a migration would derive from the old ones.
    ///
    /// A rescaled parameter takes a new key, as a renamed one does. An
    /// older build saves a block it loaded with that block's newer version.
    /// A value it wrote back in the old scale under the same key would then
    /// load here as it is, never migrated.
    ///
    /// # Examples
    ///
    /// Version 1 of a plugin's state renamed its `level` parameter `gain`:
    ///
    /// ```
    /// use seam_host::{LoadError, SavedState};
    /// use seam_params::{Kind, ParamDecl, ParamSet};
    /// use seam_session::Session;
    /// use serde_json::Value;
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let set = ParamSet::from_decls([
    ///     ParamDecl::new("gain", "Gain", Kind::continuous(-24.0, 24.0, 0.0, None), 0.0)
    ///         .with_unit("dB"),
    /// ])?;
    /// let state = SavedState::new(&set, &Session::new(&set)).with_migration(1, |from, block| {
    ///     if from < 1 {
    ///         let parameters = block
    ///             .get_mut("parameters")
    ///             .and_then(Value::as_object_mut)
    ///             .ok_or("the parameters are not an object")?;
    ///         if let Some(level) = parameters.remove("level") {
    ///             parameters.entry("gain").or_insert(level);
    ///         }
    ///     }
    ///     Ok(())
    /// });
    ///
    /// // A block from before the rename has no plugin version: it is at 0.
    /// let gain = set.id("gain").ok_or("declared")?;
    /// state.load(br#"{"seamStateVersion": 1, "parameters": {"level": 6}}"#)?;
    /// assert_eq!(set.value(gain), 6.0);
    /// assert!(state.save().starts_with(br#"{"seamStateVersion":1,"pluginStateVersion":1,"#));
    ///
    /// // A `gain` the block holds already, kept by an older build from a newer one, stands.
    /// let both = br#"{"seamStateVersion": 1, "parameters": {"level": 6, "gain": 3}}"#;
    /// state.load(both)?;
    /// assert_eq!(set.value(gain), 3.0);
    ///
    /// // The reason a migration gives reaches the host's load.
    /// let Err(LoadError::MigrationFailed(reason)) =
    ///     state.load(br#"{"seamStateVersion": 1, "parameters": []}"#)
    /// else {
    ///     panic!("not migrated");
    /// };
    /// assert_eq!(reason.to_string(), "the parameters are not an object");
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn with_migration(
        mut self,
        version: u64,
        migrate: impl Fn(u64, &mut Map<String, Json>) -> Result<(), Box<dyn Error + Send + Sync>>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.migration = Some(Migration {
            version,
            migrate: Box::new(migrate),
        });
        self
    }

    /// The versions this build writes: Seam's frame, and the plugin's schema.
    fn versions(&self) -> Versions {
        Versions {
            seam: STATE_VERSION,
            plugin: self
                .migration
                .as_ref()
                .map_or(0, |migration| migration.version),
        }
    }

    /// Returns the parameter set it saves and loads.
    ///
    /// An exporter takes the plugin's set from here, so the host's
    /// parameters, the saved block and the clock are one set.
    pub fn set(&self) -> &Arc<ParamSet> {
        &self.set
    }

    /// The parameters made at run time a save writes, and the entries a
    /// load holds for them.
    pub(crate) fn run_time(&self) -> &RunTimeSaved {
        &self.run_time
    }

    /// Returns the block as UTF-8 JSON with no byte-order mark:
    /// `{"seamStateVersion": 1, "pluginStateVersion": 0, "parameters": {...},
    /// "runTime": {...}, "extra": {...}}`.
    ///
    /// Both versions are always written, the plugin's at 0 too, so every
    /// block says what it holds. Each is the higher of this build's and the
    /// last loaded block's, on its own, so a newer build that loads the block
    /// does not migrate it as an older one, and finds what it wrote. The
    /// parameters come in declaration order, then the unknown keys the last
    /// load kept. A bool writes `true` or `false`, a choice its index as an
    /// integer, and everything else a double that holds the stored `f32`
    /// exactly.
    ///
    /// `runTime` holds the parameters the plugin made at run time and still
    /// holds ([`Parts::declare`](crate::Parts::declare)), sorted by path:
    /// each `{"value": ..., "learned": {...}}`, its value read now and
    /// written as `parameters` writes one, and its learned mapping as the
    /// last tick captured it. While a load's entries wait for the pass that
    /// applies it, it holds them as loaded, and the live value only of a
    /// parameter made since that load, with the load's mapping for its path.
    /// It takes the room the rest of the block leaves: an entry with no room
    /// is left out, and the instance records a fault. With no entry, the key
    /// is left out.
    ///
    /// The session's `extra` is left out when there is nothing in it. Last
    /// come the top-level keys the last load held that this build does not
    /// read, in key order.
    ///
    /// Thread: any but audio.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_host::SavedState;
    /// use seam_params::{Kind, ParamDecl, ParamSet};
    /// use seam_session::Session;
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let bypass = ParamDecl::new("bypass", "Bypass", Kind::Bool, 1.0).as_bypass();
    /// let set = ParamSet::from_decls([bypass])?;
    /// let state = SavedState::new(&set, &Session::new(&set));
    ///
    /// let block = String::from_utf8(state.save())?;
    /// assert_eq!(
    ///     block,
    ///     r#"{"seamStateVersion":1,"pluginStateVersion":0,"parameters":{"bypass":true}}"#
    /// );
    /// # Ok(())
    /// # }
    /// ```
    pub fn save(&self) -> Vec<u8> {
        let last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
        let declared = self.set.ids().map(|id| {
            let decl = self.set.decl(id);
            entry(decl.key, &value_json(decl, self.set.value(id)))
        });
        let kept = sorted(&last.unknown.parameters)
            .into_iter()
            .map(|(key, value)| entry(key, value));
        let parameters: Vec<String> = declared.chain(kept).collect();

        // Written by hand, since `serde_json`'s map would sort the keys.
        let Versions { seam, plugin } = self.versions().newest(last.versions);
        let mut block =
            format!("{{\"{SEAM_VERSION}\":{seam},\"{PLUGIN_VERSION}\":{plugin},\"parameters\":{{");
        block.push_str(&parameters.join(","));
        block.push('}');
        let extra = self.session.save();
        // What a load would count for `extra`: its text, or the `{` of an
        // empty one.
        let extra_bytes = extra.as_ref().map_or(1, json_bytes);
        let room = self
            .room_for_extra(&last.unknown)
            .saturating_sub(extra_bytes);
        self.run_time.write(&mut block, room);
        if let Some(extra) = extra {
            block.push_str(",\"extra\":");
            block.push_str(&extra.to_string());
        }
        for (key, value) in sorted(&last.unknown.root) {
            block.push(',');
            block.push_str(&entry(key, value));
        }
        block.push('}');
        block.into_bytes()
    }

    /// Loads a block, or says why it was not recognised.
    ///
    /// Thread: any but audio.
    ///
    /// Trailing NUL bytes are ignored: a host that stores blocks in
    /// fixed-size chunks pads them with these. A block whose plugin version
    /// is older than this build's is first upgraded by the plugin's
    /// [migration](Self::with_migration), which gets the block's plugin
    /// version. A block of a newer frame or plugin version loads what this
    /// build reads.
    ///
    /// A recognised block, of any frame version from 1 up, assigns every
    /// declared parameter, so the same block always gives the same state. A
    /// value that is missing, not a number or a bool, or outside `f32`'s
    /// range takes the parameter's default. Anything else goes through the
    /// restore write, which snaps it. A number past `f64`'s range, anywhere
    /// in the block, reads as `null`, so it alone is refused and the rest
    /// loads. Parameter keys this build does not declare, and top-level keys
    /// it does not read, replace the ones kept before and are saved back. The
    /// load marks the restore as stored before its first write, so no undo
    /// history replays anything from before it. Then the session stores
    /// `extra`, which it applies at a later tick.
    ///
    /// The load writes no parameter made at run time: on this thread those
    /// are the tree the load replaces, still playing. It holds `runTime`'s
    /// entries instead, replacing what an earlier load held. A parameter made
    /// under a held path is born holding its value. Once a pass of the
    /// session's tick that began after this load has finished, in one step,
    /// each one made before the load and still held takes the value for its
    /// path, or its default, every one still held takes the mapping held for
    /// its path, and the entries go. The entries count against the room the
    /// block leaves for `extra`, each value at its longest, so a block never
    /// saves back longer than it loaded.
    ///
    /// # Errors
    ///
    /// Returns the [`LoadError`] that names the check the block failed. A
    /// refused block changes nothing at all, parameters or session.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_host::SavedState;
    /// use seam_params::{Kind, ParamDecl, ParamSet};
    /// use seam_session::Session;
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let set = ParamSet::from_decls([
    ///     ParamDecl::new("gain", "Gain", Kind::continuous(-24.0, 24.0, 0.1, None), 0.0)
    ///         .with_unit("dB"),
    /// ])?;
    /// let gain = set.id("gain").ok_or("declared")?;
    /// let state = SavedState::new(&set, &Session::new(&set));
    ///
    /// // The value snaps like any write.
    /// state.load(br#"{"seamStateVersion":1,"parameters":{"gain":99}}"#)?;
    /// assert_eq!(set.value(gain), 24.0);
    ///
    /// // A block that is not recognised changes nothing.
    /// assert!(state.load(b"not a saved block").is_err());
    /// assert_eq!(set.value(gain), 24.0);
    /// # Ok(())
    /// # }
    /// ```
    pub fn load(&self, bytes: &[u8]) -> Result<(), LoadError> {
        if bytes.len() > MAX_STATE_BYTES {
            return Err(LoadError::TooLarge);
        }
        let mut bytes = bytes;
        while let [rest @ .., 0] = bytes {
            bytes = rest;
        }
        let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
        let mut root = match serde_json::from_slice::<Json>(&finite_numbers(bytes)) {
            Ok(Json::Object(root)) => root,
            Ok(_) => return Err(LoadError::NotAnObject),
            Err(_) => return Err(LoadError::NotJson),
        };
        let versions = versions(&root)?;
        // Seam's frame has had one shape, so only the plugin's schema
        // migrates.
        if let Some(migration) = &self.migration
            && versions.plugin < migration.version
        {
            (migration.migrate)(versions.plugin, &mut root).map_err(LoadError::MigrationFailed)?;
        }
        let Some(Json::Object(mut parameters)) = root.remove("parameters") else {
            return Err(LoadError::BadParameters);
        };
        // Taken out first, so what is left is what this build does not read.
        let declared: Vec<Option<Json>> = self
            .set
            .ids()
            .map(|id| parameters.remove(self.set.decl(id).key))
            .collect();
        root.remove(SEAM_VERSION);
        root.remove(PLUGIN_VERSION);
        let run_time = Loaded::read(root.remove(RUN_TIME));
        let extra = Restored::new(root.remove("extra"));
        let unknown = Unknown { parameters, root };
        // The run-time entries are counted as a save writes them back, each
        // value at its longest, so a block never saves back longer.
        let room = self
            .room_for_extra(&unknown)
            .saturating_sub(run_time.bytes());
        if extra.bytes() > room {
            return Err(LoadError::TooLarge);
        }
        let load = self.set.program().load();
        for (id, value) in self.set.ids().zip(declared) {
            let decl = self.set.decl(id);
            // A number outside f32's range becomes infinite here, and the
            // restore write gives the default for that.
            let value = value.as_ref().and_then(value_number);
            load.restore(id, value.unwrap_or_else(|| decl.default_value()));
        }
        *last = LastLoad { versions, unknown };
        // After the parameters' writes and before the session's store, so
        // the count tells a pass that began after the store from one that
        // began before it. Which parameters were declared before this load
        // the registry decides by its own count of loads.
        let at = self.set.clock().writer().stored_count();
        self.run_time.hold(run_time, at);
        // The sections and their budget under one lock, so no capture is
        // judged against the budget before this one.
        self.session.restore_within(extra, room);
        Ok(())
    }
}

/// A block's versions: the frame's a whole number from 1 up, and the
/// plugin's one from 0 up, 0 when the block has none.
fn versions(root: &Map<String, Json>) -> Result<Versions, LoadError> {
    let seam = root
        .get(SEAM_VERSION)
        .and_then(|value| whole(value, 1))
        .ok_or(LoadError::BadVersion)?;
    let plugin = match root.get(PLUGIN_VERSION) {
        None => 0,
        Some(value) => whole(value, 0).ok_or(LoadError::BadPluginVersion)?,
    };
    Ok(Versions { seam, plugin })
}

/// A whole number from `least` up, as an integer or a float, a float past
/// `u64`'s range read as `u64::MAX`.
fn whole(value: &Json, least: u64) -> Option<u64> {
    let Json::Number(number) = value else {
        return None;
    };
    match number.as_u64() {
        Some(whole) => (whole >= least).then_some(whole),
        None => {
            let float = number.as_f64()?;
            // `as` saturates a float past `u64`'s range.
            (float >= least as f64 && float.fract() == 0.0).then_some(float as u64)
        }
    }
}

/// `text` with every number past `f64`'s range written as `null`, which
/// the JSON parser would refuse the whole text for; the text itself when it
/// has none.
///
/// A token JSON does not allow as a number, `01e400` or `1.e400`, stays as it
/// is, so the parser refuses the text.
fn finite_numbers(text: &[u8]) -> Cow<'_, [u8]> {
    let mut nulls = Vec::new();
    let (mut index, mut in_string, mut escaped) = (0, false, false);
    while index < text.len() {
        let byte = text[index];
        if in_string {
            match byte {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            index += 1;
        } else if byte == b'"' {
            in_string = true;
            index += 1;
        } else if byte == b'-' || byte.is_ascii_digit() {
            // Outside a string only a number has a digit or a minus sign.
            let start = index;
            index += text[start..]
                .iter()
                .take_while(|byte| matches!(byte, b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E'))
                .count();
            let token = &text[start..index];
            let number = std::str::from_utf8(token)
                .ok()
                .filter(|_| is_json_number(token));
            if number
                .and_then(|number| number.parse::<f64>().ok())
                .is_some_and(f64::is_infinite)
            {
                nulls.push(start..index);
            }
        } else {
            index += 1;
        }
    }
    if nulls.is_empty() {
        return Cow::Borrowed(text);
    }
    let mut finite = Vec::with_capacity(text.len());
    let mut from = 0;
    for range in nulls {
        finite.extend_from_slice(&text[from..range.start]);
        finite.extend_from_slice(b"null");
        from = range.end;
    }
    finite.extend_from_slice(&text[from..]);
    Cow::Owned(finite)
}

/// Returns whether `token` is a number as JSON writes one: an optional minus,
/// an integer part with no leading zero, then an optional fraction and an
/// optional exponent, each with a digit at least.
fn is_json_number(token: &[u8]) -> bool {
    let digits = |at: usize| {
        token[at..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count()
    };
    let mut at = usize::from(token.first() == Some(&b'-'));
    match token.get(at) {
        Some(b'0') => at += 1,
        Some(b'1'..=b'9') => at += digits(at),
        _ => return false,
    }
    if token.get(at) == Some(&b'.') {
        let fraction = digits(at + 1);
        if fraction == 0 {
            return false;
        }
        at += 1 + fraction;
    }
    if matches!(token.get(at), Some(b'e' | b'E')) {
        at += 1;
        if matches!(token.get(at), Some(b'+' | b'-')) {
            at += 1;
        }
        let exponent = digits(at);
        if exponent == 0 {
            return false;
        }
        at += exponent;
    }
    at == token.len()
}

#[cfg(test)]
mod tests;
