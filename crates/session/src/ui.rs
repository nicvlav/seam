//! The UI thread's side of the session: registrations, captures and the
//! apply pass.
//!
//! # Entry states
//!
//! A registration has an entry on this side, whose `Status` says what a capture
//! does with it, and a claim in the shared state, whose `Saved` section is
//! pending while it waits for an apply. Only this thread changes a status,
//! while a restore on any thread can make a section pending.
//!
//! | `Status` | What it means | While its section is pending |
//! | --- | --- | --- |
//! | `Current` | A capture reads the participant. | Captures are skipped, and the restored section stays the saved one until a pass applies it. |
//! | `HeldUp` | The participant's own code held it when a capture came; the next tick captures it. | The same; the next capture finds the section pending and makes the entry `Current`. |
//! | `Faulted` | Its last apply panicked, which can stop an apply halfway. Its raw section stays the saved one, and captures are ignored. | The same, until a pass applies it. |
//! | `Ended` | Every pass and capture skips it; its section stays for a later registration of the key. | Never: ending moves its section out of the claims. |
//!
//! An apply that takes a pending section moves the status as `Status::after`
//! says. A claim is also `refused` while its last capture nested too deep or
//! had no room in the budget, and the next capture that fits clears it. That
//! mark lives in the shared state, beside the section, because a lowered
//! budget sets it on whichever thread lowers it.
//!
//! `HeldUp` and `Faulted` entries are the glossary's [held up and
//! faulted][glossary-held] participants.
//!
//! The state the entries share, all in the shared `State` but `passing`:
//!
//! - `owed`: a restore stored a section no pass has taken, or a pass found a
//!   participant busy or unwound. The next tick runs a pass and clears it.
//! - `passing`: a pass or a late apply runs on this thread. A tick inside does
//!   nothing, so passes never nest.
//! - `carried`: the keys the newest restore had, `None` before any restore.
//! - `answered`: the keys that took the newest restore's answer, in a pass or
//!   late. Each restore clears it. A key that is not `carried` and not
//!   `answered` is reset once when it registers; after that it is `answered`.
//!
//! [glossary-held]: https://github.com/nicvlav/seam/blob/main/docs/glossary.md#held

use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::{Rc, Weak};
use std::sync::Arc;

use seam_params::ParamSet;

use crate::session::{Id, Kept, Refused, Saved, Shared, Stamp, accept};
use crate::{Participant, Section, SessionError};

/// What a capture does with an entry, as the module overview's table says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Current,
    HeldUp,
    Faulted,
    /// Set the moment the registration ends, so a pass walking an older list
    /// skips it.
    Ended,
}

impl Status {
    /// The status once an apply that ended as `applied` commits.
    fn after(self, applied: Applied) -> Status {
        match (self, applied) {
            (Status::Ended, _) => Status::Ended,
            (_, Applied::Succeeded) => Status::Current,
            (_, Applied::Panicked) => Status::Faulted,
            // A failed apply leaves the participant as it was, a fault
            // included.
            (Status::Faulted, Applied::Failed) => Status::Faulted,
            (Status::Current | Status::HeldUp, Applied::Failed) => Status::Current,
        }
    }
}

/// One registration, as the UI side reaches its participant.
struct Entry {
    id: Id,
    key: String,
    participant: Weak<RefCell<dyn Participant>>,
    status: Cell<Status>,
}

impl Entry {
    fn live(&self) -> bool {
        self.status.get() != Status::Ended
    }
}

struct UiState {
    shared: Arc<Shared>,
    /// In registration order.
    entries: RefCell<Vec<Rc<Entry>>>,
    passing: Cell<bool>,
}

impl Drop for UiState {
    /// Ends every registration made through it, keeping their sections, so
    /// a later handle can register their keys again.
    fn drop(&mut self) {
        let entries = std::mem::take(self.entries.get_mut());
        {
            let mut state = self.shared.lock();
            for entry in &entries {
                entry.status.set(Status::Ended);
                state.end(entry.id);
            }
        }
        self.shared.release_ui();
    }
}

/// Marks a pass or a late apply as running until it drops, then puts the
/// mark back. If a panic that began inside it unwinds it, the pass did not
/// finish, so it is owed again: the next tick runs it, and no later restore
/// is stuck.
struct Passing<'a> {
    ui: &'a UiState,
    was: bool,
    /// The thread was already unwinding when it began, as from a drop.
    unwinding: bool,
}

