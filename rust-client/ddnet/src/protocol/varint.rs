//! Variable-length int encoding. C++: `src/engine/shared/compression.{h,cpp}` (`CVariableInt`).
//!
//! Format: `ESDDDDDD EDDDDDDD ...` (Extend bit, Sign bit, Data). Negative values
//! are stored as `!i` with the sign bit set.

pub const MAX_BYTES_PACKED: usize = 5;

/// Packs `i` into `dst`, returning the number of bytes written, or `None` if
/// `dst` is too small. C++: `CVariableInt::Pack` (compression.h:18).
pub fn pack(dst: &mut [u8], mut i: i32) -> Option<usize> {
    if dst.is_empty() {
        return None;
    }
    let mut n = 0;
    dst[0] = 0;
    if i < 0 {
        dst[0] |= 0x40; // sign bit
        i = !i;
    }
    dst[0] |= (i & 0x3F) as u8;
    i >>= 6;
    while i != 0 {
        if n + 1 >= dst.len() {
            return None;
        }
        dst[n] |= 0x80; // extend bit
        n += 1;
        dst[n] = (i & 0x7F) as u8;
        i >>= 7;
    }
    Some(n + 1)
}

/// Unpacks one int from the start of `src`, returning it and the number of bytes
/// consumed, or `None` if `src` ends inside the int. C++: `CVariableInt::Unpack`
/// (compression.cpp:9). Like C++, at most 5 bytes are read: the extend bit of the
/// fifth byte is ignored and only its low 4 bits are used.
pub fn unpack(src: &[u8]) -> Option<(i32, usize)> {
    const MASKS: [i32; 4] = [0x7F, 0x7F, 0x7F, 0x0F];
    const SHIFTS: [u32; 4] = [6, 6 + 7, 6 + 7 + 7, 6 + 7 + 7 + 7];

    let first = *src.first()?;
    let sign = ((first >> 6) & 1) as i32;
    let mut value = (first & 0x3F) as i32;
    let mut idx = 0;
    for (mask, shift) in MASKS.iter().zip(SHIFTS) {
        if src[idx] & 0x80 == 0 {
            break;
        }
        idx += 1;
        value |= (*src.get(idx)? as i32 & mask) << shift;
    }
    value ^= -sign; // if sign: value = !value
    Some((value, idx + 1))
}

/// Unpacks varints until `src` is exhausted. C++: `CVariableInt::Decompress`
/// (compression.cpp:37); `None` if `src` ends inside an int or more than `max_ints`
/// ints would be produced.
pub fn decompress(src: &[u8], max_ints: usize) -> Option<Vec<i32>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < src.len() {
        if out.len() >= max_ints {
            return None;
        }
        let (value, n) = unpack(&src[pos..])?;
        out.push(value);
        pos += n;
    }
    Some(out)
}
