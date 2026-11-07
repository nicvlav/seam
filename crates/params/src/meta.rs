//! What a host's parameter list shows: step counts, kinds, and text in real
//! units.
//!
//! Real-unit text is formatted and parsed here. A host's 0..1 never has text
//! of its own: an exporter converts with [`Kind::denormalize`] first, so the
//! two can never be confused.

use crate::decl::shortest_decimal;
use crate::{Kind, ParamDecl, Range};

impl Kind {
    /// The number of values the kind lays out, 0 meaning continuous.
    ///
    /// A bool has 2 and a choice its labels. A stepped range counts its
    /// lattice points and an off-lattice end, saturating instead of
    /// overflowing. It counts points that share an `f32`, from a step finer
    /// than the `f32` spacing, so the count can pass the values it holds.
    ///
    /// Exporters use it only for the kinds that are
    /// [discrete](Self::is_discrete) (VST3's step count is this minus 1,
    /// CLAP's stepped flag). Open: the JUCE adapter should pass the count too,
    /// so AAX gets what a JUCE build gives it; the C interface sends 0 for
    /// every continuous range.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::Kind;
    /// assert_eq!(Kind::Bool.steps(), 2);
    /// assert_eq!(Kind::Choice(&["Peak", "Shelf", "Notch"]).steps(), 3);
    /// assert_eq!(Kind::continuous(0.0, 10.0, 1.0, None).steps(), 11); // 0, 1, ... 10
    /// assert_eq!(Kind::continuous(0.0, 10.0, 0.0, None).steps(), 0); // continuous
    /// ```
    pub fn steps(&self) -> u32 {
        match self {
            Kind::Continuous(range) => range_steps(range),
            Kind::Bool => 2,
            Kind::Choice(labels) => u32::try_from(labels.len()).unwrap_or(u32::MAX),
        }
    }

    /// Whether a host should treat the values as a fixed set: bool and
    /// choice only. A stepped range stays continuous to every host, as JUCE 9
    /// showed it to VST3 and AU.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::Kind;
    ///
    /// assert!(Kind::Bool.is_discrete());
    /// assert!(Kind::Choice(&["Peak", "Shelf"]).is_discrete());
    /// // Stepped, but still continuous to a host.
    /// assert!(!Kind::continuous(0.0, 10.0, 1.0, None).is_discrete());
    /// ```
    pub fn is_discrete(&self) -> bool {
        matches!(self, Kind::Bool | Kind::Choice(_))
    }

    /// Whether it is a bool.
    pub fn is_boolean(&self) -> bool {
        matches!(self, Kind::Bool)
    }
}

fn is_stepped(range: &Range) -> bool {
    range.interval() > 0.0
}

/// A stepped range's last lattice index, and whether the end sits past
/// that point, from the lattice snapping lays out.
fn lattice(range: &Range) -> (f64, bool) {
    range.lattice().map_or((0.0, false), |lattice| {
        (lattice.last, lattice.point(lattice.last) < range.end())
    })
}

fn range_steps(range: &Range) -> u32 {
    if !is_stepped(range) {
        return 0;
    }
    let (last, off_lattice_end) = lattice(range);
    // A float-to-integer `as` saturates, so a count past `u32::MAX` is
    // `u32::MAX`.
    (last + 1.0 + f64::from(u8::from(off_lattice_end))) as u32
}

/// The decimals a stepped range shows: as many as its start or its step
/// needs, at most 6.
///
/// The lattice counts in the finer of the two, so a whole step from 0.5 sits
/// on the halves, and the step's decimals alone would show 1.5 as 2.
fn stepped_decimals(range: &Range) -> usize {
    places(range.start()).max(places(range.interval()))
}

/// The places `value` was written with, as the lattice reads them, at most 6.
fn places(value: f32) -> usize {
    shortest_decimal(value)
        .1
        .map_or(6, |places| places.min(6) as usize)
}

/// `1.2500` to `1.25`, `3.00` to `3`.
fn trim_zeros(mut text: String) -> String {
    if text.contains('.') {
        let kept = text.trim_end_matches('0').trim_end_matches('.').len();
        text.truncate(kept);
    }
    text
}

/// `-0.00` to `0.00`: a value that rounds to zero shows no sign.
fn unsigned_zero(text: String) -> String {
    match text.strip_prefix('-') {
        Some(rest) if rest.chars().all(|c| c == '0' || c == '.') => rest.to_owned(),
        _ => text,
    }
}

