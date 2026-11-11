//! What seam-host runs over and over does not allocate: the per-block event
//! list a format exporter fills and a processor reads, and a plugin's idle
//! tick when nothing changed, which every instance runs about 60 times a
//! second.

seam_testkit::install_alloc_gate!();

use seam_host::{
    AuCodes, Block, Category, Event, EventList, Layout, Parts, Plugin, PluginInfo, ProcessSetup,
    Processor, Status,
};
use seam_learn::Config;
use seam_params::{Declarer, Float, FloatDecl, Params};
use seam_testkit::alloc::assert_no_allocations;
use seam_undo::Bound;

#[test]
fn filling_and_reading_a_block_of_events_does_not_allocate() {
    let mut events = EventList::with_capacity(512);
    assert_no_allocations("an event list over many blocks, overflow included", || {
        for block in 0..64u32 {
            events.clear();
            for offset in 0..600u32 {
                events.push(Event {
                    offset,
                    words: [0x2090_3C00 | (block & 0x7F), 0, 0, 0],
                });
            }
            let sum: u32 = events.as_slice().iter().map(|event| event.offset).sum();
            std::hint::black_box((sum, events.dropped()));
        }
    });
}

struct Gain {
    _gain: Float,
}

impl Params for Gain {
    fn declare(p: &mut Declarer) -> Self {
        Gain {
            _gain: p.add(FloatDecl::new("gain", "Gain", 0.0..=1.0, 0.5)),
        }
    }
}

struct Silent;

impl Processor for Silent {
    fn process(&mut self, _: Block<'_, '_>) -> Status {
        Status::Sleep
    }
}

/// A plugin that takes MIDI, for parts with learn on; its parts are used
/// with no plugin built.
struct Listening;

impl Plugin for Listening {
    const INFO: PluginInfo = PluginInfo {
        name: "Listening",
        vendor: "Seam",
        url: "",
        email: "",
        description: "",
        version: "0.1.0",
        id: "dev.seam.listening",
        vst3_class_id: [0; 16],
        au_codes: AuCodes {
            subtype: *b"lstn",
            manufacturer: *b"Seam",
        },
        category: Category::Effect,
        features: &[],
        layouts: &[Layout::STEREO],
        midi_input: true,
        midi_output: false,
    };
    type Params = Gain;
    type Processor = Silent;

    fn new(_: Gain, _: Parts) -> Self {
        Listening
    }

    fn activate(&mut self, _: &ProcessSetup) -> Silent {
        Silent
    }
}

#[test]
fn an_idle_tick_of_a_plugin_s_parts_does_not_allocate() {
    let (_params, mut parts) = Parts::for_plugin::<Listening>().expect("valid declarations");
    let _history = parts.undo(Bound::default()).expect("no other history");
    parts
        .midi_learn(Config::default())
        .expect("learn's key is free");
    let _audio = parts.learn_audio();
    parts.tick();
    assert_no_allocations("idle ticks of parts with undo and learn on", || {
        for _ in 0..64 {
            std::hint::black_box(parts.tick());
        }
    });
}
