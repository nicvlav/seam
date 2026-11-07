//! The `midiLearn` session section, `{"schema": 1, "mappings": [entry, ...]}`
//! as [`Learn::save`](crate::Learn::save) writes it. An entry:
//!
//! ```json
//! {"target": "gain", "kind": "nrpn", "group": -1, "channel": 0,
//!  "number": 300, "coarse": true,
//!  "curve": {"invert": false, "skew": 2.0, "low": 0.0, "high": 1.0}}
//! ```
//!
//! `kind` is `cc`, `nrpn` or `note`. `group` and `channel` are 0 to 15, or
//! -1 for any. `coarse` appears only on an NRPN whose capture saw no
//! low byte. `curve` is left out when it is the identity.

use seam_session::Section;
use serde_json::{Map, Value as Json};

use crate::{Control, Curve, Mapping, Source};

/// The key the section is saved under.
pub const SESSION_KEY: &str = "midiLearn";

/// The schema this build writes and reads.
pub(crate) const SCHEMA: u64 = 1;

/// One mapping as its saved entry.
pub(crate) fn encode(key: &str, mapping: &Mapping) -> Section {
    let source = mapping.source;
    let (kind, number) = match source.control {
        Control::Cc(number) => ("cc", u64::from(number)),
        Control::Nrpn(number) => ("nrpn", u64::from(number)),
        Control::Note(number) => ("note", u64::from(number)),
    };
    let any = |field: Option<u8>| field.map_or(-1, i64::from);
    // Keys inserted in order, so the bytes do not depend on serde_json's
    // `preserve_order`.
    let mut entry = Map::new();
    entry.insert("channel".into(), any(source.channel).into());
    if matches!(source.control, Control::Nrpn(_)) && mapping.coarse {
        entry.insert("coarse".into(), true.into());
    }
    let curve = mapping.curve;
    if !curve.is_identity() {
        let mut saved = Map::new();
        saved.insert("high".into(), f64::from(curve.high()).into());
        saved.insert("invert".into(), curve.is_inverted().into());
        saved.insert("low".into(), f64::from(curve.low()).into());
        saved.insert("skew".into(), f64::from(curve.skew()).into());
        entry.insert("curve".into(), Json::Object(saved));
    }
    entry.insert("group".into(), any(source.group).into());
    entry.insert("kind".into(), kind.into());
    entry.insert("number".into(), number.into());
    entry.insert("target".into(), key.into());
    Json::Object(entry)
}

/// The entry's curve: the identity when it has none, `None` when it is
/// unreadable.
fn decode_curve(object: &Map<String, Json>) -> Option<Curve> {
    let Some(saved) = object.get("curve") else {
        return Some(Curve::IDENTITY);
    };
    let saved = saved.as_object()?;
    let number = |name: &str, default: f64| match saved.get(name) {
        None => Some(default),
        Some(value) => value.as_f64(),
    };
    let invert = match saved.get("invert") {
        None => false,
        Some(value) => value.as_bool()?,
    };
    let curve = Curve::new(
        number("skew", 1.0)? as f32,
        number("low", 0.0)? as f32,
        number("high", 1.0)? as f32,
    )
    .ok()?;
    Some(if invert { curve.inverted() } else { curve })
}

/// A saved entry's target key and mapping, or `None` for an entry this
/// build cannot read.
///
/// An entry is unreadable with the wrong shape, an unknown kind, or a number,
/// group, channel or curve out of range. Whether the mapping may exist (a
/// parser-owned controller, a reserved NRPN, the ceiling) is the restore's
/// check, as for any mapping.
pub(crate) fn decode(entry: &Section) -> Option<(String, Mapping)> {
    let object = entry.as_object()?;
    let key = object.get("target")?.as_str()?;
    if key.is_empty() {
        return None;
    }
    let number = object.get("number")?.as_u64()?;
    let control = match object.get("kind")?.as_str()? {
        "cc" => Control::Cc(u8::try_from(number).ok()?),
        "nrpn" => Control::Nrpn(u16::try_from(number).ok()?),
        "note" => Control::Note(u8::try_from(number).ok()?),
        _ => return None,
    };
    let field = |name: &str| -> Option<Option<u8>> {
        match object.get(name) {
            None => Some(None),
            Some(value) => match value.as_i64()? {
                -1 => Some(None),
                value => u8::try_from(value).ok().map(Some),
            },
        }
    };
    let source = Source {
        control,
        group: field("group")?,
        channel: field("channel")?,
    };
    if !source.is_valid() {
        return None;
    }
    let curve = decode_curve(object)?;
    let coarse = match object.get("coarse") {
        None => false,
        Some(value) => value.as_bool()?,
    };
    Some((
        key.to_owned(),
        Mapping {
            source,
            curve,
            coarse: coarse && matches!(control, Control::Nrpn(_)),
        },
    ))
}