impl<'a> Passing<'a> {
    fn new(ui: &'a UiState) -> Self {
        let was = ui.passing.replace(true);
        let unwinding = std::thread::panicking();
        Passing { ui, was, unwinding }
    }
}

impl Drop for Passing<'_> {
    fn drop(&mut self) {
        self.ui.passing.set(self.was);
        if std::thread::panicking() && !self.unwinding {
            self.ui.shared.lock().owed = true;
        }
    }
}

/// How the participant's apply ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Applied {
    Succeeded,
    Failed,
    Panicked,
}

/// The participant's own code holds it, so its section waits, owed, for a
/// later pass.
struct Busy;

impl UiState {
    /// Applies `entry`'s pending section, if any, then re-captures it and
    /// commits, unless a newer restore stored its section meanwhile.
    ///
    /// No lock is held while the participant runs; a failed or panicking
    /// apply leaves the participant as it left it and keeps its raw section.
    fn apply(&self, entry: &Entry) -> Result<(), Busy> {
        if !entry.live() {
            return Ok(());
        }
        let Some(participant) = entry.participant.upgrade() else {
            return Ok(());
        };
        let Some((section, stamp)) = self.pending_of(entry) else {
            return Ok(());
        };
        let applying = section.as_ref().map(|kept| &*kept.section);
        let applied = self.run_apply(&participant, applying)?;
        let capture = if applied == Applied::Succeeded && entry.live() {
            participant.try_borrow().ok().and_then(|participant| {
                catch_unwind(AssertUnwindSafe(|| participant.capture())).ok()
            })
        } else {
            None
        };
        self.commit(entry, stamp, applied, capture.map(accept));
        // Dropped after the commit's unlock.
        drop(section);
        Ok(())
    }

    /// The section pending for `entry`, and the restore it came from.
    fn pending_of(&self, entry: &Entry) -> Option<(Option<Kept>, Stamp)> {
        let state = self.shared.lock();
        match &state.claim(entry.id)?.saved {
            Saved::Pending(section) => Some((section.clone(), state.restores)),
            Saved::Settled(_) => None,
        }
    }

    /// Runs the participant's apply with no lock held.
    fn run_apply(
        &self,
        participant: &Rc<RefCell<dyn Participant>>,
        section: Option<&Section>,
    ) -> Result<Applied, Busy> {
        let Ok(mut participant) = participant.try_borrow_mut() else {
            // Its own code is running further up the stack, or the tick
            // runs while the plugin holds it.
            self.shared.lock().owed = true;
            return Err(Busy);
        };
        // Undo records nothing the apply writes.
        let _applying = self.shared.clock.applying();
        let applied = match catch_unwind(AssertUnwindSafe(|| participant.apply(section))) {
            Ok(Ok(())) => Applied::Succeeded,
            Ok(Err(_)) => Applied::Failed,
            Err(_) => Applied::Panicked,
        };
        Ok(applied)
    }

    /// Ends `entry`'s pending section with the apply's outcome, unless a
    /// newer restore stored its section while it ran: that one waits for the
    /// next pass.
    fn commit(
        &self,
        entry: &Entry,
        stamp: Stamp,
        applied: Applied,
        capture: Option<Result<Option<Kept>, Refused>>,
    ) {
        let unused = {
            let mut state = self.shared.lock();
            let restores = state.restores;
            let Some(index) = state.claims.iter().position(|claim| claim.id == entry.id) else {
                return;
            };
            if !state.claims[index].saved.is_pending() || restores != stamp {
                return;
            }
            entry.status.set(entry.status.get().after(applied));
            state.answered.insert(entry.key.clone());
            // The pending section stays saved unless the re-capture is kept.
            state.claims[index].saved.settle();
            match capture {
                Some(Ok(capture)) => state.keep(index, capture),
                Some(Err(Refused)) => {
                    state.claims[index].refused = true;
                    None
                }
                None => None,
            }
        };
        drop(unused);
    }

