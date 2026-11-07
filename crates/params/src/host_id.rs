//! Host ids: what VST3, CLAP and AU hosts identify a parameter by, and
//! save automation against.

use std::ops::RangeInclusive;

/// The host ids no parameter may take: the VST3 exporter's own proxy
/// parameters, the grid of 16 channels by 130 controllers and the MIDI 2.0
/// pool. The builder refuses a key whose id falls here.
pub const RESERVED_HOST_IDS: RangeInclusive<u32> = 0x7FFF_0000..=0x7FFF_FFFF;

/// The host id of `key`: 32-bit FNV-1a over its UTF-8 bytes, with the top
/// bit cleared.
///
/// That fits VST3's 31 bits and is the same number in VST3 and CLAP. Through
/// the JUCE adapter, AU and AAX derive their ids from the key as JUCE does.
/// Fixed forever: a host's saved automation names parameters by it, so
/// changing it orphans every project that automates the plugin.
///
/// # Examples
///
/// ```
/// use seam_params::{RESERVED_HOST_IDS, host_id};
///
/// assert_eq!(host_id(""), 0x011C_9DC5);
/// assert_eq!(host_id("gain"), host_id("gain"));
/// assert!(!RESERVED_HOST_IDS.contains(&host_id("gain")));
/// ```
pub const fn host_id(key: &str) -> u32 {
    const OFFSET_BASIS: u32 = 0x811C_9DC5;
    const PRIME: u32 = 0x0100_0193;
    let bytes = key.as_bytes();
    let mut hash = OFFSET_BASIS;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u32;
        hash = hash.wrapping_mul(PRIME);
        index += 1;
    }
    hash & 0x7FFF_FFFF
}
