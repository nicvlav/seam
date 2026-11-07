//! The table the audio thread looks messages up in.

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

use std::sync::Arc;

use seam_params::ParamId;

use crate::parse::Message;
use crate::source::key;
use crate::target::Staging;
use crate::{Curve, Mapping};

/// Where a slot's value goes.
#[derive(Debug, Clone)]
pub(crate) enum Route {
    /// A parameter's store, as a learned write.
    Param(ParamId),
    /// A UI-tier target's staging slot, which the table owns while it can
    /// reach it, so a block in flight never writes one that is gone.
    Staged(Arc<Staging>),
}

/// One mapping, as the audio thread reads it.
#[derive(Debug, Clone)]
struct Slot {
    key: u64,
    route: Route,
    curve: Curve,
    coarse: bool,
}

/// Every mapping, sorted by kind, number, group and channel, "any" first.
/// Built on the UI thread and never changed after.
#[derive(Debug, Clone, Default)]
pub(crate) struct Table {
    slots: Box<[Slot]>,
}

impl Table {
    pub(crate) fn new<'a>(mappings: impl IntoIterator<Item = (Route, &'a Mapping)>) -> Self {
        let mut slots: Vec<Slot> = mappings
            .into_iter()
            .map(|(route, mapping)| Slot {
                key: mapping.source.key(),
                route,
                curve: mapping.curve,
                coarse: mapping.coarse,
            })
            .collect();
        // Stable, so slots with one key keep the order they came in.
        slots.sort_by_key(|slot| slot.key);
        Table {
            slots: slots.into_boxed_slice(),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.slots.len()
    }

    /// Hands every mapping `message` drives to `write`, with the target's
    /// 0..1: four binary searches, one per way a source can name the
    /// message's group and channel. Allocation-free.
    pub(crate) fn dispatch(&self, message: &Message, mut write: impl FnMut(&Route, f64)) {
        if self.slots.is_empty() {
            return;
        }
        let (group, channel) = (Some(message.group), Some(message.channel));
        for (group, channel) in [
            (group, channel),
            (group, None),
            (None, channel),
            (None, None),
        ] {
            let key = key(message.control, group, channel);
            let start = self.slots.partition_point(|slot| slot.key < key);
            for slot in self
                .slots
                .iter()
                .skip(start)
                .take_while(|slot| slot.key == key)
            {
                if let Some(value) = message.value.normalized(slot.coarse) {
                    write(&slot.route, slot.curve.apply(value));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use proptest::prelude::*;
    use proptest::test_runner::FileFailurePersistence;
    use seam_params::{Kind, ParamDecl, ParamSet, ParamSetBuilder};

    use super::*;
    use crate::parse::Value;
    use crate::{Control, Source};

    const TARGETS: usize = 24;

    fn set() -> Arc<ParamSet> {
        let mut builder = ParamSetBuilder::new();
        for index in 0..TARGETS {
            let key: &'static str = Box::leak(format!("p{index}").into_boxed_str());
            builder
                .add(ParamDecl::new(key, key, Kind::Bool, 0.0))
                .expect("valid");
        }
        builder.build()
    }

    /// Edge numbers, few enough that sources and messages meet often.
    fn control() -> impl Strategy<Value = Control> {
        let seven = prop::sample::select(vec![0_u8, 1, 64, 127]);
        let fourteen = prop::sample::select(vec![0_u16, 1, 127, 128, 8192, 16383]);
        prop_oneof![
            seven.clone().prop_map(Control::Cc),
            fourteen.prop_map(Control::Nrpn),
            seven.prop_map(Control::Note),
        ]
    }

    fn field() -> impl Strategy<Value = Option<u8>> {
        prop_oneof![
            Just(None),
            prop::sample::select(vec![0_u8, 1, 15]).prop_map(Some)
        ]
    }

    fn source() -> impl Strategy<Value = Source> {
        (control(), field(), field()).prop_map(|(control, group, channel)| Source {
            control,
            group,
            channel,
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
                "tests/table.proptest-regressions",
            ))),
            ..ProptestConfig::default()
        })]

        /// The four binary searches find exactly the targets a scan of
        /// every mapping finds, each once.
        #[test]
        fn the_lookup_finds_what_a_scan_finds(
            sources in proptest::collection::vec(source(), 0..TARGETS),
            control in control(),
            group in prop::sample::select(vec![0_u8, 1, 15]),
            channel in prop::sample::select(vec![0_u8, 1, 15]),
        ) {
            let set = set();
            let mappings: Vec<(ParamId, Mapping)> = set
                .ids()
                .zip(&sources)
                .map(|(id, &source)| (id, Mapping::new(source)))
                .collect();
            let table = Table::new(mappings.iter().map(|(id, mapping)| (Route::Param(*id), mapping)));
            let message = Message { control, group, channel, value: Value::Seven(127) };
            let mut found = Vec::new();
            table.dispatch(&message, |route, value| {
                assert_eq!(value, 1.0);
                let Route::Param(target) = route else {
                    panic!("only parameters here");
                };
                found.push(*target);
            });
            found.sort();
            let meets = |field: Option<u8>, wanted: u8| field.is_none_or(|field| field == wanted);
            let scanned: Vec<ParamId> = mappings
                .iter()
                .filter(|(_, mapping)| {
                    let source = mapping.source;
                    source.control == control
                        && meets(source.group, group)
                        && meets(source.channel, channel)
                })
                .map(|(id, _)| *id)
                .collect();
            prop_assert_eq!(found, scanned);
        }
    }
}
