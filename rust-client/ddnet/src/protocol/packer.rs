//! Message packer/unpacker. C++: `src/engine/shared/packer.{h,cpp}`, plus the
//! UTF-8 and sanitize helpers it uses from `src/base/str.cpp`.

use super::varint;

/// C++: `CPacker::PACKER_BUFFER_SIZE` (packer.h:42).
pub const PACKER_BUFFER_SIZE: usize = 2048;

/// C++: `CPacker`. Errors are sticky: after the first failed add, every further
/// add is ignored, exactly like the C++ packer.
#[derive(Clone, Debug, Default)]
pub struct Packer {
    buf: Vec<u8>,
    error: bool,
}

impl Packer {
    pub fn reset(&mut self) {
        self.buf.clear();
        self.error = false;
    }

    /// C++: `CAbstractPacker::AddInt` (packer.cpp:23).
    pub fn add_int(&mut self, i: i32) {
        if self.error {
            return;
        }
        let mut tmp = [0u8; varint::MAX_BYTES_PACKED];
        let room = (PACKER_BUFFER_SIZE - self.buf.len()).min(tmp.len());
        match varint::pack(&mut tmp[..room], i) {
            Some(n) => self.buf.extend_from_slice(&tmp[..n]),
            None => self.error = true,
        }
    }

    /// C++: `CAbstractPacker::AddString` (packer.cpp:37). `s` is read like a C
    /// string: it ends at the first NUL. Invalid UTF-8 is re-encoded as U+FFFD.
    /// `limit <= 0` means the buffer size; C++ defaults are `limit = 0`,
    /// `allow_truncation = true`.
    pub fn add_string(&mut self, s: &[u8], mut limit: i32, allow_truncation: bool) {
        if self.error {
            return;
        }
        let prev_len = self.buf.len();
        if limit <= 0 {
            limit = PACKER_BUFFER_SIZE as i32;
        }
        let mut pos = 0;
        while byte_at(s, pos) != 0 {
            let mut codepoint = utf8_decode(s, &mut pos);
            if codepoint == -1 {
                codepoint = 0xFFFD; // Unicode replacement character
            }
            let (encoded, len) = utf8_encode(codepoint);
            // Limit must ensure space for null termination if desired.
            if limit < len as i32 {
                if allow_truncation {
                    break;
                }
                self.error = true;
                self.buf.truncate(prev_len);
                return;
            }
            // Ensure space for the null termination.
            if self.buf.len() + len + 1 > PACKER_BUFFER_SIZE {
                self.error = true;
                self.buf.truncate(prev_len);
                return;
            }
            self.buf.extend_from_slice(&encoded[..len]);
            limit -= len as i32;
        }
        self.buf.push(0);
    }

    /// C++: `CAbstractPacker::AddRaw` (packer.cpp:81).
    pub fn add_raw(&mut self, data: &[u8]) {
        if self.error {
            return;
        }
        if self.buf.len() + data.len() > PACKER_BUFFER_SIZE {
            self.error = true;
            return;
        }
        self.buf.extend_from_slice(data);
    }

    pub fn data(&self) -> &[u8] {
        &self.buf
    }

    pub fn error(&self) -> bool {
        self.error
    }
}

/// `GetString` flags. C++: `CUnpacker::SANITIZE` etc. (packer.h:62).
pub const SANITIZE: i32 = 1;
pub const SANITIZE_CC: i32 = 2;
pub const SKIP_START_WHITESPACES: i32 = 4;

/// C++: `CUnpacker`. Errors are sticky; after an error every getter returns its
/// zero value (`0`, `""`, `None`).
#[derive(Clone, Debug)]
pub struct Unpacker<'a> {
    data: &'a [u8],
    pos: usize,
    error: bool,
}

