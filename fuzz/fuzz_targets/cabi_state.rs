//! The C interface's state calls, driven as the JUCE adapter drives them:
//! load whatever bytes a host holds, save, load that and save again. The
//! plugin keeps no fault, and a block it saved is a fixpoint.
#![no_main]

use libfuzzer_sys::fuzz_target;
use seam_fuzz::Fuzzed;
use seam_host::cabi::TestHost;

fuzz_target!(|data: &[u8]| {
    let host = TestHost::<Fuzzed>::new();
    let _ = host.load(data);
    host.idle();
    let block = host.save().expect("a block");
    assert_eq!(
        host.load(&block),
        Ok(()),
        "a block this plugin saved does not load"
    );
    host.idle();
    assert_eq!(host.save().as_deref(), Some(block.as_slice()));
    assert_eq!(host.faults(), 0);
});
