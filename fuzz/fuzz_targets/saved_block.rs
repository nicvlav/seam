//! The block a host hands back to load. A load never panics, and whatever it
//! accepts, once the participants have taken their sections on a tick, saves
//! a block that loads again and saves the same bytes.
#![no_main]

use libfuzzer_sys::fuzz_target;
use seam_fuzz::Fuzzed;

fuzz_target!(|data: &[u8]| {
    let (plugin, driver) = Fuzzed::built();
    let saved = std::sync::Arc::clone(plugin.saved());
    let _ = saved.load(data);
    driver.tick();
    let block = saved.save();
    assert!(
        saved.load(&block).is_ok(),
        "a block this plugin saved does not load"
    );
    driver.tick();
    assert_eq!(saved.save(), block, "a saved block is not a fixpoint");
});