    /// Re-captures `entry`'s participant into the session.
    ///
    /// A section pending for it stays the saved one until its apply
    /// re-captures. When the participant's own code holds it, marks the entry
    /// held up instead, so a later tick captures it.
    fn capture(&self, entry: &Entry) {
        match entry.status.get() {
            // A faulted entry's captures are ignored until an apply succeeds.
            Status::Ended | Status::Faulted => return,
            Status::Current | Status::HeldUp => {}
        }
        let stamp = {
            let state = self.shared.lock();
            let Some(claim) = state.claim(entry.id) else {
                return;
            };
            if claim.saved.is_pending() {
                entry.status.set(Status::Current);
                return;
            }
            state.restores
        };
        let Some(participant) = entry.participant.upgrade() else {
            entry.status.set(Status::Current);
            return;
        };
        let Ok(borrowed) = participant.try_borrow() else {
            entry.status.set(Status::HeldUp);
            return;
        };
        entry.status.set(Status::Current);
        let capture = borrowed.capture();
        drop(borrowed);
        let Ok(capture) = accept(capture) else {
            // Too deep to load again: the previous section stays saved.
            self.shared.lock().refuse(entry.id);
            return;
        };
        let unused = {
            let mut state = self.shared.lock();
            let restores = state.restores;
            let index = state.claims.iter().position(|claim| {
                claim.id == entry.id && !claim.saved.is_pending() && restores == stamp
            });
            match index {
                Some(index) => state.keep(index, capture),
                // A newer restore's section waits: the section saved before
                // stays saved.
                None => capture,
            }
        };
        drop(unused);
    }
}

/// The UI thread's handle to a [`Session`](crate::Session): participants
/// register through it, and [`tick`](Self::tick) runs the apply pass.
///
/// Clones share it. Dropping the last clone ends every registration made
/// through it.
///
/// Thread: UI.
///
/// # Examples
///
/// ```
#[doc = include_str!("../doc/setup.rs")]
/// let session = Session::new(&ParamSetBuilder::new().build());
/// // The editor and the plugin hold this one handle for the plugin's life.
/// let ui = session.take_ui().ok_or("the first UI handle")?;
///
/// let zoom = Rc::new(RefCell::new(Zoom(1.0)));
/// let registration = ui.register("zoom", &zoom)?;
/// assert!(registration.connected());
/// ui.tick();
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone)]
pub struct SessionUi {
    state: Rc<UiState>,
}

impl std::fmt::Debug for SessionUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionUi").finish_non_exhaustive()
    }
}

impl SessionUi {
    pub(crate) fn new(shared: Arc<Shared>) -> SessionUi {
        SessionUi {
            state: Rc::new(UiState {
                shared,
                entries: RefCell::new(Vec::new()),
                passing: Cell::new(false),
            }),
        }
    }

    /// Returns whether its session was made from `set`, so it writes that
    /// set's clock.
    pub fn uses(&self, set: &ParamSet) -> bool {
        self.state.shared.clock.on(set.clock())
    }

