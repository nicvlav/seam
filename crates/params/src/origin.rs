//! Who wrote a parameter's value, and a reading that carries it on.

/// Who wrote a parameter's value. Every write names one, and it alone decides
/// the write's side effects.
///
/// | Origin | Thread | Host told | Gesture | Undo records | UI shows it |
/// | --- | --- | --- | --- | --- | --- |
/// | [`Host`](Self::Host), [`Restore`](Self::Restore), [`Learn`](Self::Learn) | any, audio included | never | never | never | on the next poll with no gesture open |
/// | [`User`](Self::User) | UI | begin, value, end | yes; nested and multi-control gestures collapse | one step per gesture | at once |
/// | [`Program`](Self::Program) | UI for an edit, any but audio for a sync | an edit as a gesture; a sync as a request to re-read | an edit joins the count | never, for a parameter | an edit at once; a sync on the next poll with no gesture open |
/// | [`Replay`](Self::Replay) | UI | as one complete gesture | its own | never | at once |
///
/// Every origin snaps the value to the parameter's range, and none stores a
/// value that is not finite: [`ParamDecl`](crate::ParamDecl)'s methods say
/// how each falls back. The writes that wait for a gesture to close may come
/// from another thread, and showing one mid-drag would yank the control from
/// under the user.
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
/// use seam_params::Origin;
///
/// // Every write names its origin, and the store keeps the newest.
/// set.audio().host_write(gain, 0.5);
/// assert_eq!(set.read(gain).origin, Origin::Host);
///
/// ui.write(gain, -6.0);
/// assert_eq!(set.read(gain).origin, Origin::User);
/// # Ok::<(), seam_params::DeclError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
#[non_exhaustive]
pub enum Origin {
    /// Host automation, through a plugin format.
    Host,
    /// A saved state being loaded.
    Restore,
    /// A learned MIDI controller, applied on the audio thread in the block it
    /// arrived in.
    Learn,
    /// A user's change.
    ///
    /// It is a parameter written through
    /// [`UiParams::write`](crate::UiParams::write) or a gesture from
    /// [`UiParams::begin`](crate::UiParams::begin). It is also a plain
    /// [`Value`](crate::Value) or [`List`](crate::List) changed through its
    /// own setters, which record as the user's whoever calls them.
    User,
    /// The plugin's own code writing a parameter: an [`Edit`](crate::Edit),
    /// which opens a gesture as a user's write does, or a
    /// [`sync`](crate::ProgramParams::sync), which opens none.
    Program,
    /// Undo or redo re-applying a step.
    Replay,
}

/// A byte that is no [`Origin`]'s number: the error of `Origin::try_from`.
///
/// Only that conversion makes one, so a later version may add to it.
///
/// # Examples
///
/// ```
/// use seam_params::{Origin, UnknownOrigin};
///
/// let error: UnknownOrigin = Origin::try_from(200_u8).unwrap_err();
/// assert_eq!(error.0, 200);
/// assert_eq!(error.to_string(), "200 is no origin's number");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct UnknownOrigin(pub u8);

impl std::fmt::Display for UnknownOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} is no origin's number", self.0)
    }
}

impl std::error::Error for UnknownOrigin {}

/// Makes `Origin::ALL` from one list of origins, beside a match on the same
/// list. The match is exhaustive, so an origin declared and not listed fails
/// to compile.
macro_rules! every_origin {
    ($($origin:ident),+ $(,)?) => {
        impl Origin {
            /// Every origin, in number order: `ALL[n]` is the origin whose
            /// number (`origin as u8`) is `n`, so a stored number reads back
            /// by indexing.
            pub(crate) const ALL: [Origin; [$(Origin::$origin),+].len()] = [$(Origin::$origin),+];
        }

        const _: fn(Origin) = |origin| match origin {
            $(Origin::$origin)|+ => {}
        };
    };
}

every_origin!(Host, Restore, Learn, User, Program, Replay);

// Each origin sits at its own number. With the match above, that makes the
// list exactly as long as the enum.
const _: () = {
    let mut number = 0;
    while number < Origin::ALL.len() {
        assert!(
            Origin::ALL[number] as usize == number,
            "Origin::ALL lists the origins in number order, each once"
        );
        number += 1;
    }
};

impl TryFrom<u8> for Origin {
    type Error = UnknownOrigin;

    /// The origin whose number (`origin as u8`) is `bits`.
    fn try_from(bits: u8) -> Result<Origin, UnknownOrigin> {
        Origin::ALL
            .get(usize::from(bits))
            .copied()
            .ok_or(UnknownOrigin(bits))
    }
}

/// A parameter, a [`Value`](crate::Value) or a [`List`](crate::List) as one
/// read found it: how many writes it had taken, its value, and who wrote
/// that value.
///
/// The value and its origin come from one load or one lock, and the value is
/// at least as new as the count.
///
/// Before any write the count is 0 and the origin [`Origin::Restore`]: the
/// initial value is the state it was made with. A value derived from the
/// source passes the origin on, as below. Derive from one reading: a second
/// read of the value may find a newer write than the origin names.
///
/// More fields may come, so a pattern that takes it apart ends in `..`.
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
/// use seam_params::{Origin, Reading, Value};
///
/// // A meter's scale follows the gain, and passes on who moved it.
/// let scale = Value::new(1.0_f32, &set);
/// set.audio().host_write(gain, 1.0);
///
/// let Reading { changes, value, origin, .. } = set.read(gain);
/// scale.set_from(10.0_f32.powf(value / 20.0), origin);
/// assert_eq!((changes, value, origin), (1, 12.0, Origin::Host));
/// assert_eq!(scale.read().origin, Origin::Host);
/// # Ok::<(), seam_params::DeclError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Reading<T> {
    /// How many writes had reached it when the count was read.
    pub changes: u64,
    /// The value it held.
    pub value: T,
    /// Who wrote that value.
    pub origin: Origin,
}

/// Whose change a [`Value`](crate::Value) or a [`List`](crate::List) takes,
/// which decides the origin it stores and whether undo records it.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Author {
    /// The user's or the plugin's own, through a plain setter or verb.
    Own,
    /// Derived from a change with this origin, or a replay.
    Derived(Origin),
}

impl Author {
    /// The origin the change stores, and whether undo records it. Only the
    /// user's or the plugin's own change records, as the user's. A change
    /// made while a restore applies on this thread (`applying`) is the
    /// restore's, and records nothing.
    pub(crate) fn resolve(self, applying: bool) -> (Origin, bool) {
        match self {
            _ if applying => (Origin::Restore, false),
            Author::Own => (Origin::User, true),
            Author::Derived(origin) => (origin, false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_origin_reads_back_from_its_number_and_no_other_byte_reads() {
        for origin in Origin::ALL {
            assert_eq!(Origin::try_from(origin as u8), Ok(origin));
        }
        // As many bytes read as the list holds, so it names no origin twice.
        let read = (0..=u8::MAX).filter(|&bits| Origin::try_from(bits).is_ok());
        assert_eq!(read.count(), Origin::ALL.len(), "one byte per origin");
    }
}
