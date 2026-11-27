//! Ogg Opus decoding: C++ `CSound::DecodeOpus` (engine/client/sound.cpp:464) on the real
//! opusfile 0.12 / libopus 1.3.1 / libogg 1.3.5, vendored in `rust-client/vendor` and built
//! by `build.rs` (no pure-Rust decoder is bit-exact with libopus: DECISIONS.md 13.3).
//!
//! This module is the only place that calls opusfile; every `unsafe` block is an FFI call
//! with its SAFETY reasoning, and the API is safe.

use crate::wavpack::Decoded;
use std::ffi::{c_char, c_int, c_uchar};

/// opusfile's `OggOpusFile`, opaque.
#[repr(C)]
struct OggOpusFile {
    _private: [u8; 0],
}

/// opusfile's `OpusTags`.
#[repr(C)]
struct OpusTags {
    user_comments: *mut *mut c_char,
    comment_lengths: *mut c_int,
    comments: c_int,
    vendor: *mut c_char,
}

unsafe extern "C" {
    fn op_open_memory(data: *const c_uchar, size: usize, error: *mut c_int) -> *mut OggOpusFile;
    fn op_channel_count(of: *const OggOpusFile, li: c_int) -> c_int;
    fn op_pcm_total(of: *const OggOpusFile, li: c_int) -> i64;
    fn op_read(of: *mut OggOpusFile, pcm: *mut i16, buf_size: c_int, li: *mut c_int) -> c_int;
    fn op_tags(of: *const OggOpusFile, li: c_int) -> *const OpusTags;
    fn op_free(of: *mut OggOpusFile);
}

/// An open file; freed on drop. It borrows the data, which opusfile reads in place.
struct File<'a> {
    of: *mut OggOpusFile,
    _data: &'a [u8],
}

impl Drop for File<'_> {
    fn drop(&mut self) {
        // SAFETY: `of` came from `op_open_memory`, is non-null and freed only here.
        unsafe { op_free(self.of) }
    }
}

impl<'a> File<'a> {
    fn open(data: &'a [u8]) -> Option<Self> {
        let mut error = 0;
        // SAFETY: the pointer and length describe `data`, which outlives the file (the
        // borrow in `File`); `error` is a valid out pointer.
        let of = unsafe { op_open_memory(data.as_ptr(), data.len(), &mut error) };
        (!of.is_null()).then_some(File { of, _data: data })
    }

    fn channel_count(&self) -> i32 {
        // SAFETY: `of` is an open file.
        unsafe { op_channel_count(self.of, -1) }
    }

    fn pcm_total(&self) -> i64 {
        // SAFETY: `of` is an open file.
        unsafe { op_pcm_total(self.of, -1) }
    }

    /// `op_read` into `buf`: the sample frames read, or a negative error.
    fn read(&mut self, buf: &mut [i16]) -> i32 {
        let len = buf.len().min(c_int::MAX as usize) as c_int;
        // SAFETY: `of` is an open file; opusfile writes at most `len` values into `buf`,
        // which holds at least that many; a null link index pointer is allowed.
        unsafe { op_read(self.of, buf.as_mut_ptr(), len, std::ptr::null_mut()) }
    }

    /// The comments of the tags, as bytes.
    fn comments(&self) -> Vec<Vec<u8>> {
        // SAFETY: `of` is an open file; the tags it returns live as long as it does.
        let tags = unsafe { op_tags(self.of, -1) };
        if tags.is_null() {
            return Vec::new();
        }
        // SAFETY: `tags` points to the file's `OpusTags`, whose `user_comments` and
        // `comment_lengths` arrays hold `comments` entries.
        let t = unsafe { &*tags };
        (0..t.comments.max(0) as usize)
            .filter_map(|i| {
                // SAFETY: `i < comments`, see above.
                let (p, n) = unsafe { (*t.user_comments.add(i), *t.comment_lengths.add(i)) };
                // SAFETY: a non-null comment has `n` bytes (opusfile stores the length).
                (!p.is_null()).then(|| unsafe {
                    std::slice::from_raw_parts(p as *const u8, n.max(0) as usize).to_vec()
                })
            })
            .collect()
    }
}

/// DDNet's `str_toint` (base/str.cpp:874): `strtol` (whitespace, sign, digits, 32-bit `long`
/// saturating) with nothing after the number; an empty string is 0.
fn str_toint(s: &[u8]) -> Option<i32> {
    let t = s.trim_ascii_start();
    let (neg, rest) = match t.first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let n = rest.iter().take_while(|c| c.is_ascii_digit()).count();
    if n == 0 {
        // no conversion: strtol's end is the start of the string
        return s.is_empty().then_some(0);
    }
    if n != rest.len() {
        return None;
    }
    let v = rest
        .iter()
        .fold(0i64, |v, &c| (v * 10 + (c - b'0') as i64).min(1 << 40));
    let v = if neg { -v } else { v };
    Some(v.clamp(i32::MIN as i64, i32::MAX as i64) as i32)
}

/// C++ `CSound::DecodeOpus`: `None` where C++ logs an error and returns false.
pub fn decode(data: &[u8]) -> Option<Decoded> {
    let mut file = File::open(data)?;
    let channels = file.channel_count();
    if channels > 2 {
        return None;
    }
    // C++ stores the int64 total in an int
    let num_samples = file.pcm_total() as i32;
    if num_samples < 0 || channels < 1 {
        return None;
    }
    let ch = channels as usize;
    let mut pcm = Vec::new();
    pcm.try_reserve_exact(num_samples as usize * ch).ok()?;
    pcm.resize(num_samples as usize * ch, 0i16);
    let mut pos = 0usize;
    while pos < num_samples as usize {
        let read = file.read(&mut pcm[pos * ch..]);
        if read < 0 {
            return None;
        }
        if read == 0 {
            break; // EOF
        }
        pos += read as usize;
    }
    pcm.truncate(pos * ch);
    let mut loop_start = 0;
    let comments = file.comments();
    if let Some(c) = comments.iter().find(|c| c.starts_with(b"LOOP_START=")) {
        // C++: the first LOOP_START tag decides; a bad or out of range value keeps 0
        if let Some(v) = str_toint(&c[b"LOOP_START=".len()..])
            && v >= 0
            && (v as usize) < pos
        {
            loop_start = v as usize;
        }
    }
    Some(Decoded {
        data: pcm,
        frames: pos,
        rate: 48000,
        channels: ch,
        loop_start,
    })
}

#[cfg(test)]
mod tests {
    use super::str_toint;

    #[test]
    fn str_toint_like_strtol() {
        assert_eq!(str_toint(b"123"), Some(123));
        assert_eq!(str_toint(b" -7"), Some(-7));
        assert_eq!(str_toint(b"+5"), Some(5));
        assert_eq!(str_toint(b""), Some(0));
        assert_eq!(str_toint(b" "), None);
        assert_eq!(str_toint(b"12x"), None);
        assert_eq!(str_toint(b"99999999999"), Some(i32::MAX));
    }
}
