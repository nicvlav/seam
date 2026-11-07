//! The sections every thread but audio shares: saves and restores.

use std::collections::HashSet;
use std::io;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use seam_params::{ParamSet, RestoreWriter, audio_thread};

use crate::ui::SessionUi;
use crate::{Section, SessionError};

/// A restore's number, from a counter that only grows; 0 before any.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Stamp(u64);

/// A registration's handle into the shared state, never reused.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Id(u64);

/// A section as the session keeps it: shared, so a save copies it outside
/// the lock, and measured, so a capture's budget check reads no other section.
/// An `Option<Kept>` of `None` is nothing to save.
#[derive(Debug, Clone)]
pub(crate) struct Kept {
    pub(crate) section: Arc<Section>,
    /// The bytes of its JSON text, as a save writes it.
    bytes: usize,
}

#[cfg(debug_assertions)]
impl Drop for Kept {
    /// A debug build checks that no section is freed under the lock, where a
    /// large one would hold up every other thread.
    fn drop(&mut self) {
        let last = Arc::get_mut(&mut self.section).is_some();
        if last && LOCKED.with(std::cell::Cell::get) > 0 && !std::thread::panicking() {
            panic!("a session section was freed under the session's lock");
        }
    }
}

/// A JSON `null` is nothing to save.
fn kept(section: Section) -> Option<Kept> {
    (!section.is_null()).then(|| Kept {
        bytes: json_len(&section),
        section: Arc::new(section),
    })
}

/// Counts the bytes handed to it, so a JSON text's length is known without
/// building the text.
struct Count(usize);

impl io::Write for Count {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The bytes of `section`'s JSON text, as `to_string` writes it.
fn json_len(section: &Section) -> usize {
    let mut count = Count(0);
    serde_json::to_writer(&mut count, section).expect("a JSON value always encodes");
    count.0
}

/// The bytes one section takes in the `extra` object: its key as a JSON
/// string, the `:`, the section, and the `,` or `}` after it.
fn entry_bytes(key: &str, section: &Kept) -> usize {
    let mut count = Count(0);
    serde_json::to_writer(&mut count, key).expect("a string always encodes");
    count.0 + 2 + section.bytes
}

/// The bytes of an `extra` object of `sections`: the `{`, then each entry.
fn object_bytes<'a>(sections: impl Iterator<Item = (&'a str, &'a Kept)>) -> usize {
    1 + sections
        .map(|(key, section)| entry_bytes(key, section))
        .sum::<usize>()
}

/// The longest saved block a load reads, 64 MiB, which `seam_host` publishes
/// as its own `MAX_STATE_BYTES`.
///
/// A session's budget starts here, so no capture grows `extra` past the whole
/// block before a saved state sets the budget its block leaves.
#[doc(hidden)]
pub const MAX_STATE_BYTES: usize = 64 * 1024 * 1024;

/// How many levels of JSON the saved block puts around a section: the block's
/// own object and its `extra` object.
const BLOCK_LEVELS: usize = 2;

/// How many levels of arrays and objects `serde_json` reads before it
/// refuses a document: 127 nested levels load, 128 do not.
const LOAD_LEVELS: usize = 127;

/// The deepest section a capture may return: 125 levels of arrays and objects,
/// a scalar counting as level 0.
///
/// The saved block could not be loaded again with a deeper one, so it is
/// refused, as [`Registration::refused`](crate::Registration::refused) says.
pub const MAX_SECTION_DEPTH: usize = LOAD_LEVELS - BLOCK_LEVELS;

/// Returns whether `section` nests no deeper than [`MAX_SECTION_DEPTH`].
/// Walks with its own stack, so a hostile depth cannot overflow the thread's.
fn within_depth(section: &Section) -> bool {
    let mut open = vec![(section, 0_usize)];
    while let Some((value, depth)) = open.pop() {
        let deeper = |child| (child, depth + 1);
        match value {
            Section::Array(_) | Section::Object(_) if depth == MAX_SECTION_DEPTH => return false,
            Section::Array(items) => open.extend(items.iter().map(deeper)),
            Section::Object(map) => open.extend(map.values().map(deeper)),
            _ => {}
        }
    }
    true
}

