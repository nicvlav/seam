//! The audio-thread path of seam-host does not allocate: here, the
//! per-block event list a format exporter fills and a processor reads.

seam_testkit::install_alloc_gate!();

use seam_host::{Event, EventList};
use seam_testkit::alloc::assert_no_allocations;

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