impl ParamDecl {
    /// `value`, in real units, as a host or a value box shows it, without
    /// the unit.
    ///
    /// A continuous range shows 2 decimals. A stepped one shows as many as
    /// its start or its step needs, with trailing zeros trimmed. An
    /// off-lattice end shows the shortest text that reads back as itself, so a
    /// value box can still reach it. A bool shows On or Off and a choice its
    /// label.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{Kind, ParamDecl};
    ///
    /// let gain = ParamDecl::new("gain", "Gain", Kind::continuous(-60.0, 12.0, 0.1, None), 0.0);
    /// let mode = ParamDecl::new("mode", "Mode", Kind::Choice(&["Peak", "Shelf"]), 0.0);
    ///
    /// assert_eq!(gain.format_real(-6.0), "-6"); // the step's decimals, zeros trimmed
    /// assert_eq!(mode.format_real(1.0), "Shelf");
    /// ```
    pub fn format_real(&self, value: f32) -> String {
        match self.kind {
            Kind::Continuous(range) => {
                if !is_stepped(&range) {
                    return unsigned_zero(format!("{value:.2}"));
                }
                if value == range.end() && lattice(&range).1 {
                    return format!("{value}");
                }
                let decimals = stepped_decimals(&range);
                unsigned_zero(trim_zeros(format!("{value:.decimals$}")))
            }
            Kind::Bool => if value >= 0.5 { "On" } else { "Off" }.to_owned(),
            Kind::Choice(labels) => {
                let last = labels.len().saturating_sub(1) as f32;
                let index = value.round_ties_even().clamp(0.0, last) as usize;
                labels.get(index).copied().unwrap_or_default().to_owned()
            }
        }
    }

    /// Text a user typed, read as a value in real units, not snapped: the
    /// write that takes it snaps. The unit may follow the number.
    ///
    /// A bool reads `on`, `true` or `1`, in any case, as on, and anything
    /// else as off. A choice reads its label; an unknown label reads as
    /// `current`, so a renamed label never jumps to the first choice. A
    /// number that is not finite reads as nothing.
    ///
    /// # Examples
    ///
    /// ```
    /// use seam_params::{Kind, ParamDecl};
    ///
    /// let gain = ParamDecl::new("gain", "Gain", Kind::continuous(-60.0, 12.0, 0.1, None), 0.0)
    ///     .with_unit("dB");
    /// let mode = ParamDecl::new("mode", "Mode", Kind::Choice(&["Peak", "Shelf"]), 0.0);
    ///
    /// assert_eq!(gain.parse_real("-6.5 dB", 0.0), Some(-6.5)); // the unit may follow
    /// assert_eq!(gain.parse_real("loud", 0.0), None);
    /// assert_eq!(mode.parse_real("shelf", 0.0), Some(1.0));
    /// assert_eq!(mode.parse_real("unknown", 1.0), Some(1.0)); // keeps the current value
    /// ```
    pub fn parse_real(&self, text: &str, current: f32) -> Option<f32> {
        let text = text.trim();
        match self.kind {
            Kind::Continuous(_) => {
                let number = text
                    .strip_suffix(self.unit)
                    .filter(|_| !self.unit.is_empty())
                    .unwrap_or(text)
                    .trim();
                number.parse::<f32>().ok().filter(|value| value.is_finite())
            }
            Kind::Bool => {
                let on = ["on", "true", "1"]
                    .iter()
                    .any(|word| text.eq_ignore_ascii_case(word));
                Some(if on { 1.0 } else { 0.0 })
            }
            Kind::Choice(labels) => {
                let found = labels.iter().position(|label| *label == text).or_else(|| {
                    labels
                        .iter()
                        .position(|label| label.eq_ignore_ascii_case(text))
                });
                Some(found.map_or(current, |index| index as f32))
            }
        }
    }
}

/// `text` cut to a host's maximum length, in bytes, on a character
/// boundary. A maximum of 0 or less means unlimited: AU passes 0.
///
/// # Examples
///
/// ```
/// use seam_params::fit_text;
///
/// assert_eq!(fit_text("Low shelf gain", 9), "Low shelf");
/// assert_eq!(fit_text("Low shelf gain", 0), "Low shelf gain"); // AU passes 0: unlimited
/// assert_eq!(fit_text("caf\u{e9}", 4), "caf"); // never inside a character
/// ```
pub fn fit_text(text: &str, max_len: i32) -> &str {
    match usize::try_from(max_len) {
        Ok(max) if max > 0 => &text[..text.floor_char_boundary(max)],
        _ => text,
    }
}