/// A capture the session refuses: nested deeper than [`MAX_SECTION_DEPTH`],
/// or too large for the saved block's budget.
#[derive(Debug)]
pub(crate) struct Refused;

/// A capture as the session keeps it, measured, unless it nests too deep.
/// Runs outside the lock, since measuring a large section takes a while.
pub(crate) fn accept(section: Section) -> Result<Option<Kept>, Refused> {
    if within_depth(&section) {
        Ok(kept(section))
    } else {
        Err(Refused)
    }
}

/// What a save writes for a registration, and whether it waits for an apply.
#[derive(Debug)]
pub(crate) enum Saved {
    /// No apply waits: the last capture taken, or the section an apply left
    /// saved.
    Settled(Option<Kept>),
    /// A section waiting for the participant's apply: a restore's, or one a
    /// registration that ended left under the key. `None` resets the
    /// participant to its defaults.
    Pending(Option<Kept>),
}

impl Saved {
    pub(crate) fn section(&self) -> Option<&Kept> {
        match self {
            Saved::Settled(section) | Saved::Pending(section) => section.as_ref(),
        }
    }

    fn into_section(self) -> Option<Kept> {
        match self {
            Saved::Settled(section) | Saved::Pending(section) => section,
        }
    }

    pub(crate) fn is_pending(&self) -> bool {
        matches!(self, Saved::Pending(_))
    }

    /// Ends the wait for the apply, keeping the pending section saved.
    pub(crate) fn settle(&mut self) {
        if let Saved::Pending(section) = self {
            *self = Saved::Settled(section.take());
        }
    }
}

/// A registration's key and the section a save writes under it.
#[derive(Debug)]
pub(crate) struct Claim {
    pub(crate) id: Id,
    key: String,
    pub(crate) saved: Saved,
    /// The last capture nested too deep or had no room, or a lowered budget
    /// dropped the saved section, and no capture has been taken since.
    pub(crate) refused: bool,
}

#[derive(Debug)]
pub(crate) struct State {
    /// In registration order.
    pub(crate) claims: Vec<Claim>,
    /// Sections no registration claims: a restore's for keys nobody
    /// registered, and the last section of each registration that ended. A
    /// key is here or claimed, never both.
    unclaimed: Vec<(String, Kept)>,
    /// The newest restore's stamp. An apply or a capture that began before a
    /// newer restore stored its section does not commit.
    pub(crate) restores: Stamp,
    // `carried`, `answered` and `owed` are the ui module overview's.
    carried: Option<HashSet<String>>,
    pub(crate) answered: HashSet<String>,
    pub(crate) owed: bool,
    next_id: Id,
    /// The most bytes the `extra` object may take ([`Session::set_budget`]).
    budget: usize,
}

impl Default for State {
    fn default() -> Self {
        State {
            claims: Vec::new(),
            unclaimed: Vec::new(),
            restores: Stamp::default(),
            carried: None,
            answered: HashSet::new(),
            owed: false,
            next_id: Id::default(),
            budget: MAX_STATE_BYTES,
        }
    }
}

impl State {
    pub(crate) fn claim(&self, id: Id) -> Option<&Claim> {
        self.claims.iter().find(|claim| claim.id == id)
    }

    /// Every section a save writes, under its key.
    fn sections(&self) -> impl Iterator<Item = (&str, &Kept)> {
        let claimed = self.claims.iter().filter_map(|claim| {
            let saved = claim.saved.section()?;
            Some((claim.key.as_str(), saved))
        });
        let unclaimed = self
            .unclaimed
            .iter()
            .map(|(key, section)| (key.as_str(), section));
        claimed.chain(unclaimed)
    }

