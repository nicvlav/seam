//! Strings into and out of the SDK's fixed-size buffers.

use std::ffi::c_char;

/// Copies `text` into a NUL-terminated 8-bit buffer, cut short at a whole
/// character to fit.
pub(crate) fn copy_utf8(text: &str, into: &mut [c_char]) {
    let Some(room) = into.len().checked_sub(1) else {
        return;
    };
    let end = text.floor_char_boundary(room);
    for (slot, &byte) in into.iter_mut().zip(&text.as_bytes()[..end]) {
        *slot = byte as c_char;
    }
    into[end] = 0;
}

/// Copies `text` into a NUL-terminated UTF-16 buffer, cut short at a whole
/// character to fit.
pub(crate) fn copy_utf16(text: &str, into: &mut [u16]) {
    let Some(room) = into.len().checked_sub(1) else {
        return;
    };
    let mut written = 0;
    for character in text.chars() {
        let mut units = [0u16; 2];
        let units = character.encode_utf16(&mut units);
        if written + units.len() > room {
            break;
        }
        into[written..written + units.len()].copy_from_slice(units);
        written += units.len();
    }
    into[written] = 0;
}

/// Reads a NUL-terminated UTF-16 string of at most `max` units, replacing
/// anything that is not UTF-16.
///
/// # Safety
///
/// `text` is null, or valid for reads up to its NUL or `max` units,
/// whichever comes first.
pub(crate) unsafe fn read_utf16(text: *const u16, max: usize) -> Option<String> {
    if text.is_null() {
        return None;
    }
    let mut length = 0;
    // SAFETY: the caller's contract: every unit up to the NUL or `max` is
    // readable, and the loop stops at either.
    while length < max && unsafe { *text.add(length) } != 0 {
        length += 1;
    }
    // SAFETY: the `length` units just read.
    let units = unsafe { std::slice::from_raw_parts(text, length) };
    Some(String::from_utf16_lossy(units))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_cut_at_a_whole_character_and_terminated() {
        let mut narrow = [1 as c_char; 5];
        copy_utf8("ab\u{e9}cd", &mut narrow);
        assert_eq!(narrow.map(|byte| byte as u8), *b"ab\xc3\xa9\0");
        copy_utf8("ab\u{e9}", &mut narrow[..4]);
        assert_eq!(
            narrow[..3]
                .iter()
                .map(|&byte| byte as u8)
                .collect::<Vec<_>>(),
            b"ab\0"
        );

        let mut wide = [1u16; 4];
        copy_utf16("a\u{1F3B5}b", &mut wide);
        assert_eq!(wide, [0x61, 0xD83C, 0xDFB5, 0]);
        copy_utf16("ab\u{1F3B5}", &mut wide);
        assert_eq!(wide[..3], [0x61, 0x62, 0], "no half a surrogate pair");

        // SAFETY: a live, terminated buffer.
        let read = unsafe { read_utf16(wide.as_ptr(), 128) };
        assert_eq!(read.as_deref(), Some("ab"));
        // SAFETY: null is allowed.
        assert_eq!(unsafe { read_utf16(std::ptr::null(), 128) }, None);
    }
}