impl<'a> Unpacker<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            error: false,
        }
    }

    /// C++: `CUnpacker::GetInt` (packer.cpp:104).
    pub fn get_int(&mut self) -> i32 {
        if self.error {
            return 0;
        }
        match varint::unpack(&self.data[self.pos..]) {
            Some((value, n)) => {
                self.pos += n;
                value
            }
            None => {
                self.error = true;
                0
            }
        }
    }

    /// C++: `CUnpacker::GetIntOrDefault` (packer.cpp:126). Returns `default` only at
    /// the exact end of the data; in the error state it returns 0, like C++.
    pub fn get_int_or_default(&mut self, default: i32) -> i32 {
        if self.error {
            return 0;
        }
        if self.pos == self.data.len() {
            return default;
        }
        self.get_int()
    }

    /// C++: `CUnpacker::GetUncompressedInt` (packer.cpp:139). C++ copies a native
    /// `int`; all supported targets are little-endian.
    pub fn get_uncompressed_int(&mut self) -> i32 {
        if self.error {
            return 0;
        }
        match self.data.get(self.pos..self.pos + 4) {
            Some(bytes) => {
                self.pos += 4;
                i32::from_le_bytes(bytes.try_into().unwrap())
            }
            None => {
                self.error = true;
                0
            }
        }
    }

    /// C++: `CUnpacker::GetUncompressedIntOrDefault` (packer.cpp:156).
    pub fn get_uncompressed_int_or_default(&mut self, default: i32) -> i32 {
        if self.error {
            return 0;
        }
        if self.pos == self.data.len() {
            return default;
        }
        self.get_uncompressed_int()
    }

    /// C++: `CUnpacker::GetString` (packer.cpp:169); the C++ default for `flags` is
    /// `SANITIZE`. C++ sanitizes the receive buffer in place and returns a pointer
    /// into it; this returns a sanitized copy and leaves the input untouched.
    pub fn get_string(&mut self, flags: i32) -> String {
        if self.error {
            return String::new();
        }
        let rest = &self.data[self.pos..];
        let Some(nul) = rest.iter().position(|&b| b == 0) else {
            self.error = true;
            return String::new();
        };
        self.pos += nul + 1;
        let mut s = rest[..nul].to_vec();
        if !utf8_check(&s) {
            self.error = true;
            return String::new();
        }
        if flags & SANITIZE != 0 {
            sanitize(&mut s);
        } else if flags & SANITIZE_CC != 0 {
            sanitize_cc(&mut s);
        }
        if flags & SKIP_START_WHITESPACES != 0 {
            s.drain(..utf8_skip_whitespaces(&s));
        }
        // Valid UTF-8 was checked above; sanitizing only replaces ASCII bytes.
        String::from_utf8(s).expect("checked UTF-8")
    }

    /// C++: `CUnpacker::GetRaw` (packer.cpp:206).
    pub fn get_raw(&mut self, size: i32) -> Option<&'a [u8]> {
        if self.error {
            return None;
        }
        let end = usize::try_from(size).ok().map(|size| self.pos + size);
        match end.and_then(|end| self.data.get(self.pos..end)) {
            Some(raw) => {
                self.pos += raw.len();
                Some(raw)
            }
            None => {
                self.error = true;
                None
            }
        }
    }

    pub fn error(&self) -> bool {
        self.error
    }
}

/// Byte of a C string held in a slice: past the end reads as the NUL terminator.
fn byte_at(s: &[u8], pos: usize) -> u8 {
    s.get(pos).copied().unwrap_or(0)
}

/// C++: `str_utf8_decode` (str.cpp:971), WHATWG UTF-8 decoder. Returns the code
/// point, 0 at the terminator, or -1 on error. On an invalid continuation byte the
/// cursor is left on that byte.
pub(crate) fn utf8_decode(s: &[u8], pos: &mut usize) -> i32 {
    let mut lower: u8 = 0x80;
    let mut upper: u8 = 0xBF;
    let mut code_point: i32 = 0;
    let mut bytes_seen = 0;
    let mut bytes_needed = 0;
    loop {
        let byte = byte_at(s, *pos);
        *pos += 1;
        if bytes_needed == 0 {
            match byte {
                0x00..=0x7F => return byte as i32,
                0xC2..=0xDF => {
                    bytes_needed = 1;
                    code_point = byte as i32 - 0xC0;
                }
                0xE0..=0xEF => {
                    if byte == 0xE0 {
                        lower = 0xA0;
                    }
                    if byte == 0xED {
                        upper = 0x9F;
                    }
                    bytes_needed = 2;
                    code_point = byte as i32 - 0xE0;
                }
                0xF0..=0xF4 => {
                    if byte == 0xF0 {
                        lower = 0x90;
                    }
                    if byte == 0xF4 {
                        upper = 0x8F;
                    }
                    bytes_needed = 3;
                    code_point = byte as i32 - 0xF0;
                }
                _ => return -1,
            }
            code_point <<= 6 * bytes_needed;
            continue;
        }
        if !(lower <= byte && byte <= upper) {
            *pos -= 1;
            return -1;
        }
        lower = 0x80;
        upper = 0xBF;
        bytes_seen += 1;
        code_point += (byte as i32 - 0x80) << (6 * (bytes_needed - bytes_seen));
        if bytes_seen == bytes_needed {
            return code_point;
        }
    }
}