    /// Returns whether `key`'s saved section may change from `old` to `new`:
    /// it takes no more room than before, or the `extra` object fits the
    /// budget with it.
    fn room_for(&self, key: &str, old: Option<&Kept>, new: Option<&Kept>) -> bool {
        let size = |section: Option<&Kept>| section.map_or(0, |section| entry_bytes(key, section));
        let (old, new) = (size(old), size(new));
        new <= old || object_bytes(self.sections()) - old + new <= self.budget
    }

    /// Claims `key` for a registering participant, and returns the claim's id.
    ///
    /// Its section is the one waiting unclaimed under `key`, a reset when the
    /// newest restore lacked the key and no registration took that answer,
    /// or else `capture`, refused when it has no room. Takes `capture` only
    /// when it becomes the saved section, so the caller drops any other after
    /// the lock.
    pub(crate) fn register(
        &mut self,
        key: &str,
        capture: &mut Option<Kept>,
        too_deep: bool,
    ) -> Result<Id, SessionError> {
        if self.claims.iter().any(|claim| claim.key == key) {
            return Err(SessionError::KeyTaken(key.to_owned()));
        }
        let id = self.next_id;
        self.next_id.0 += 1;
        let waiting = self
            .unclaimed
            .iter()
            .position(|(unclaimed, _)| unclaimed == key);
        let mut refused = too_deep;
        let saved = match waiting {
            Some(index) => Saved::Pending(Some(self.unclaimed.remove(index).1)),
            None if self
                .carried
                .as_ref()
                .is_some_and(|carried| !carried.contains(key))
                && !self.answered.contains(key) =>
            {
                Saved::Pending(None)
            }
            None if self.room_for(key, None, capture.as_ref()) => Saved::Settled(capture.take()),
            None => {
                refused = true;
                Saved::Settled(None)
            }
        };
        self.claims.push(Claim {
            id,
            key: key.to_owned(),
            saved,
            refused,
        });
        Ok(id)
    }

    /// Makes `capture` the saved section of the claim at `index` when it has
    /// room, and records the answer as the claim's refused mark.
    ///
    /// Returns the section no longer saved, the one replaced or the capture
    /// refused, for the caller to drop after the lock.
    pub(crate) fn keep(&mut self, index: usize, capture: Option<Kept>) -> Option<Kept> {
        let claim = &self.claims[index];
        let fits = self.room_for(&claim.key, claim.saved.section(), capture.as_ref());
        let claim = &mut self.claims[index];
        claim.refused = !fits;
        if fits {
            std::mem::replace(&mut claim.saved, Saved::Settled(capture)).into_section()
        } else {
            capture
        }
    }

    /// Marks the claim `id` refused, for a capture nested too deep.
    pub(crate) fn refuse(&mut self, id: Id) {
        if let Some(claim) = self.claims.iter_mut().find(|claim| claim.id == id) {
            claim.refused = true;
        }
    }

    /// Drops each saved section the budget no longer has room for, as
    /// [`Session::set_budget`] describes, and returns them, to drop after the
    /// lock.
    fn refuse_past_budget(&mut self) -> Vec<Kept> {
        let mut dropped = Vec::new();
        if object_bytes(self.sections()) <= self.budget {
            return dropped;
        }
        let budget = self.budget;
        let mut used = 1;
        let mut takes_room = |key: &str, section: &Kept| {
            let bytes = entry_bytes(key, section);
            let fits = used + bytes <= budget;
            if fits {
                used += bytes;
            }
            fits
        };
        for claim in &mut self.claims {
            if let Some(section) = claim.saved.section()
                && !takes_room(&claim.key, section)
            {
                // A pending section goes too, so no apply can hand it to the
                // participant or save it back.
                let saved = std::mem::replace(&mut claim.saved, Saved::Settled(None));
                dropped.extend(saved.into_section());
                claim.refused = true;
            }
        }
        // In key order, as a save writes them, so which are dropped does not
        // follow the order a map handed them over in.
        let mut unclaimed = std::mem::take(&mut self.unclaimed);
        unclaimed.sort_by(|a, b| a.0.cmp(&b.0));
        for (key, section) in unclaimed {
            if takes_room(&key, &section) {
                self.unclaimed.push((key, section));
            } else {
                dropped.push(section);
            }
        }
        dropped
    }