    /// Registers `participant` under `key` and captures it, so every save
    /// from now on writes its section.
    ///
    /// A section waiting under exactly `key` (restored before anyone
    /// registered it, or left by a registration that ended) is applied before
    /// this returns. With nothing waiting, a participant under a key the newest
    /// restore lacked is reset to its defaults the same way, once. A failed
    /// apply still registers. The capture comes first, so one that panics
    /// registers nothing.
    ///
    /// # Errors
    ///
    /// Changes nothing and returns the [`SessionError`] that says why.
    ///
    /// # Examples
    ///
    /// ```
    #[doc = include_str!("../doc/setup.rs")]
    /// let session = Session::new(&ParamSetBuilder::new().build());
    /// let ui = session.take_ui().ok_or("the first UI handle")?;
    /// let zoom = Rc::new(RefCell::new(Zoom(2.0)));
    ///
    /// let registration = ui.register("zoom", &zoom)?;
    /// assert_eq!(registration.key(), "zoom");
    /// assert_eq!(session.save(), Some(json!({ "zoom": 2.0 })));
    ///
    /// // A key holds one participant at a time.
    /// let other = Rc::new(RefCell::new(Zoom(3.0)));
    /// assert!(ui.register("zoom", &other).is_err());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn register<P: Participant + 'static>(
        &self,
        key: &str,
        participant: &Rc<RefCell<P>>,
    ) -> Result<Registration, SessionError> {
        if key.is_empty() {
            return Err(SessionError::EmptyKey);
        }
        let participant: Rc<RefCell<dyn Participant>> = participant.clone();
        if participant.try_borrow_mut().is_err() {
            return Err(SessionError::Busy);
        }
        let weak = Rc::downgrade(&participant);
        let ui = &self.state;
        if ui
            .entries
            .borrow()
            .iter()
            .any(|entry| entry.live() && Weak::ptr_eq(&entry.participant, &weak))
        {
            return Err(SessionError::AlreadyRegistered);
        }
        let (mut capture, too_deep) = match accept(participant.borrow().capture()) {
            Ok(capture) => (capture, false),
            // Treated as none, so that a section already waiting under this
            // key is not shadowed by a capture too deep to load again.
            Err(Refused) => (None, true),
        };
        let id = ui.shared.lock().register(key, &mut capture, too_deep);
        // What the session did not take drops after the unlock.
        drop(capture);
        let entry = Rc::new(Entry {
            id: id?,
            key: key.to_owned(),
            participant: weak,
            status: Cell::new(Status::Current),
        });
        ui.entries.borrow_mut().push(Rc::clone(&entry));
        // The section waiting for it, if any, applies now. A tick from inside
        // that apply does nothing, as inside a pass.
        let _passing = Passing::new(ui);
        // Never busy: a busy participant was refused above.
        let _ = ui.apply(&entry);
        Ok(Registration {
            ui: Rc::downgrade(ui),
            entry,
        })
    }

    /// Captures every participant whose change its own code held up, then runs
    /// the apply pass a restore left owed, if any.
    ///
    /// Call it once per UI tick, whether or not an editor is open. A restored
    /// state reaches a registered participant, and through it the audio thread,
    /// only here. Each pending section applies in registration order, and
    /// restores stored before the tick collapse into one apply of the newest.
    ///
    /// Do not hold a participant's borrow across the call: a busy participant
    /// leaves its section owed until a tick finds it free. Called from inside
    /// a participant's apply, or the capture that follows it, it does nothing;
    /// from inside any other capture it runs as any tick does.
    ///
    /// Thread: UI.
    ///
    /// # Examples
    ///
    /// ```
    #[doc = include_str!("../doc/setup.rs")]
    /// let session = Session::new(&ParamSetBuilder::new().build());
    /// let ui = session.take_ui().ok_or("the first UI handle")?;
    /// let zoom = Rc::new(RefCell::new(Zoom(1.0)));
    /// let _registration = ui.register("zoom", &zoom)?;
    ///
    /// // Two restores before one tick are one apply, of the newest.
    /// session.restore(Some(&json!({ "zoom": 2.0 })));
    /// session.restore(Some(&json!({ "zoom": 3.0 })));
    /// assert_eq!(zoom.borrow().0, 1.0); // nothing applies until the tick
    /// ui.tick();
    /// assert_eq!(zoom.borrow().0, 3.0);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn tick(&self) {
        let ui = &self.state;
        if ui.passing.get() {
            return;
        }
        let held_up = |entry: &Rc<Entry>| entry.status.get() == Status::HeldUp;
        if ui.entries.borrow().iter().any(held_up) {
            // Participant code may register or end registrations meanwhile.
            let entries = ui.entries.borrow().clone();
            for entry in entries.iter().filter(|entry| held_up(entry)) {
                // A capture a tick makes is contained.
                let _ = catch_unwind(AssertUnwindSafe(|| ui.capture(entry)));
            }
        }
        if !std::mem::take(&mut ui.shared.lock().owed) {
            return;
        }
        let _passing = Passing::new(ui);
        let entries = ui.entries.borrow().clone();
        let mut finished = true;
        for entry in &entries {
            finished &= ui.apply(entry).is_ok();
        }
        // Once per finished pass, also with no participants, so every undo
        // history drops what it recorded since the restore was stored. A pass
        // that left a busy participant owed is not finished: marking it
        // completed would clear every history at every tick for as long as
        // that participant stays busy.
        if finished {
            ui.shared.clock.completed();
        }
    }
}

/// A participant's registration with a session.
///
/// Dropping it ends the registration, and so does dropping the
/// [`SessionUi`] it was made through; it and the session end in either
/// order. When it ends, the participant's last section stays in the session
/// under its key, unless it is nothing to save. Every save writes it, and the
/// next participant to register the key takes it, over whatever that
/// participant holds. While a restored section waits for this participant,
/// that section is the one kept. To forget a section, capture nothing, call
/// [`changed`](Self::changed), then end the registration.
///
/// Thread: UI.
///
/// # Examples
///
/// ```
#[doc = include_str!("../doc/setup.rs")]
/// let session = Session::new(&ParamSetBuilder::new().build());
/// let ui = session.take_ui().ok_or("the first UI handle")?;
/// let zoom = Rc::new(RefCell::new(Zoom(2.0)));
///
/// // Dropping the registration ends it; its last section stays saved.
/// let registration = ui.register("zoom", &zoom)?;
/// drop(registration);
/// assert_eq!(session.save(), Some(json!({ "zoom": 2.0 })));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct Registration {
    ui: Weak<UiState>,
    entry: Rc<Entry>,
}