/// C++: `str_utf8_encode` (str.cpp:926). Returns the bytes and their count
/// (0 for code points above U+10FFFF).
pub(crate) fn utf8_encode(chr: i32) -> ([u8; 4], usize) {
    if chr <= 0x7F {
        ([chr as u8, 0, 0, 0], 1)
    } else if chr <= 0x7FF {
        (
            [
                0xC0 | ((chr >> 6) & 0x1F) as u8,
                0x80 | (chr & 0x3F) as u8,
                0,
                0,
            ],
            2,
        )
    } else if chr <= 0xFFFF {
        (
            [
                0xE0 | ((chr >> 12) & 0x0F) as u8,
                0x80 | ((chr >> 6) & 0x3F) as u8,
                0x80 | (chr & 0x3F) as u8,
                0,
            ],
            3,
        )
    } else if chr <= 0x10FFFF {
        (
            [
                0xF0 | ((chr >> 18) & 0x07) as u8,
                0x80 | ((chr >> 12) & 0x3F) as u8,
                0x80 | ((chr >> 6) & 0x3F) as u8,
                0x80 | (chr & 0x3F) as u8,
            ],
            4,
        )
    } else {
        ([0; 4], 0)
    }
}

/// C++: `str_utf8_check` (str.cpp:1241).
pub fn utf8_check(s: &[u8]) -> bool {
    let mut pos = 0;
    loop {
        match utf8_decode(s, &mut pos) {
            0 => return true,
            -1 => return false,
            _ => {}
        }
    }
}

/// C++: `str_sanitize` (str.cpp:171): control chars except `\r\n\t` become spaces.
pub(crate) fn sanitize(s: &mut [u8]) {
    for b in s.iter_mut() {
        if *b < 32 && *b != b'\r' && *b != b'\n' && *b != b'\t' {
            *b = b' ';
        }
    }
}

/// C++: `str_sanitize_cc` (str.cpp:159): all control chars become spaces.
pub(crate) fn sanitize_cc(s: &mut [u8]) {
    for b in s.iter_mut() {
        if *b < 32 {
            *b = b' ';
        }
    }
}

/// C++: `str_utf8_isspace` (str.cpp:1115).
pub(crate) fn utf8_isspace(code: i32) -> bool {
    code <= 0x0020
        || code == 0x0085
        || code == 0x00A0
        || code == 0x034F
        || code == 0x115F
        || code == 0x1160
        || code == 0x1680
        || code == 0x180E
        || (0x2000..=0x200F).contains(&code)
        || (0x2028..=0x202F).contains(&code)
        || (0x205F..=0x2064).contains(&code)
        || (0x206A..=0x206F).contains(&code)
        || code == 0x2800
        || code == 0x3000
        || code == 0x3164
        || (0xFE00..=0xFE0F).contains(&code)
        || code == 0xFEFF
        || code == 0xFFA0
        || (0xFFF9..=0xFFFC).contains(&code)
}

/// C++: `str_utf8_skip_whitespaces` (str.cpp:1211). Returns the byte offset of
/// the first non-whitespace code point (or the end).
pub(crate) fn utf8_skip_whitespaces(s: &[u8]) -> usize {
    let mut pos = 0;
    while byte_at(s, pos) != 0 {
        let old = pos;
        if !utf8_isspace(utf8_decode(s, &mut pos)) {
            return old;
        }
    }
    pos
}

/// C++: `str_utf8_fix_truncation` (str.cpp:1055), run by every `str_copy`: drops the
/// last code point if it does not decode (e.g. cut off mid-sequence).
pub(crate) fn utf8_fix_truncation(s: &mut Vec<u8>) {
    // C++ `str_utf8_rewind`: back up to the start of the last code point
    let mut last = s.len();
    while last > 0 {
        last -= 1;
        if s[last] & 0xC0 != 0x80 {
            break;
        }
    }
    let mut pos = last;
    if !s.is_empty() && utf8_decode(s, &mut pos) == -1 {
        s.truncate(last);
    }
}