    /// Ends a registration: its saved section, pending or not, stays
    /// unclaimed under its key unless it is nothing to save. One lock, so a
    /// save sees the section exactly once.
    pub(crate) fn end(&mut self, id: Id) {
        let index = self.claims.iter().position(|claim| claim.id == id);
        if let Some(claim) = index.map(|index| self.claims.remove(index))
            && let Some(section) = claim.saved.into_section()
        {
            self.unclaimed.push((claim.key, section));
        }
    }
}

#[derive(Debug)]
pub(crate) struct Shared {
    state: Mutex<State>,
    pub(crate) clock: RestoreWriter,
    ui_taken: AtomicBool,
}

#[cfg(debug_assertions)]
thread_local! {
    /// How many session locks this thread holds, for [`Kept`]'s drop.
    static LOCKED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// The session's state, locked until this drops.
pub(crate) struct Locked<'a>(MutexGuard<'a, State>);

impl Deref for Locked<'_> {
    type Target = State;

    fn deref(&self) -> &State {
        &self.0
    }
}

impl DerefMut for Locked<'_> {
    fn deref_mut(&mut self) -> &mut State {
        &mut self.0
    }
}

impl Drop for Locked<'_> {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        LOCKED.with(|locked| locked.set(locked.get() - 1));
    }
}

impl Shared {
    /// Never poisoned for good: no participant code runs under it.
    pub(crate) fn lock(&self) -> Locked<'_> {
        audio_thread::assert_not_here("a Session call");
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        #[cfg(debug_assertions)]
        LOCKED.with(|locked| locked.set(locked.get() + 1));
        Locked(state)
    }

    pub(crate) fn release_ui(&self) {
        self.ui_taken.store(false, Ordering::Release);
    }
}

/// A plugin instance's session: every participant's section, readable and
/// restorable from any thread but the audio thread. Clones share it.
///
/// A save writes what each participant last captured. A restore stores
/// every section at once and leaves the apply to a later UI tick. Neither a
/// save nor a restore runs participant code, waits for the UI thread, or holds
/// a lock while participant code runs.
///
/// Thread: any but audio.
///
/// # Examples
///
/// ```
#[doc = include_str!("../doc/setup.rs")]
/// let session = Session::new(&ParamSetBuilder::new().build());
/// let ui = session.take_ui().ok_or("the first UI handle")?;
/// let zoom = Rc::new(RefCell::new(Zoom(2.0)));
/// let _registration = ui.register("zoom", &zoom)?;
///
/// // The host restores on its own thread: stored at once, applied on the
/// // next UI tick.
/// let host = session.clone();
/// std::thread::spawn(move || host.restore(Some(&json!({ "zoom": 4.0 }))))
///     .join()
///     .map_err(|_| "the host's thread")?;
/// assert_eq!(session.save(), Some(json!({ "zoom": 4.0 })));
/// assert_eq!(zoom.borrow().0, 2.0);
/// ui.tick();
/// assert_eq!(zoom.borrow().0, 4.0);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Panics
///
/// In a build with `debug_assertions`, every method that locks panics while
/// the thread holds an [`audio_thread`] mark.
#[derive(Debug, Clone)]
pub struct Session {
    shared: Arc<Shared>,
}

impl Session {
    /// Creates an empty session that writes `set`'s restore clock, which every
    /// undo history reads, as the [`Session`] example shows.
    pub fn new(set: &ParamSet) -> Session {
        Session {
            shared: Arc::new(Shared {
                state: Mutex::new(State::default()),
                clock: set.clock().writer(),
                ui_taken: AtomicBool::new(false),
            }),
        }
    }

    /// Returns whether it was made from `set`, so it writes that set's clock.
    pub fn uses(&self, set: &ParamSet) -> bool {
        self.shared.clock.on(set.clock())
    }

