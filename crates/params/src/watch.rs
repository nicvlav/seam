//! What a surface shows: one watch per surface per parameter, polled on the
//! UI thread.

use crate::{ParamId, UiParams};

/// One surface's view of one parameter: a slider, a value box, the page's
/// copy of a value.
///
/// Each watch keeps its own position, so making, moving or dropping one
/// never changes what another shows.
///
/// [`poll`](Self::poll) it each UI tick. A poll costs one atomic load when
/// nothing changed, and however many writes landed between two polls, it
/// reports the latest once. It shows what the store holds, never what
/// some writer announced, so two writers racing leave it on the stored value.
///
/// While a gesture is open on the parameter, the watch shows what the gesture
/// shows ([`UiParams::shown`]); the writes the [`Origin`](crate::Origin) table
/// holds back wait in the store until the first poll after the close. A watch
/// made mid-gesture holds them back too.
///
/// It holds a clone of the [`UiParams`] it came from, and stays on the UI
/// thread.
///
/// # Examples
///
/// ```
/// # use seam_params as sp;
/// # let set = sp::ParamSet::from_decls([
/// #     sp::ParamDecl::new("gain", "Gain", sp::Kind::continuous(-60.0, 12.0, 0.1, None), 0.0),
/// # ])?;
/// # let gain = set.id("gain").expect("declared");
/// # let ui = set.take_ui().expect("the first UI handle");
///
/// let mut watch = ui.watch(gain);
///
/// set.audio().host_write(gain, 1.0); // automation
/// assert_eq!(watch.poll(), Some(12.0));
/// assert_eq!(watch.poll(), None); // nothing new
///
/// // While a drag is open the watch shows the drag, and automation waits.
/// let drag = ui.begin(gain);
/// drag.write(-6.0);
/// set.audio().host_write(gain, 0.5);
/// assert_eq!(watch.poll(), Some(-6.0));
/// drop(drag);
/// assert_eq!(watch.poll(), Some(set.value(gain)));
/// # Ok::<(), seam_params::DeclError>(())
/// ```
#[derive(Debug)]
pub struct Watch {
    ui: UiParams,
    id: ParamId,
    /// The change count the shown value accounts for. `None` until a poll
    /// with no gesture open reads the store, so a watch made mid-gesture
    /// still reads it after the close.
    seen: Option<u64>,
    value: f32,
}

impl Watch {
    /// Seeds from what surfaces show now: the store through `read`, or what
    /// an open gesture shows, with no count seen.
    pub(crate) fn new(ui: UiParams, id: ParamId) -> Self {
        let (seen, value) = match ui.shown(id) {
            Some(shown) => (None, shown),
            None => {
                let reading = ui.set().read(id);
                (Some(reading.changes), reading.value)
            }
        };
        Self {
            ui,
            id,
            seen,
            value,
        }
    }

    /// Returns the parameter it watches.
    pub fn id(&self) -> ParamId {
        self.id
    }

    /// Returns the value the surface shows, in real units: the seed, then each
    /// value a poll returned.
    pub fn value(&self) -> f32 {
        self.value
    }

    /// Returns the value to show now, if it differs from
    /// [`value`](Self::value), and shows it from then on. [`Watch`] shows it
    /// polled through automation and a drag.
    pub fn poll(&mut self) -> Option<f32> {
        let next = match self.ui.shown(self.id) {
            Some(shown) => shown,
            None => {
                if self.seen == Some(self.ui.set().changes(self.id)) {
                    return None;
                }
                // Through `read`, the one place the order of the two loads
                // lives.
                let reading = self.ui.set().read(self.id);
                self.seen = Some(reading.changes);
                reading.value
            }
        };
        if next.to_bits() == self.value.to_bits() {
            return None;
        }
        self.value = next;
        Some(next)
    }
}

impl UiParams {
    /// Creates a watch on the parameter for a new surface, seeded with what
    /// surfaces show now. [`Watch`] shows one polled.
    pub fn watch(&self, id: ParamId) -> Watch {
        Watch::new(self.clone(), id)
    }
}