impl std::fmt::Debug for Registration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registration")
            .field("key", &self.entry.key)
            .finish_non_exhaustive()
    }
}

impl Registration {
    /// Returns the key it saves under, as [`SessionUi::register`] was given.
    pub fn key(&self) -> &str {
        &self.entry.key
    }

    /// Returns whether the registration stands: until it drops, or the
    /// [`SessionUi`] it was made through does. Its participant may be gone
    /// meanwhile; passes then skip it, and its key stays taken.
    pub fn connected(&self) -> bool {
        self.entry.live()
    }

    /// Returns whether the participant's last capture was refused, since the
    /// saved block could not be loaded again with it.
    ///
    /// A capture is refused when it nests deeper than
    /// [`MAX_SECTION_DEPTH`](crate::MAX_SECTION_DEPTH), or when it would grow
    /// the saved block past the longest a load reads. A refused capture
    /// changes nothing: the section saved before stays saved, and the
    /// participant sees no error and nothing is logged. Its next accepted
    /// capture clears this.
    ///
    /// The plugin's saved state, once made, counts the parameters too, and
    /// judges the sections saved before it again: one that then has no room
    /// is dropped and refused the same way, a restored section waiting for its
    /// apply with it, and its key saves nothing until a capture fits. Once the
    /// registration has ended, this returns false.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::cell::RefCell;
    /// use std::rc::Rc;
    ///
    /// use seam_params::ParamSetBuilder;
    /// use seam_session::{ApplyError, MAX_SECTION_DEPTH, Participant, Section, Session};
    /// use serde_json::json;
    ///
    /// struct Tree(usize);
    ///
    /// impl Participant for Tree {
    ///     fn capture(&self) -> Section {
    ///         (0..self.0).fold(json!(1), |inner, _| json!([inner]))
    ///     }
    /// #
    /// #   fn apply(&mut self, _section: Option<&Section>) -> Result<(), ApplyError> {
    /// #       Ok(())
    /// #   }
    /// }
    ///
    /// let session = Session::new(&ParamSetBuilder::new().build());
    /// let ui = session.take_ui().ok_or("the first UI handle")?;
    /// let tree = Rc::new(RefCell::new(Tree(1)));
    /// let registration = ui.register("tree", &tree)?;
    /// let before = session.save();
    ///
    /// // Too deep for the saved block to load again: refused, and the
    /// // section saved before stays saved.
    /// tree.borrow_mut().0 = MAX_SECTION_DEPTH + 1;
    /// registration.changed();
    /// assert!(registration.refused());
    /// assert_eq!(session.save(), before);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn refused(&self) -> bool {
        self.ui.upgrade().is_some_and(|ui| {
            let state = ui.shared.lock();
            state
                .claim(self.entry.id)
                .is_some_and(|claim| claim.refused)
        })
    }

    /// Re-captures the participant into the session: call it whenever the
    /// state it saves has changed.
    ///
    /// Does nothing while a restored section waits for this participant's
    /// apply, or while its last apply panicked and none has succeeded since,
    /// or once the registration has ended or the participant is gone. Called
    /// from the participant's own method, it cannot capture now: the next
    /// [`SessionUi::tick`] does. A capture that panics here reaches the
    /// caller; one a tick makes is contained.
    ///
    /// # Examples
    ///
    /// ```
    #[doc = include_str!("../doc/setup.rs")]
    /// let session = Session::new(&ParamSetBuilder::new().build());
    /// let ui = session.take_ui().ok_or("the first UI handle")?;
    /// let zoom = Rc::new(RefCell::new(Zoom(1.0)));
    /// let registration = ui.register("zoom", &zoom)?;
    ///
    /// // A save writes the last capture, so tell the session when the state
    /// // moves.
    /// zoom.borrow_mut().0 = 3.0;
    /// assert_eq!(session.save(), Some(json!({ "zoom": 1.0 })));
    /// registration.changed();
    /// assert_eq!(session.save(), Some(json!({ "zoom": 3.0 })));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn changed(&self) {
        if let Some(ui) = self.ui.upgrade() {
            ui.capture(&self.entry);
        }
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.entry.status.set(Status::Ended);
        let Some(ui) = self.ui.upgrade() else {
            // The handle ended it already.
            return;
        };
        if let Ok(mut entries) = ui.entries.try_borrow_mut() {
            entries.retain(|entry| !Rc::ptr_eq(entry, &self.entry));
        }
        ui.shared.lock().end(self.entry.id);
    }
}