    /// Returns the UI thread's handle: registrations and the apply pass.
    ///
    /// There is one per session, since two would run passes apart: this returns
    /// `None` while one, or a clone of it, is alive. Keep it for the plugin's
    /// life, not an editor's, since dropping it ends its registrations.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::ParamSetBuilder;
    /// use seam_session::Session;
    ///
    /// let session = Session::new(&ParamSetBuilder::new().build());
    /// let ui = session.take_ui().ok_or("the first UI handle")?;
    ///
    /// // A second handle would run passes of its own, so it is refused.
    /// assert!(session.take_ui().is_none());
    /// drop(ui);
    /// assert!(session.take_ui().is_some());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn take_ui(&self) -> Option<SessionUi> {
        let taken = self.shared.ui_taken.swap(true, Ordering::AcqRel);
        (!taken).then(|| SessionUi::new(Arc::clone(&self.shared)))
    }

    /// Returns the `extra` object a save writes, or `None` when there is
    /// nothing to save.
    ///
    /// The object holds each registered participant's section and each section
    /// no participant claims, under its key, in key order, so the same
    /// sections give the same bytes whatever order participants registered in.
    ///
    /// Thread: any but audio.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::ParamSetBuilder;
    /// use seam_session::Session;
    /// use serde_json::json;
    ///
    /// let session = Session::new(&ParamSetBuilder::new().build());
    /// // Nothing to save is `None`, not an empty object.
    /// assert_eq!(session.save(), None);
    ///
    /// // Sections come back under their keys, in key order, whoever holds them.
    /// session.restore(Some(&json!({ "zoom": 2.0, "layout": { "tab": 1 } })));
    /// let extra = session.save().ok_or("two sections")?;
    /// let keys: Vec<_> = extra.as_object().ok_or("an object")?.keys().collect();
    /// assert_eq!(keys, ["layout", "zoom"]);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn save(&self) -> Option<Section> {
        let mut sections: Vec<(String, Arc<Section>)> = self
            .shared
            .lock()
            .sections()
            .map(|(key, kept)| (key.to_owned(), Arc::clone(&kept.section)))
            .collect();
        if sections.is_empty() {
            return None;
        }
        // Inserted in key order, so the bytes are the same whether or not
        // some crate in the build turns on serde_json's `preserve_order`.
        sections.sort_by(|a, b| a.0.cmp(&b.0));
        let map = sections
            .into_iter()
            .map(|(key, section)| (key, Section::clone(&section)))
            .collect();
        Some(Section::Object(map))
    }

    /// Restores every section from the saved block's `extra` (`None` when it
    /// had none).
    ///
    /// Every section is stored at once, so a save straight after returns
    /// exactly what was restored.
    ///
    /// - Each registered participant's section waits for its apply on a later
    ///   [`SessionUi::tick`].
    /// - A participant whose key is missing, or whose section is `null`, or
    ///   every participant when `extra` is not an object, is reset to its
    ///   defaults.
    /// - Sections no participant claims are saved back verbatim, and apply to
    ///   the next participant that registers the key.
    /// - A participant that registers later under a key this restore lacked is
    ///   reset to its defaults once; one registering the same key after that
    ///   keeps its own state.
    ///
    /// A restore bumps the restore clock's stored count, so every undo history
    /// clears before anything from before the restore can replay.
    ///
    /// Thread: any but audio.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::ParamSetBuilder;
    /// use seam_session::Session;
    /// use serde_json::json;
    ///
    /// let session = Session::new(&ParamSetBuilder::new().build());
    ///
    /// // A host's restore call: even a key no participant holds yet is kept.
    /// let extra = json!({ "layout": { "tab": 2 } });
    /// session.restore(Some(&extra));
    /// assert_eq!(session.save(), Some(extra));
    ///
    /// // A state with no `extra` empties the session.
    /// session.restore(None);
    /// assert_eq!(session.save(), None);
    /// ```
    pub fn restore(&self, extra: Option<&Section>) {
        // Copied before the lock: the caller's value stays theirs.
        self.store(Restored::new(extra.cloned()), None);
    }

    /// Stores the sections `restored` holds, as [`restore`](Self::restore)
    /// stores the ones it copies, and sets the budget to `budget` under the
    /// same lock.
    ///
    /// Plumbing for the saved state, whose load checks that `restored` fits
    /// `budget` before it changes anything. The restored sections replace
    /// every other, so none is judged again. One lock, so no capture is
    /// judged against the budget before the load's.
    ///
    /// Thread: any but audio.
    #[doc(hidden)]
    pub fn restore_within(&self, restored: Restored, budget: usize) {
        self.store(restored, Some(budget));
    }

    /// Stores a restore's sections, and the budget its caller checked them
    /// against, when it has one.
    fn store(&self, restored: Restored, budget: Option<usize>) {
        let Restored { mut sections } = restored;
        let carried = sections.iter().map(|(key, _)| key.clone()).collect();
        let mut old = Vec::new();
        let old_unclaimed = {
            let mut state = self.shared.lock();
            // Before anything changes, so an undo from here on replays
            // nothing from before the restore.
            self.shared.clock.stored();
            if let Some(budget) = budget {
                state.budget = budget;
            }
            state.restores.0 += 1;
            state.owed = true;
            old.reserve(state.claims.len());
            for claim in &mut state.claims {
                let section = sections
                    .iter()
                    .position(|(key, _)| *key == claim.key)
                    .map(|index| sections.remove(index).1);
                old.push(std::mem::replace(&mut claim.saved, Saved::Pending(section)));
            }
            state.carried = Some(carried);
            state.answered.clear();
            std::mem::replace(&mut state.unclaimed, sections)
        };
        // Dropped after the unlock, so a section's destructor never runs under
        // it.
        drop((old, old_unclaimed));
    }

    /// Sets the most bytes the `extra` object a save returns may take: a
    /// capture that would grow it past `bytes` is refused, as one nested too
    /// deep is.
    ///
    /// The sections saved already are judged against it too, the registered
    /// ones in registration order, then those no participant claims in key
    /// order. Each that no longer has room is dropped, so its key saves
    /// nothing, and a registered one is refused, a pending section with it, so
    /// the participant keeps the state it has. A restore stores its sections
    /// whatever their size, so its caller checks them first.
    ///
    /// Plumbing for the saved state, which sets it at creation from the room
    /// its block leaves for `extra`, so a save never writes a block longer
    /// than a load reads, whether the saved state or a registration comes
    /// first. A load sets it with its sections, through
    /// [`restore_within`](Self::restore_within).
    ///
    /// Thread: any but audio.
    #[doc(hidden)]
    pub fn set_budget(&self, bytes: usize) {
        let dropped = {
            let mut state = self.shared.lock();
            state.budget = bytes;
            state.refuse_past_budget()
        };
        // Dropped after the unlock, so a section's destructor never runs under
        // it.
        drop(dropped);
    }
}

/// A restore's sections, copied and measured before the session stores them.
///
/// Plumbing for the saved state, whose load refuses a block that would save
/// back past [`MAX_STATE_BYTES`] before it changes anything.
#[doc(hidden)]
#[derive(Debug)]
pub struct Restored {
    sections: Vec<(String, Kept)>,
}

impl Restored {
    /// Measures the sections of `extra` that a restore stores: none when it
    /// is `None` or not an object, and none for a `null` section.
    pub fn new(extra: Option<Section>) -> Restored {
        let sections = match extra {
            Some(Section::Object(map)) => map
                .into_iter()
                .filter_map(|(key, section)| Some((key, kept(section)?)))
                .collect(),
            _ => Vec::new(),
        };
        Restored { sections }
    }

    /// Returns the bytes of the `extra` object a save writes once these are
    /// the session's only sections, as its budget counts them.
    pub fn bytes(&self) -> usize {
        object_bytes(
            self.sections
                .iter()
                .map(|(key, section)| (key.as_str(), section)),
        )
    }
}
