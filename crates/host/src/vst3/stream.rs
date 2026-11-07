//! The saved block in and out of a host's `IBStream`.

use std::ffi::c_void;

use ::vst3::ComRef;
use ::vst3::Steinberg::{IBStream, IBStreamTrait, kResultOk};

/// The most bytes one read or write asks for.
const CHUNK: usize = 64 * 1024;

/// Reads the stream from where it is to its end, keeping what was read
/// when a read fails. `None` when the stream is null or holds more than
/// `limit` bytes, which are not read past.
///
/// # Safety
///
/// `stream` is null or an `IBStream` the host keeps alive until this call
/// returns.
pub(crate) unsafe fn read_all(stream: *mut IBStream, limit: usize) -> Option<Vec<u8>> {
    // SAFETY: the caller's contract.
    let stream = unsafe { ComRef::from_raw(stream) }?;
    let mut bytes = Vec::new();
    let mut chunk = vec![0u8; CHUNK];
    loop {
        let mut read = 0;
        // SAFETY: `chunk` has room for `CHUNK` bytes, and `read` for the
        // count.
        let result =
            unsafe { stream.read(chunk.as_mut_ptr().cast::<c_void>(), CHUNK as i32, &mut read) };
        let read = usize::try_from(read).unwrap_or(0).min(CHUNK);
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() > limit {
            return None;
        }
        // Some hosts answer the read that reaches the end with an error and
        // the bytes, and an empty stream with an error. Anything read is
        // kept, and none is an empty block, which loads nothing.
        if result != kResultOk || read == 0 {
            return Some(bytes);
        }
    }
}

/// Writes all of `bytes`. False when the stream is null or stops taking
/// them.
///
/// # Safety
///
/// `stream` is null or an `IBStream` the host keeps alive until this call
/// returns.
pub(crate) unsafe fn write_all(stream: *mut IBStream, mut bytes: &[u8]) -> bool {
    // SAFETY: the caller's contract.
    let Some(stream) = (unsafe { ComRef::from_raw(stream) }) else {
        return false;
    };
    while !bytes.is_empty() {
        let size = bytes.len().min(CHUNK);
        let mut written = 0;
        // SAFETY: `bytes` holds at least `size` bytes; the stream only reads
        // them, despite the pointer's type.
        let result = unsafe {
            stream.write(
                bytes.as_ptr().cast_mut().cast::<c_void>(),
                size as i32,
                &mut written,
            )
        };
        let written = usize::try_from(written).unwrap_or(0).min(size);
        if result != kResultOk || written == 0 {
            return false;
        }
        bytes = &bytes[written..];
    }
    true
}

#[cfg(test)]
mod tests {
    use ::vst3::Steinberg::{IBStream, int32, int64, kNotImplemented, kResultFalse, tresult};
    use ::vst3::{Class, ComWrapper};

    use super::*;

    /// A stream that answers its first read with every byte and an error,
    /// and takes no bytes, answering that it did.
    struct Awkward(Vec<u8>);

    impl Class for Awkward {
        type Interfaces = (IBStream,);
    }

    impl IBStreamTrait for Awkward {
        unsafe fn read(&self, buffer: *mut c_void, count: int32, read: *mut int32) -> tresult {
            let length = self.0.len().min(usize::try_from(count).unwrap_or(0));
            // SAFETY: the reader passes room for `count` bytes, and for the
            // count read.
            unsafe {
                std::ptr::copy_nonoverlapping(self.0.as_ptr(), buffer.cast::<u8>(), length);
                *read = length as int32;
            }
            kResultFalse
        }

        unsafe fn write(
            &self,
            _buffer: *mut c_void,
            _count: int32,
            written: *mut int32,
        ) -> tresult {
            // SAFETY: the writer passes room for the count written.
            unsafe { *written = 0 };
            kResultOk
        }

        unsafe fn seek(&self, _position: int64, _mode: int32, _result: *mut int64) -> tresult {
            kNotImplemented
        }

        unsafe fn tell(&self, _position: *mut int64) -> tresult {
            kNotImplemented
        }
    }

    fn pointer(stream: &ComWrapper<Awkward>) -> *mut IBStream {
        stream.as_com_ref::<IBStream>().expect("a stream").as_ptr()
    }

    #[test]
    fn a_read_ending_in_an_error_keeps_its_bytes() {
        let stream = ComWrapper::new(Awkward(b"block".to_vec()));
        // SAFETY: a live stream.
        let read = unsafe { read_all(pointer(&stream), 5) };
        assert_eq!(read.as_deref(), Some(&b"block"[..]));
        let empty = ComWrapper::new(Awkward(Vec::new()));
        // SAFETY: a live stream.
        let read = unsafe { read_all(pointer(&empty), 5) };
        assert_eq!(read, Some(Vec::new()), "an empty block");
        // SAFETY: null is allowed.
        assert_eq!(unsafe { read_all(std::ptr::null_mut(), 5) }, None);
        // SAFETY: a live stream.
        let read = unsafe { read_all(pointer(&stream), 4) };
        assert_eq!(read, None, "one byte past the limit");
    }

    /// A stream that never ends: every read fills the room it is given.
    struct Endless(std::cell::Cell<usize>);

    impl Class for Endless {
        type Interfaces = (IBStream,);
    }

    impl IBStreamTrait for Endless {
        unsafe fn read(&self, buffer: *mut c_void, count: int32, read: *mut int32) -> tresult {
            let length = usize::try_from(count).unwrap_or(0);
            // SAFETY: the reader passes room for `count` bytes, and for the
            // count read.
            unsafe {
                std::ptr::write_bytes(buffer.cast::<u8>(), b' ', length);
                *read = count;
            }
            self.0.set(self.0.get() + length);
            kResultOk
        }

        unsafe fn write(&self, _: *mut c_void, _: int32, _: *mut int32) -> tresult {
            kNotImplemented
        }

        unsafe fn seek(&self, _position: int64, _mode: int32, _result: *mut int64) -> tresult {
            kNotImplemented
        }

        unsafe fn tell(&self, _position: *mut int64) -> tresult {
            kNotImplemented
        }
    }

    #[test]
    fn an_endless_stream_is_read_no_further_than_one_chunk_past_the_limit() {
        let stream = ComWrapper::new(Endless(std::cell::Cell::new(0)));
        let pointer = stream.as_com_ref::<IBStream>().expect("a stream").as_ptr();
        // SAFETY: a live stream.
        assert_eq!(unsafe { read_all(pointer, 3 * CHUNK) }, None);
        assert_eq!(stream.0.get(), 4 * CHUNK);
    }

    #[test]
    fn a_stream_that_takes_nothing_fails_the_write() {
        let stream = ComWrapper::new(Awkward(Vec::new()));
        // SAFETY: a live stream.
        assert!(!unsafe { write_all(pointer(&stream), b"block") });
        // SAFETY: null is allowed.
        assert!(!unsafe { write_all(std::ptr::null_mut(), b"block") });
    }
}
