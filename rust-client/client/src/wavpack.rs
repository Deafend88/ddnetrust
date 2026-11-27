//! WavPack decoder: the part of libwavpack 5.9.0 (ddnet-libs) that C++ `CSound::DecodeWV`
//! (engine/client/sound.cpp:603) reaches, i.e. `WavpackOpenFileInputEx` + one
//! `WavpackUnpackSamples` of the whole file for single-stream mono/stereo files: lossless
//! and hybrid lossy (no correction file), every decorrelation term, joint and false
//! stereo, shifts and the extended-integer fixups. Errors behave like the library's
//! (muted blocks, CRC failures, truncated data, skipped junk, lengths from the file end).
//! What is left out returns `None` (DECISIONS.md 13.3): multi-stream layouts, float, DSD
//! and extended 32-bit (wvx) data, and lengths that cannot be allocated.
//!
//! Function names follow the C sources (open_utils.c, unpack_utils.c, unpack.c,
//! read_words.c, entropy_utils.c, decorr_utils.c).

/// A decoded sample, as C++ `DecodeWV` fills `CSample`.
#[derive(Debug, Clone, PartialEq)]
pub struct Decoded {
    /// Interleaved 16-bit samples, `frames * channels` long.
    pub data: Vec<i16>,
    pub frames: usize,
    pub rate: u32,
    pub channels: usize,
    /// C++ `m_LoopStart`: 0, or an Opus file's `LOOP_START` tag
    pub loop_start: usize,
}

const BYTES_STORED: u32 = 3;
const MONO_FLAG: u32 = 4;
const HYBRID_FLAG: u32 = 8;
const JOINT_STEREO: u32 = 0x10;
const FLOAT_DATA: u32 = 0x80;
const INT32_DATA: u32 = 0x100;
const HYBRID_BITRATE: u32 = 0x200;
const HYBRID_BALANCE: u32 = 0x400;
const INITIAL_BLOCK: u32 = 0x800;
const FINAL_BLOCK: u32 = 0x1000;
const SHIFT_LSB: u32 = 13;
const SHIFT_MASK: u32 = 0x1f << SHIFT_LSB;
const MAG_LSB: u32 = 18;
const MAG_MASK: u32 = 0x1f << MAG_LSB;
const SRATE_LSB: u32 = 23;
const SRATE_MASK: u32 = 0xf << SRATE_LSB;
const FALSE_STEREO: u32 = 0x4000_0000;
const DSD_FLAG: u32 = 0x8000_0000;
const MONO_DATA: u32 = MONO_FLAG | FALSE_STEREO;

const ID_UNIQUE: u8 = 0x3f;
const ID_OPTIONAL_DATA: u8 = 0x20;
const ID_ODD_SIZE: u8 = 0x40;
const ID_LARGE: u8 = 0x80;
const ID_BLOCK_CHECKSUM: u8 = ID_OPTIONAL_DATA | 0xf;

const MAX_NTERMS: usize = 16;
const MAX_TERM: i32 = 8;
const SLS: u32 = 8;
const SLO: u32 = 1 << (SLS - 1);
const LIMIT_ONES: u32 = 16;
const WORD_EOF: i32 = i32::MIN;
const SAMPLE_RATES: [u32; 15] = [
    6000, 8000, 9600, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000, 64000, 88200, 96000,
    192000,
];

#[rustfmt::skip]
const LOG2_TABLE: [u8; 256] = [
    0x00, 0x01, 0x03, 0x04, 0x06, 0x07, 0x09, 0x0a, 0x0b, 0x0d, 0x0e, 0x10, 0x11, 0x12, 0x14, 0x15,
    0x16, 0x18, 0x19, 0x1a, 0x1c, 0x1d, 0x1e, 0x20, 0x21, 0x22, 0x24, 0x25, 0x26, 0x28, 0x29, 0x2a,
    0x2c, 0x2d, 0x2e, 0x2f, 0x31, 0x32, 0x33, 0x34, 0x36, 0x37, 0x38, 0x39, 0x3b, 0x3c, 0x3d, 0x3e,
    0x3f, 0x41, 0x42, 0x43, 0x44, 0x45, 0x47, 0x48, 0x49, 0x4a, 0x4b, 0x4d, 0x4e, 0x4f, 0x50, 0x51,
    0x52, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x5c, 0x5d, 0x5e, 0x5f, 0x60, 0x61, 0x62, 0x63,
    0x64, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x6b, 0x6c, 0x6d, 0x6e, 0x6f, 0x70, 0x71, 0x72, 0x74, 0x75,
    0x76, 0x77, 0x78, 0x79, 0x7a, 0x7b, 0x7c, 0x7d, 0x7e, 0x7f, 0x80, 0x81, 0x82, 0x83, 0x84, 0x85,
    0x86, 0x87, 0x88, 0x89, 0x8a, 0x8b, 0x8c, 0x8d, 0x8e, 0x8f, 0x90, 0x91, 0x92, 0x93, 0x94, 0x95,
    0x96, 0x97, 0x98, 0x99, 0x9a, 0x9b, 0x9b, 0x9c, 0x9d, 0x9e, 0x9f, 0xa0, 0xa1, 0xa2, 0xa3, 0xa4,
    0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xa9, 0xaa, 0xab, 0xac, 0xad, 0xae, 0xaf, 0xb0, 0xb1, 0xb2, 0xb2,
    0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xb9, 0xba, 0xbb, 0xbc, 0xbd, 0xbe, 0xbf, 0xc0, 0xc0,
    0xc1, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xcb, 0xcb, 0xcc, 0xcd, 0xce,
    0xcf, 0xd0, 0xd0, 0xd1, 0xd2, 0xd3, 0xd4, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd8, 0xd9, 0xda, 0xdb,
    0xdc, 0xdc, 0xdd, 0xde, 0xdf, 0xe0, 0xe0, 0xe1, 0xe2, 0xe3, 0xe4, 0xe4, 0xe5, 0xe6, 0xe7, 0xe7,
    0xe8, 0xe9, 0xea, 0xea, 0xeb, 0xec, 0xed, 0xee, 0xee, 0xef, 0xf0, 0xf1, 0xf1, 0xf2, 0xf3, 0xf4,
    0xf4, 0xf5, 0xf6, 0xf7, 0xf7, 0xf8, 0xf9, 0xf9, 0xfa, 0xfb, 0xfc, 0xfc, 0xfd, 0xfe, 0xff, 0xff,
];

#[rustfmt::skip]
const EXP2_TABLE: [u8; 256] = [
    0x00, 0x01, 0x01, 0x02, 0x03, 0x03, 0x04, 0x05, 0x06, 0x06, 0x07, 0x08, 0x08, 0x09, 0x0a, 0x0b,
    0x0b, 0x0c, 0x0d, 0x0e, 0x0e, 0x0f, 0x10, 0x10, 0x11, 0x12, 0x13, 0x13, 0x14, 0x15, 0x16, 0x16,
    0x17, 0x18, 0x19, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1d, 0x1e, 0x1f, 0x20, 0x20, 0x21, 0x22, 0x23,
    0x24, 0x24, 0x25, 0x26, 0x27, 0x28, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2c, 0x2d, 0x2e, 0x2f, 0x30,
    0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x3a, 0x3b, 0x3c, 0x3d,
    0x3e, 0x3f, 0x40, 0x41, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x48, 0x49, 0x4a, 0x4b,
    0x4c, 0x4d, 0x4e, 0x4f, 0x50, 0x51, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a,
    0x5b, 0x5c, 0x5d, 0x5e, 0x5e, 0x5f, 0x60, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
    0x6a, 0x6b, 0x6c, 0x6d, 0x6e, 0x6f, 0x70, 0x71, 0x72, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79,
    0x7a, 0x7b, 0x7c, 0x7d, 0x7e, 0x7f, 0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x87, 0x88, 0x89, 0x8a,
    0x8b, 0x8c, 0x8d, 0x8e, 0x8f, 0x90, 0x91, 0x92, 0x93, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0x9b,
    0x9c, 0x9d, 0x9f, 0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa8, 0xa9, 0xaa, 0xab, 0xac, 0xad,
    0xaf, 0xb0, 0xb1, 0xb2, 0xb3, 0xb4, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xbc, 0xbd, 0xbe, 0xbf, 0xc0,
    0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc8, 0xc9, 0xca, 0xcb, 0xcd, 0xce, 0xcf, 0xd0, 0xd2, 0xd3, 0xd4,
    0xd6, 0xd7, 0xd8, 0xd9, 0xdb, 0xdc, 0xdd, 0xde, 0xe0, 0xe1, 0xe2, 0xe4, 0xe5, 0xe6, 0xe8, 0xe9,
    0xea, 0xec, 0xed, 0xee, 0xf0, 0xf1, 0xf2, 0xf4, 0xf5, 0xf6, 0xf8, 0xf9, 0xfa, 0xfc, 0xfd, 0xff,
];

fn count_bits(v: u32) -> u32 {
    32 - v.leading_zeros()
}

/// C: entropy_utils.c `wp_log2`.
fn wp_log2(avalue: u32) -> i32 {
    let avalue = avalue.wrapping_add(avalue >> 9);
    let dbits = count_bits(avalue);
    let frac = if dbits <= 9 {
        avalue << (9 - dbits)
    } else {
        avalue >> (dbits - 9)
    };
    ((dbits << 8) + LOG2_TABLE[(frac & 0xff) as usize] as u32) as i32
}

/// C: entropy_utils.c `wp_exp2s`.
fn wp_exp2s(log: i32) -> i32 {
    if log < 0 {
        return !((wp_exp2s(log.wrapping_neg()) as u32).wrapping_sub(1)) as i32;
    }
    let value = EXP2_TABLE[(log & 0xff) as usize] as u32 | 0x100;
    let log = log >> 8;
    if log <= 9 {
        (value >> (9 - log)) as i32
    } else {
        (value << ((log - 9) & 0x1f)) as i32
    }
}

/// C: entropy_utils.c `restore_weight`.
fn restore_weight(weight: i8) -> i32 {
    let mut result = weight as i32 * 8;
    if result > 0 {
        result += (result + 64) >> 7;
    }
    result
}

fn le16s(b: &[u8]) -> i32 {
    i16::from_le_bytes([b[0], b[1]]) as i32
}

/// C: wavpack_local.h `apply_weight` (the `apply_weight_i`/`apply_weight_f` pair).
fn apply_weight(weight: i32, sample: i32) -> i32 {
    if sample != sample as i16 as i32 {
        ((((sample & 0xffff).wrapping_mul(weight)) >> 9)
            .wrapping_add(((sample & !0xffff) >> 9).wrapping_mul(weight))
            .wrapping_add(1))
            >> 1
    } else {
        weight.wrapping_mul(sample).wrapping_add(512) >> 10
    }
}

/// C: `update_weight`.
fn update_weight(weight: &mut i32, delta: i32, source: i32, result: i32) {
    if source != 0 && result != 0 {
        let s = (source ^ result) >> 31;
        *weight = (delta ^ s).wrapping_add(weight.wrapping_sub(s));
    }
}

/// C: `update_weight_clip`.
fn update_weight_clip(weight: &mut i32, delta: i32, source: i32, result: i32) {
    if source != 0 && result != 0 {
        let s = (source ^ result) >> 31;
        *weight = (*weight ^ s).wrapping_add(delta.wrapping_sub(s));
        if *weight > 1024 {
            *weight = 1024;
        }
        *weight = (*weight ^ s).wrapping_sub(s);
    }
}

/// The 32-byte block header (wavpack.h `WavpackHeader`).
#[derive(Clone, Copy, Default)]
struct Header {
    ck_size: u32,
    version: u16,
    block_index_u8: u8,
    total_samples_u8: u8,
    total_samples: u32,
    block_index: u32,
    block_samples: u32,
    flags: u32,
    crc: u32,
}

impl Header {
    fn parse(b: &[u8]) -> Header {
        let u = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        Header {
            ck_size: u(4),
            version: u16::from_le_bytes([b[8], b[9]]),
            block_index_u8: b[10],
            total_samples_u8: b[11],
            total_samples: u(12),
            block_index: u(16),
            block_samples: u(20),
            flags: u(24),
            crc: u(28),
        }
    }

    fn block_index(&self) -> i64 {
        self.block_index as i64 + ((self.block_index_u8 as i64) << 32)
    }

    /// C: `SET_BLOCK_INDEX` (40 bits, wrapping).
    fn set_block_index(&mut self, v: i64) {
        self.block_index = v as u32;
        self.block_index_u8 = (v >> 32) as u8;
    }

    fn total_samples(&self) -> i64 {
        if self.total_samples == u32::MAX {
            -1
        } else {
            self.total_samples as i64 + ((self.total_samples_u8 as i64) << 32)
                - self.total_samples_u8 as i64
        }
    }
}

/// C: open_utils.c `read_next_header`: the next valid header at or after `pos`, as
/// (header offset, header). Every 'w' is a candidate, as the C scan.
fn read_next_header(data: &[u8], pos: usize) -> Option<(usize, Header)> {
    let mut p = pos;
    loop {
        if p + 32 > data.len() {
            return None;
        }
        let s = &data[p..p + 32];
        if s[..4] == *b"wvpk"
            && s[4] & 1 == 0
            && s[6] < 16
            && s[7] == 0
            && (s[6] != 0 || s[5] != 0 || s[4] >= 24)
            && s[9] == 4
            && s[8] >= 0x02
            && s[8] <= 0x10
            && s[22] < 3
            && s[23] == 0
        {
            return Some((p, Header::parse(s)));
        }
        p += 1;
        while p < data.len() && data[p] != b'w' {
            p += 1;
        }
        if p - pos > 1024 * 1024 {
            return None;
        }
    }
}

/// C: open_utils.c `seek_eof_information` (without the wrapper): the end index of the last
/// audio block near the file end, or -1. The reader's positions are modelled like
/// `CWavpackReader` (sound.cpp:549), whose relative seeks past the end fail.
fn seek_eof_information(data: &[u8]) -> i64 {
    let len = data.len() as i64;
    let mut final_index = -1;
    let (mut blocks, mut audio_blocks) = (0u32, 0u32);
    let mut last_pos = -1i64;
    let mut pos: i64 = if len > 1_048_576 { len - 1_048_576 } else { 0 };
    let seek_rel = |pos: &mut i64, delta: i64| {
        let p = *pos + delta;
        if (0..=len).contains(&p) {
            *pos = p;
        }
    };
    loop {
        let found = read_next_header(data, pos as usize);
        let current_pos = found.map_or(len, |(p, _)| p as i64 + 32);
        pos = current_pos;
        if current_pos == last_pos {
            return final_index;
        }
        last_pos = current_pos;
        let Some((_, hdr)) = found else {
            if blocks == 0 {
                pos = if current_pos > 2_000_000 {
                    current_pos - 2_000_000
                } else {
                    0
                };
                continue;
            }
            return final_index;
        };
        blocks += 1;
        if hdr.block_samples != 0 {
            final_index = hdr.block_index() + hdr.block_samples as i64;
            audio_blocks += 1;
        } else if audio_blocks == 0 {
            pos = if current_pos > 1_048_576 {
                current_pos - 1_048_576
            } else {
                0
            };
            continue;
        }
        let mut bcount = hdr.ck_size.wrapping_sub(24);
        while bcount >= 2 {
            let Some(&[meta_id, c1]) = data.get(pos as usize..pos as usize + 2) else {
                return final_index;
            };
            pos += 2;
            let mut meta_bc = (c1 as u32) << 1;
            bcount -= 2;
            if meta_id & ID_LARGE != 0 {
                let Some(&[c1, c2]) = data
                    .get(pos as usize..pos as usize + 2)
                    .filter(|_| bcount >= 2)
                else {
                    return final_index;
                };
                pos += 2;
                meta_bc += ((c1 as u32) << 9) + ((c2 as u32) << 17);
                bcount -= 2;
            }
            if meta_id & ID_UNIQUE == 0x26 && meta_bc == 16 && bcount >= 16 {
                if pos + 16 > len {
                    return final_index;
                }
                pos += 16;
            } else {
                seek_rel(&mut pos, meta_bc as i64);
            }
            bcount = bcount.wrapping_sub(meta_bc);
        }
    }
}

/// C: open_utils.c `WavpackVerifySingleBlock` (with checksum verification).
fn verify_single_block(block: &[u8]) -> bool {
    let hdr = Header::parse(block);
    if (hdr.ck_size as u64) + 8 < 32 {
        return false;
    }
    let mut bcount = hdr.ck_size - 24;
    let mut dp = 32;
    let mut checksum_passed = false;
    while bcount >= 2 {
        let meta_id = block[dp];
        let c1 = block[dp + 1];
        dp += 2;
        let mut meta_bc = (c1 as u32) << 1;
        bcount -= 2;
        if meta_id & ID_LARGE != 0 {
            if bcount < 2 {
                return false;
            }
            meta_bc += ((block[dp] as u32) << 9) + ((block[dp + 1] as u32) << 17);
            dp += 2;
            bcount -= 2;
        }
        if bcount < meta_bc {
            return false;
        }
        if meta_id & ID_UNIQUE == ID_BLOCK_CHECKSUM {
            if meta_id & ID_ODD_SIZE != 0 || !(2..=4).contains(&meta_bc) {
                return false;
            }
            // BITSTREAM_SHORTS: 16-bit words up to the checksum's metadata header
            let wcount = (dp - 2) >> 1;
            let mut csum = u32::MAX;
            for w in 0..wcount {
                csum = csum
                    .wrapping_mul(3)
                    .wrapping_add(u16::from_le_bytes([block[2 * w], block[2 * w + 1]]) as u32);
            }
            let d = &block[dp..];
            if meta_bc == 4 {
                if d[..4] != csum.to_le_bytes() {
                    return false;
                }
            } else {
                csum ^= csum >> 16;
                if d[0] != csum as u8 || d[1] != (csum >> 8) as u8 {
                    return false;
                }
            }
            checksum_passed = true;
        }
        bcount -= meta_bc;
        dp += meta_bc as usize;
    }
    // HAS_CHECKSUM
    bcount == 0 && (hdr.flags & 0x1000_0000 == 0 || checksum_passed)
}

/// C: open_utils.c `read_metadata_buff`, as an iterator of (id, data) over a block.
fn metadata(block: &[u8]) -> impl Iterator<Item = (u8, &[u8])> {
    let end = (Header::parse(block).ck_size as usize + 8).min(block.len());
    let mut p = 32;
    std::iter::from_fn(move || {
        if end.saturating_sub(p) < 2 {
            return None;
        }
        let mut id = block[p];
        let mut len = (block[p + 1] as usize) << 1;
        p += 2;
        if id & ID_LARGE != 0 {
            id &= !ID_LARGE;
            if end - p < 2 {
                return None;
            }
            len += ((block[p] as usize) << 9) + ((block[p + 1] as usize) << 17);
            p += 2;
        }
        if id & ID_ODD_SIZE != 0 {
            if len == 0 {
                return None;
            }
            id &= !ID_ODD_SIZE;
            len -= 1;
        }
        if len == 0 {
            return Some((id, &block[p..p]));
        }
        if end - p < len + (len & 1) {
            return None;
        }
        let d = &block[p..p + len];
        p += len + (len & 1);
        Some((id, d))
    })
}

/// C: `Bitstream` with `bs_read` as its wrap: reading past the end continues at the start.
#[derive(Clone, Default)]
struct Bits {
    data: Vec<u8>,
    pos: u64,
}

impl Bits {
    fn is_open(&self) -> bool {
        !self.data.is_empty()
    }

    fn bit_at(&self, pos: u64) -> u32 {
        let len = self.data.len() as u64;
        (self.data[((pos / 8) % len) as usize] >> (pos % 8)) as u32 & 1
    }

    fn getbit(&mut self) -> bool {
        let b = self.bit_at(self.pos);
        self.pos += 1;
        b != 0
    }

    /// The next `n` (<= 32) bits, LSB first, without consuming them.
    fn peek(&self, n: u32) -> u32 {
        (0..n).fold(0, |v, i| v | self.bit_at(self.pos + i as u64) << i)
    }

    /// C: read_words.c `read_code`.
    fn read_code(&mut self, maxcode: u32) -> u32 {
        if maxcode < 2 {
            return if maxcode != 0 {
                self.getbit() as u32
            } else {
                0
            };
        }
        let mut bitcount = count_bits(maxcode);
        let extras = ((1u64 << bitcount) - maxcode as u64 - 1) as u32;
        let local_sr = self.peek(bitcount);
        let mut code = local_sr & ((1u32 << (bitcount - 1)) - 1);
        if code >= extras {
            code = (code << 1)
                .wrapping_sub(extras)
                .wrapping_add((local_sr >> (bitcount - 1)) & 1);
        } else {
            bitcount -= 1;
        }
        self.pos += bitcount as u64;
        code
    }

    /// Consecutive ones, up to `LIMIT_ONES + 1`, plus the zero that ends them (the
    /// `USE_CTZ_OPTIMIZATION` block, which reads the same bits as the plain loop).
    fn ones(&mut self) -> u32 {
        let mut n = 0;
        while n < LIMIT_ONES + 1 && self.getbit() {
            n += 1;
        }
        n
    }

    /// The Elias-gamma-like count after a run of `LIMIT_ONES` ones (or for zero runs);
    /// `None` is C's `cbits == 33` end of data.
    fn escape(&mut self) -> Option<u32> {
        let mut cbits = 0;
        while cbits < 33 && self.getbit() {
            cbits += 1;
        }
        if cbits == 33 {
            return None;
        }
        if cbits < 2 {
            return Some(cbits);
        }
        let (mut mask, mut v) = (1u32, 0u32);
        cbits -= 1;
        while cbits > 0 {
            if self.getbit() {
                v |= mask;
            }
            mask = mask.wrapping_shl(1);
            cbits -= 1;
        }
        Some(v | mask)
    }
}

#[derive(Clone, Copy, Default)]
struct Entropy {
    median: [u32; 3],
    slow_level: u32,
    error_limit: u32,
}

impl Entropy {
    fn get_med(&self, i: usize) -> u32 {
        (self.median[i] >> 4) + 1
    }
    fn inc(&mut self, i: usize) {
        let div = [128, 64, 32][i];
        self.median[i] =
            self.median[i].wrapping_add((self.median[i].wrapping_add(div) / div).wrapping_mul(5));
    }
    fn dec(&mut self, i: usize) {
        let div = [128u32, 64, 32][i];
        self.median[i] = self.median[i]
            .wrapping_sub((self.median[i].wrapping_add(div - 2) / div).wrapping_mul(2));
    }
    /// The median breakpoints a ones count selects (shared by `get_word` and
    /// `get_words_lossless`): (low, high).
    fn range(&mut self, ones_count: u32) -> (u32, u32) {
        if ones_count == 0 {
            let high = self.get_med(0).wrapping_sub(1);
            self.dec(0);
            return (0, high);
        }
        let mut low = self.get_med(0);
        self.inc(0);
        if ones_count == 1 {
            let high = low.wrapping_add(self.get_med(1)).wrapping_sub(1);
            self.dec(1);
            return (low, high);
        }
        low = low.wrapping_add(self.get_med(1));
        self.inc(1);
        if ones_count == 2 {
            let high = low.wrapping_add(self.get_med(2)).wrapping_sub(1);
            self.dec(2);
            return (low, high);
        }
        low = low.wrapping_add((ones_count - 2).wrapping_mul(self.get_med(2)));
        let high = low.wrapping_add(self.get_med(2)).wrapping_sub(1);
        self.inc(2);
        (low, high)
    }
}

#[derive(Clone, Copy, Default)]
struct Words {
    bitrate_delta: [u32; 2],
    bitrate_acc: [u32; 2],
    holding_one: u32,
    zeros_acc: u32,
    holding_zero: u32,
    c: [Entropy; 2],
}

#[derive(Clone, Copy, Default)]
struct DecorrPass {
    term: i32,
    delta: i32,
    weight_a: i32,
    weight_b: i32,
    samples_a: [i32; 8],
    samples_b: [i32; 8],
}

/// C: `WavpackStream`, single stream, no correction file.
#[derive(Clone, Default)]
struct Stream {
    hdr: Header,
    sample_index: i64,
    mute_error: bool,
    crc: u32,
    num_terms: usize,
    passes: [DecorrPass; MAX_NTERMS],
    w: Words,
    wvbits: Bits,
    /// C: int32_sent_bits, zeros, ones, dups; `unpack_init` keeps them (stale values apply
    /// to a block without its own INT32_INFO)
    int32: [i32; 4],
    /// C: `init_done`
    init_done: bool,
    /// A wvx stream was seen (extended 32-bit data, not ported: `decode` refuses)
    wvx: bool,
}

/// C: `WavpackContext` fields the decode reads.
#[derive(Default)]
struct Context {
    num_channels: i32,
    sample_rate: u32,
}

impl Stream {
    fn flags(&self) -> u32 {
        self.hdr.flags
    }

    fn mono(&self) -> bool {
        self.flags() & MONO_DATA != 0
    }

    /// C: open_utils.c `unpack_init` for one block.
    fn unpack_init(&mut self, ctx: &mut Context, block: &[u8]) -> bool {
        self.num_terms = 0;
        self.mute_error = false;
        self.crc = u32::MAX;
        self.wvbits = Bits::default();
        self.passes = Default::default();
        self.w = Words::default();
        let f = self.flags();
        if f & MONO_FLAG == 0
            && ctx.num_channels != 0
            && self.hdr.block_samples != 0
            && ctx.num_channels == 1
        {
            self.mute_error = true;
            return false;
        }
        if f & MONO_DATA == MONO_DATA {
            self.mute_error = true;
            return false;
        }
        for (id, d) in metadata(block) {
            if !self.process_metadata(ctx, id, d) {
                self.mute_error = true;
                return false;
            }
        }
        if self.hdr.block_samples != 0 && !self.wvbits.is_open() {
            self.mute_error = true;
            return false;
        }
        if self.hdr.block_samples != 0 {
            self.sample_index = self.hdr.block_index();
        }
        true
    }

    /// C: open_utils.c `process_metadata` (the ids a stereo/mono PCM stream uses; the
    /// others are accepted like the library's optional data or rejected).
    fn process_metadata(&mut self, ctx: &mut Context, id: u8, d: &[u8]) -> bool {
        let mono = self.mono();
        let per = if mono { 1 } else { 2 };
        match id {
            0x0 => true,
            0x2 => self.read_decorr_terms(d),
            0x3 => self.read_decorr_weights(d),
            0x4 => self.read_decorr_samples(d),
            0x5 => {
                // read_entropy_vars
                if d.len() != 6 * per {
                    return false;
                }
                for ch in 0..per {
                    for i in 0..3 {
                        self.w.c[ch].median[i] = wp_exp2s(
                            d[6 * ch + 2 * i] as i32 + ((d[6 * ch + 2 * i + 1] as i32) << 8),
                        ) as u32;
                    }
                }
                true
            }
            0x6 => self.read_hybrid_profile(d),
            // read_shaping_info: the shaping state only matters with a correction file
            0x7 => d.len() == 2 || d.len() >= 4 * per,
            // read_float_info
            0x8 => d.len() == 4,
            0x9 => {
                // read_int32_info
                if d.len() != 4 {
                    return false;
                }
                self.int32 = [
                    d[0] as i8 as i32,
                    d[1] as i8 as i32,
                    d[2] as i8 as i32,
                    d[3] as i8 as i32,
                ];
                true
            }
            0xa => {
                // init_wv_bitstream
                if d.is_empty() || d.len() & 1 != 0 {
                    return false;
                }
                self.wvbits = Bits {
                    data: d.to_vec(),
                    pos: 0,
                };
                true
            }
            // init_wvc_bitstream: without a correction file its data is never read
            0xb => !d.is_empty() && d.len() & 1 == 0,
            // init_wvx_bitstream
            0xc | 0x2c => {
                self.wvx = d.len() > 4 && d.len() & 1 == 0;
                self.wvx
            }
            0xd => read_channel_info(ctx, d),
            // DSD blocks are refused at open (DECISIONS.md 13.3)
            0xe => false,
            // read_channel_identities
            0x2b => !d.is_empty() && d.iter().all(|&c| c != 0),
            // read_new_config_info: only the channel reordering can fail
            0x2a => d.len() < 5 || d.len() - 4 <= d[3] as usize,
            0x25 | 0x26 | 0x28 | 0x2f | 0x21 | 0x22 => true,
            0x27 => {
                // read_sample_rate
                if d.len() == 3 || d.len() == 4 {
                    let mut r = d[0] as u32 | (d[1] as u32) << 8 | (d[2] as u32) << 16;
                    if d.len() == 4 {
                        r |= ((d[3] & 0x7f) as u32) << 24;
                    }
                    ctx.sample_rate = r;
                }
                true
            }
            // ID_ALT_*: ignored without OPEN_ALT_TYPES
            0x23 | 0x24 | 0x29 => true,
            _ => id & ID_OPTIONAL_DATA != 0,
        }
    }

    /// C: decorr_utils.c `read_decorr_terms`.
    fn read_decorr_terms(&mut self, d: &[u8]) -> bool {
        if d.len() > MAX_NTERMS {
            return false;
        }
        self.num_terms = d.len();
        let mono = self.mono();
        for (i, &b) in d.iter().enumerate() {
            let p = &mut self.passes[d.len() - 1 - i];
            p.term = (b & 0x1f) as i32 - 5;
            p.delta = ((b >> 5) & 7) as i32;
            if p.term == 0
                || p.term < -3
                || (p.term > MAX_TERM && p.term < 17)
                || p.term > 18
                || (mono && p.term < 0)
            {
                return false;
            }
        }
        true
    }

    /// C: decorr_utils.c `read_decorr_weights`.
    fn read_decorr_weights(&mut self, d: &[u8]) -> bool {
        let mono = self.mono();
        let termcnt = if mono { d.len() } else { d.len() / 2 };
        if termcnt > self.num_terms {
            return false;
        }
        for p in &mut self.passes[..self.num_terms] {
            p.weight_a = 0;
            p.weight_b = 0;
        }
        let mut b = d.iter();
        for p in self.passes[..self.num_terms].iter_mut().rev().take(termcnt) {
            p.weight_a = restore_weight(*b.next().unwrap_or(&0) as i8);
            if !mono {
                p.weight_b = restore_weight(*b.next().unwrap_or(&0) as i8);
            }
        }
        true
    }

    /// C: decorr_utils.c `read_decorr_samples`.
    fn read_decorr_samples(&mut self, d: &[u8]) -> bool {
        let mono = self.mono();
        for p in &mut self.passes[..self.num_terms] {
            p.samples_a = [0; 8];
            p.samples_b = [0; 8];
        }
        let mut b = 0;
        let end = d.len();
        if self.hdr.version == 0x402 && self.flags() & HYBRID_FLAG != 0 {
            // dc.error, only read with a correction file
            b += if mono { 2 } else { 4 };
            if b > end {
                return false;
            }
        }
        for p in self.passes[..self.num_terms].iter_mut().rev() {
            if b >= end {
                break;
            }
            if p.term > MAX_TERM {
                if b + if mono { 4 } else { 8 } > end {
                    return false;
                }
                p.samples_a[0] = wp_exp2s(le16s(&d[b..]));
                p.samples_a[1] = wp_exp2s(le16s(&d[b + 2..]));
                b += 4;
                if !mono {
                    p.samples_b[0] = wp_exp2s(le16s(&d[b..]));
                    p.samples_b[1] = wp_exp2s(le16s(&d[b + 2..]));
                    b += 4;
                }
            } else if p.term < 0 {
                if b + 4 > end {
                    return false;
                }
                p.samples_a[0] = wp_exp2s(le16s(&d[b..]));
                p.samples_b[0] = wp_exp2s(le16s(&d[b + 2..]));
                b += 4;
            } else {
                for m in 0..p.term as usize {
                    if b + if mono { 2 } else { 4 } > end {
                        return false;
                    }
                    p.samples_a[m] = wp_exp2s(le16s(&d[b..]));
                    b += 2;
                    if !mono {
                        p.samples_b[m] = wp_exp2s(le16s(&d[b..]));
                        b += 2;
                    }
                }
            }
        }
        b == end
    }

    /// C: entropy_utils.c `read_hybrid_profile`.
    fn read_hybrid_profile(&mut self, d: &[u8]) -> bool {
        let per = if self.mono() { 1 } else { 2 };
        let u16le = |i: usize| d[i] as u32 + ((d[i + 1] as u32) << 8);
        let mut b = 0;
        if self.flags() & HYBRID_BITRATE != 0 {
            if b + 2 * per > d.len() {
                return false;
            }
            for ch in 0..per {
                self.w.c[ch].slow_level = wp_exp2s(u16le(b) as i32) as u32;
                b += 2;
            }
        }
        if b + 2 * per > d.len() {
            return false;
        }
        for ch in 0..per {
            self.w.bitrate_acc[ch] = u16le(b) << 16;
            b += 2;
        }
        if b < d.len() {
            if b + 2 * per > d.len() {
                return false;
            }
            for ch in 0..per {
                self.w.bitrate_delta[ch] = wp_exp2s(u16le(b) as u16 as i16 as i32) as u32;
                b += 2;
            }
            if b < d.len() {
                return false;
            }
        } else {
            self.w.bitrate_delta = [0; 2];
        }
        true
    }

    /// C: entropy_utils.c `update_error_limit`.
    fn update_error_limit(&mut self) {
        let w = &mut self.w;
        w.bitrate_acc[0] = w.bitrate_acc[0].wrapping_add(w.bitrate_delta[0]);
        let mut bitrate_0 = (w.bitrate_acc[0] >> 16) as i32;
        let f = self.hdr.flags;
        let limit = |slow_log: i32, bitrate: i32| {
            if slow_log - bitrate > -0x100 {
                wp_exp2s(slow_log - bitrate + 0x100) as u32
            } else {
                0
            }
        };
        if f & MONO_DATA != 0 {
            w.c[0].error_limit = if f & HYBRID_BITRATE != 0 {
                limit(
                    (w.c[0].slow_level.wrapping_add(SLO) >> SLS) as i32,
                    bitrate_0,
                )
            } else {
                wp_exp2s(bitrate_0) as u32
            };
            return;
        }
        w.bitrate_acc[1] = w.bitrate_acc[1].wrapping_add(w.bitrate_delta[1]);
        let mut bitrate_1 = (w.bitrate_acc[1] >> 16) as i32;
        if f & HYBRID_BITRATE != 0 {
            let slow_log_0 = (w.c[0].slow_level.wrapping_add(SLO) >> SLS) as i32;
            let slow_log_1 = (w.c[1].slow_level.wrapping_add(SLO) >> SLS) as i32;
            if f & HYBRID_BALANCE != 0 {
                let balance = (slow_log_1 - slow_log_0 + bitrate_1 + 1) >> 1;
                if balance > bitrate_0 {
                    bitrate_1 = bitrate_0 * 2;
                    bitrate_0 = 0;
                } else if -balance > bitrate_0 {
                    bitrate_0 *= 2;
                    bitrate_1 = 0;
                } else {
                    bitrate_1 = bitrate_0 + balance;
                    bitrate_0 -= balance;
                }
            }
            w.c[0].error_limit = limit(slow_log_0, bitrate_0);
            w.c[1].error_limit = limit(slow_log_1, bitrate_1);
        } else {
            w.c[0].error_limit = wp_exp2s(bitrate_0) as u32;
            w.c[1].error_limit = wp_exp2s(bitrate_1) as u32;
        }
    }

    /// C: read_words.c `get_word` without a correction stream.
    fn get_word(&mut self, chan: usize) -> i32 {
        if self.w.c[0].median[0] & !1 == 0
            && self.w.holding_zero == 0
            && self.w.holding_one == 0
            && self.w.c[1].median[0] & !1 == 0
        {
            if self.w.zeros_acc != 0 {
                self.w.zeros_acc -= 1;
                if self.w.zeros_acc != 0 {
                    let c = &mut self.w.c[chan];
                    c.slow_level = c
                        .slow_level
                        .wrapping_sub(c.slow_level.wrapping_add(SLO) >> SLS);
                    return 0;
                }
            } else {
                let Some(z) = self.wvbits.escape() else {
                    return WORD_EOF;
                };
                self.w.zeros_acc = z;
                if z != 0 {
                    let c = &mut self.w.c[chan];
                    c.slow_level = c
                        .slow_level
                        .wrapping_sub(c.slow_level.wrapping_add(SLO) >> SLS);
                    self.w.c[0].median = [0; 3];
                    self.w.c[1].median = [0; 3];
                    return 0;
                }
            }
        }
        let mut ones_count;
        if self.w.holding_zero != 0 {
            ones_count = 0;
            self.w.holding_zero = 0;
        } else {
            ones_count = self.wvbits.ones();
            if ones_count == LIMIT_ONES + 1 {
                return WORD_EOF;
            }
            if ones_count == LIMIT_ONES {
                let Some(n) = self.wvbits.escape() else {
                    return WORD_EOF;
                };
                ones_count = n.wrapping_add(LIMIT_ONES);
            }
            if self.w.holding_one != 0 {
                self.w.holding_one = ones_count & 1;
                ones_count = (ones_count >> 1) + 1;
            } else {
                self.w.holding_one = ones_count & 1;
                ones_count >>= 1;
            }
            self.w.holding_zero = !self.w.holding_one & 1;
        }
        if self.hdr.flags & HYBRID_FLAG != 0 && chan == 0 {
            self.update_error_limit();
        }
        let c = &mut self.w.c[chan];
        let (mut low, mut high) = c.range(ones_count);
        low &= 0x7fff_ffff;
        high &= 0x7fff_ffff;
        if low > high {
            high = low;
        }
        let mut mid = (high + low + 1) >> 1;
        let error_limit = c.error_limit;
        if error_limit == 0 {
            mid = self.wvbits.read_code(high - low).wrapping_add(low);
        } else {
            while high - low > error_limit {
                if self.wvbits.getbit() {
                    low = mid;
                    mid = (high + low + 1) >> 1;
                } else {
                    high = mid.wrapping_sub(1);
                    mid = (high.wrapping_add(low).wrapping_add(1)) >> 1;
                }
            }
        }
        let sign = self.wvbits.getbit();
        let c = &mut self.w.c[chan];
        if self.hdr.flags & HYBRID_BITRATE != 0 {
            c.slow_level = c
                .slow_level
                .wrapping_sub(c.slow_level.wrapping_add(SLO) >> SLS);
            c.slow_level = c.slow_level.wrapping_add(wp_log2(mid) as u32);
        }
        if sign { !(mid as i32) } else { mid as i32 }
    }

    /// C: read_words.c `get_words_lossless`; returns the samples (frames) read.
    fn get_words_lossless(&mut self, buffer: &mut [i32], nsamples: usize) -> usize {
        let mono = self.mono();
        let n = if mono { nsamples } else { nsamples * 2 };
        let mut cs = 0;
        let chan = |cs: usize| if mono { 0 } else { cs & 1 };
        while cs < n {
            let mut ch = chan(cs);
            if self.w.holding_zero != 0 {
                self.w.holding_zero = 0;
                let med = self.w.c[ch].get_med(0).wrapping_sub(1);
                let low = self.wvbits.read_code(med);
                self.w.c[ch].dec(0);
                buffer[cs] = if self.wvbits.getbit() {
                    !(low as i32)
                } else {
                    low as i32
                };
                cs += 1;
                if cs == n {
                    break;
                }
                ch = chan(cs);
            }
            if self.w.c[0].median[0] < 2 && self.w.holding_one == 0 && self.w.c[1].median[0] < 2 {
                if self.w.zeros_acc != 0 {
                    self.w.zeros_acc -= 1;
                    if self.w.zeros_acc != 0 {
                        buffer[cs] = 0;
                        cs += 1;
                        continue;
                    }
                } else {
                    let Some(z) = self.wvbits.escape() else {
                        break;
                    };
                    self.w.zeros_acc = z;
                    if z != 0 {
                        self.w.c[0].median = [0; 3];
                        self.w.c[1].median = [0; 3];
                        buffer[cs] = 0;
                        cs += 1;
                        continue;
                    }
                }
            }
            let mut ones_count = self.wvbits.ones();
            if ones_count == LIMIT_ONES + 1 {
                break;
            }
            if ones_count == LIMIT_ONES {
                let Some(c) = self.wvbits.escape() else {
                    break;
                };
                ones_count = c.wrapping_add(LIMIT_ONES);
            }
            let low = self.w.holding_one;
            self.w.holding_one = ones_count & 1;
            self.w.holding_zero = !ones_count & 1;
            let ones_count = (ones_count >> 1).wrapping_add(low);
            let (low, high) = self.w.c[ch].range(ones_count);
            let low = low.wrapping_add(self.wvbits.read_code(high.wrapping_sub(low)));
            buffer[cs] = if self.wvbits.getbit() {
                !(low as i32)
            } else {
                low as i32
            };
            cs += 1;
        }
        if mono { cs } else { cs / 2 }
    }

    /// C: unpack.c `unpack_samples` without a correction stream. `buffer` is the caller's
    /// buffer from this block's first output sample on.
    fn unpack_samples(&mut self, buffer: &mut [i32], mut sample_count: usize) -> usize {
        let flags = self.flags();
        let mut crc = self.crc;
        let mag = (flags & MAG_MASK) >> MAG_LSB;
        let mut mute_limit = 1i32.wrapping_shl(mag).wrapping_add(2);
        let block_end = self.hdr.block_index() + self.hdr.block_samples as i64;
        if self.sample_index + sample_count as i64 > block_end
            && ((block_end - self.sample_index) as u32 as usize) < sample_count
        {
            sample_count = (block_end - self.sample_index) as u32 as usize;
        }
        if self.hdr.block_index() > self.sample_index
            || (self.hdr.block_samples as usize) < sample_count
        {
            self.mute_error = true;
        }
        let zero = |buffer: &mut [i32], n: usize| {
            let n = n.min(buffer.len());
            buffer[..n].fill(0);
        };
        if self.mute_error {
            // C: reduced_channels == 1 || num_channels == 1 || MONO_FLAG; a stereo block in
            // a mono file never gets here (WavpackUnpackSamples catches it)
            zero(
                buffer,
                if flags & MONO_FLAG != 0 {
                    sample_count
                } else {
                    sample_count * 2
                },
            );
            self.sample_index += sample_count as i64;
            return sample_count;
        }
        if flags & HYBRID_FLAG != 0 {
            mute_limit = mute_limit.wrapping_mul(2).wrapping_add(128);
        }
        let mut m = 0usize;
        let mut i;
        let mono = flags & MONO_DATA != 0;
        let eof;
        if mono {
            if flags & HYBRID_FLAG != 0 {
                i = sample_count;
                for (k, b) in buffer[..sample_count].iter_mut().enumerate() {
                    *b = self.get_word(0);
                    if *b == WORD_EOF {
                        // C: i = bptr - buffer after the increment
                        i = k + 1;
                        break;
                    }
                }
            } else {
                i = self.get_words_lossless(buffer, sample_count);
            }
            eof = i != sample_count;
            if !eof {
                for p in &mut self.passes[..self.num_terms] {
                    decorr_mono_pass(p, &mut buffer[..sample_count]);
                }
                for (k, &v) in buffer[..sample_count].iter().enumerate() {
                    if v.wrapping_abs() > mute_limit {
                        i = k;
                        break;
                    }
                    crc = crc.wrapping_mul(3).wrapping_add(v as u32);
                }
            }
        } else {
            if flags & HYBRID_FLAG != 0 {
                i = sample_count;
                for k in 0..sample_count {
                    buffer[2 * k] = self.get_word(0);
                    if buffer[2 * k] == WORD_EOF {
                        i = k;
                        break;
                    }
                    buffer[2 * k + 1] = self.get_word(1);
                    if buffer[2 * k + 1] == WORD_EOF {
                        i = k;
                        break;
                    }
                }
            } else {
                i = self.get_words_lossless(buffer, sample_count);
            }
            eof = i != sample_count;
            if !eof {
                let b = &mut buffer[..sample_count * 2];
                for p in &mut self.passes[..self.num_terms] {
                    decorr_stereo_pass(p, b);
                }
                m = sample_count & (MAX_TERM as usize - 1);
                for s in b.as_chunks_mut::<2>().0 {
                    if flags & JOINT_STEREO != 0 {
                        s[1] = s[1].wrapping_sub(s[0] >> 1);
                        s[0] = s[0].wrapping_add(s[1]);
                    }
                    crc = crc
                        .wrapping_add(crc << 3)
                        .wrapping_add((s[0] as u32) << 1)
                        .wrapping_add(s[0] as u32)
                        .wrapping_add(s[1] as u32);
                }
                for (k, s) in b.as_chunks::<2>().0.iter().enumerate().step_by(8) {
                    if s[0].wrapping_abs() > mute_limit || s[1].wrapping_abs() > mute_limit {
                        i = k;
                        break;
                    }
                }
            }
        }
        if i != sample_count {
            zero(
                buffer,
                if flags & MONO_FLAG != 0 {
                    sample_count
                } else {
                    sample_count * 2
                },
            );
            self.mute_error = true;
            i = sample_count;
        }
        if m != 0 {
            for p in &mut self.passes[..self.num_terms] {
                if p.term > 0 && p.term <= MAX_TERM {
                    let (ta, tb) = (p.samples_a, p.samples_b);
                    for k in 0..8 {
                        p.samples_a[k] = ta[m];
                        p.samples_b[k] = tb[m];
                        m = (m + 1) & 7;
                    }
                }
            }
        }
        self.fixup_samples(buffer, i);
        if flags & FALSE_STEREO != 0 {
            for k in (0..i).rev() {
                buffer[2 * k + 1] = buffer[k];
                buffer[2 * k] = buffer[k];
            }
        }
        self.sample_index += i as i64;
        self.crc = crc;
        if !self.mute_error && self.sample_index == block_end && self.crc != self.hdr.crc {
            zero(
                buffer,
                if flags & MONO_FLAG != 0 {
                    sample_count
                } else {
                    sample_count * 2
                },
            );
            self.mute_error = true;
        }
        i
    }

    /// C: unpack.c `fixup_samples` (no float data, no wvx stream).
    fn fixup_samples(&self, buffer: &mut [i32], sample_count: usize) {
        let flags = self.flags();
        let lossy = flags & HYBRID_FLAG != 0;
        let mut shift = ((flags & SHIFT_MASK) >> SHIFT_LSB) as i32;
        let count = if flags & MONO_DATA != 0 {
            sample_count
        } else {
            sample_count * 2
        };
        let buf = &mut buffer[..count];
        if flags & INT32_DATA != 0 {
            let sent_bits = self.int32[0] & 0x1f;
            let (mut zeros, mut ones, mut dups) = (
                self.int32[1] & 0x1f,
                self.int32[2] & 0x1f,
                self.int32[3] & 0x1f,
            );
            if sent_bits == 0 && zeros + ones + dups != 0 {
                while lossy && flags & BYTES_STORED == 3 && shift < 8 {
                    if zeros != 0 {
                        zeros -= 1;
                    } else if ones != 0 {
                        ones -= 1;
                    } else if dups != 0 {
                        dups -= 1;
                    } else {
                        break;
                    }
                    shift += 1;
                }
                for v in buf.iter_mut() {
                    if zeros != 0 {
                        *v = ((*v as u32) << zeros) as i32;
                    } else if ones != 0 {
                        *v = ((v.wrapping_add(1) as u32) << ones) as i32;
                        *v = v.wrapping_sub(1);
                    } else if dups != 0 {
                        let odd = *v & 1;
                        *v = ((v.wrapping_add(odd) as u32) << dups) as i32;
                        *v = v.wrapping_sub(odd);
                    }
                }
            } else {
                shift += zeros + sent_bits + ones + dups;
            }
        }
        let shift = (shift & 0x1f) as u32;
        if lossy {
            let (min_value, max_value): (i32, i32) = match flags & BYTES_STORED {
                0 => (-128 >> shift, 127 >> shift),
                1 => (-32768 >> shift, 32767 >> shift),
                2 => (-8388608 >> shift, 8388607 >> shift),
                _ => (i32::MIN >> shift, i32::MAX >> shift),
            };
            let min_shifted = ((min_value as u32) << shift) as i32;
            let max_shifted = ((max_value as u32) << shift) as i32;
            for v in buf.iter_mut() {
                *v = if *v < min_value {
                    min_shifted
                } else if *v > max_value {
                    max_shifted
                } else {
                    ((*v as u32) << shift) as i32
                };
            }
        } else if shift != 0 {
            for v in buf.iter_mut() {
                *v = ((*v as u32) << shift) as i32;
            }
        }
    }
}

/// C: open_utils.c `read_channel_info`.
fn read_channel_info(ctx: &mut Context, d: &[u8]) -> bool {
    if d.is_empty() || d.len() > 7 {
        return false;
    }
    if ctx.num_channels != 0 {
        return true;
    }
    let mut mask: u32 = 0;
    let max_streams;
    if d.len() >= 6 {
        ctx.num_channels = (d[0] as i32 | ((d[2] as i32 & 0xf) << 8)) + 1;
        max_streams = (d[1] as i32 | ((d[2] as i32 & 0xf0) << 4)) + 1;
        if ctx.num_channels < max_streams {
            return false;
        }
        mask = d[3] as u32 | (d[4] as u32) << 8 | (d[5] as u32) << 16;
        if d.len() == 7 {
            mask |= (d[6] as u32) << 24;
        }
    } else {
        ctx.num_channels = d[0] as i32;
        for (k, &b) in d[1..].iter().enumerate() {
            mask |= (b as u32) << (8 * k);
        }
        // OLD_MAX_STREAMS
        max_streams = ctx.num_channels.max(8);
    }
    if ctx.num_channels > max_streams * 2 {
        return false;
    }
    mask.count_ones() as i32 <= ctx.num_channels
}

/// C: unpack.c `decorr_mono_pass`.
fn decorr_mono_pass(p: &mut DecorrPass, buffer: &mut [i32]) {
    let (delta, mut weight_a) = (p.delta, p.weight_a);
    match p.term {
        17 | 18 => {
            for b in buffer.iter_mut() {
                let sam_a = if p.term == 17 {
                    p.samples_a[0].wrapping_mul(2).wrapping_sub(p.samples_a[1])
                } else {
                    p.samples_a[0].wrapping_mul(3).wrapping_sub(p.samples_a[1]) >> 1
                };
                p.samples_a[1] = p.samples_a[0];
                p.samples_a[0] = apply_weight(weight_a, sam_a).wrapping_add(*b);
                update_weight(&mut weight_a, delta, sam_a, *b);
                *b = p.samples_a[0];
            }
        }
        _ => {
            let mut m = 0usize;
            let mut k = (p.term & (MAX_TERM - 1)) as usize;
            for b in buffer.iter_mut() {
                let sam_a = p.samples_a[m];
                p.samples_a[k] = apply_weight(weight_a, sam_a).wrapping_add(*b);
                update_weight(&mut weight_a, delta, sam_a, *b);
                *b = p.samples_a[k];
                m = (m + 1) & 7;
                k = (k + 1) & 7;
            }
            if m != 0 {
                let t = p.samples_a;
                for (k, s) in p.samples_a.iter_mut().enumerate() {
                    *s = t[(m + k) & 7];
                }
            }
        }
    }
    p.weight_a = weight_a;
}

/// C: unpack.c `decorr_stereo_pass`.
fn decorr_stereo_pass(p: &mut DecorrPass, buffer: &mut [i32]) {
    let delta = p.delta;
    match p.term {
        17 | 18 => {
            let term = p.term;
            for b in buffer.as_chunks_mut::<2>().0 {
                for (ch, (w, s)) in [
                    (&mut p.weight_a, &mut p.samples_a),
                    (&mut p.weight_b, &mut p.samples_b),
                ]
                .into_iter()
                .enumerate()
                {
                    let sam = if term == 17 {
                        s[0].wrapping_mul(2).wrapping_sub(s[1])
                    } else {
                        s[0].wrapping_add(s[0].wrapping_sub(s[1]) >> 1)
                    };
                    s[1] = s[0];
                    let tmp = b[ch];
                    s[0] = apply_weight(*w, sam).wrapping_add(tmp);
                    b[ch] = s[0];
                    update_weight(w, delta, sam, tmp);
                }
            }
        }
        -1 => {
            for b in buffer.as_chunks_mut::<2>().0 {
                let sam = b[0].wrapping_add(apply_weight(p.weight_a, p.samples_a[0]));
                update_weight_clip(&mut p.weight_a, delta, p.samples_a[0], b[0]);
                b[0] = sam;
                p.samples_a[0] = b[1].wrapping_add(apply_weight(p.weight_b, sam));
                update_weight_clip(&mut p.weight_b, delta, sam, b[1]);
                b[1] = p.samples_a[0];
            }
        }
        -2 => {
            for b in buffer.as_chunks_mut::<2>().0 {
                let sam = b[1].wrapping_add(apply_weight(p.weight_b, p.samples_b[0]));
                update_weight_clip(&mut p.weight_b, delta, p.samples_b[0], b[1]);
                b[1] = sam;
                p.samples_b[0] = b[0].wrapping_add(apply_weight(p.weight_a, sam));
                update_weight_clip(&mut p.weight_a, delta, sam, b[0]);
                b[0] = p.samples_b[0];
            }
        }
        -3 => {
            for b in buffer.as_chunks_mut::<2>().0 {
                let sam_a = b[0].wrapping_add(apply_weight(p.weight_a, p.samples_a[0]));
                update_weight_clip(&mut p.weight_a, delta, p.samples_a[0], b[0]);
                let sam_b = b[1].wrapping_add(apply_weight(p.weight_b, p.samples_b[0]));
                update_weight_clip(&mut p.weight_b, delta, p.samples_b[0], b[1]);
                p.samples_b[0] = sam_a;
                b[0] = sam_a;
                p.samples_a[0] = sam_b;
                b[1] = sam_b;
            }
        }
        _ => {
            let mut m = 0usize;
            let mut k = (p.term & (MAX_TERM - 1)) as usize;
            for b in buffer.as_chunks_mut::<2>().0 {
                let sam = p.samples_a[m];
                p.samples_a[k] = apply_weight(p.weight_a, sam).wrapping_add(b[0]);
                update_weight(&mut p.weight_a, delta, sam, b[0]);
                b[0] = p.samples_a[k];
                let sam = p.samples_b[m];
                p.samples_b[k] = apply_weight(p.weight_b, sam).wrapping_add(b[1]);
                update_weight(&mut p.weight_b, delta, sam, b[1]);
                b[1] = p.samples_b[k];
                m = (m + 1) & 7;
                k = (k + 1) & 7;
            }
        }
    }
}

/// Reads the block at `p` (header already found there); `None` when the file ends
/// before the block does.
fn read_block(data: &[u8], p: usize, hdr: &Header) -> Option<Vec<u8>> {
    let end = p.checked_add(hdr.ck_size as usize + 8)?;
    data.get(p..end).map(|b| b.to_vec())
}

/// C++ `CSound::DecodeWV` (sound.cpp:603): open, check channels/bits/length, unpack every
/// sample and cast to 16 bits. `None` where C++ logs an error and returns false.
pub fn decode(data: &[u8]) -> Option<Decoded> {
    // WavpackOpenFileInputEx64 (open_utils.c:35), flags 0
    if data.first() == Some(&b'R') {
        return None; // legacy (pre-4.0) files need ENABLE_LEGACY
    }
    let mut ctx = Context::default();
    let mut wps = Stream::default();
    let mut total_samples: i64 = -1;
    let mut initial_index = 0;
    let mut pos = 0;
    let mut num_blocks = 0;
    let mut block;
    loop {
        let (p, hdr) = read_next_header(data, pos)?;
        if hdr.block_samples == 0 {
            let n = num_blocks;
            num_blocks += 1;
            if n > 16 {
                return None;
            }
        }
        wps.hdr = hdr;
        block = read_block(data, p, &hdr)?;
        pos = p + block.len();
        if !verify_single_block(&block) {
            wps.hdr.block_samples = 0;
            continue;
        }
        if wps.hdr.block_samples != 0 {
            if total_samples == -1 {
                if wps.hdr.block_index() != 0 || wps.hdr.total_samples() == -1 {
                    initial_index = wps.hdr.block_index();
                    wps.hdr.set_block_index(0);
                    let final_index = seek_eof_information(data);
                    if final_index != -1 {
                        total_samples = final_index - initial_index;
                    }
                } else {
                    total_samples = wps.hdr.total_samples();
                }
            }
        } else if total_samples == -1 && wps.hdr.block_index() == 0 && wps.hdr.total_samples != 0 {
            total_samples = wps.hdr.total_samples();
        }
        if !wps.unpack_init(&mut ctx, &block) {
            return None;
        }
        if wps.hdr.block_samples != 0 {
            break;
        }
    }
    wps.init_done = true;
    if wps.wvx {
        return None;
    }
    let first_flags = wps.hdr.flags;
    if ctx.num_channels == 0 {
        ctx.num_channels = if first_flags & MONO_FLAG != 0 { 1 } else { 2 };
    }
    if first_flags & DSD_FLAG != 0 || first_flags & FLOAT_DATA != 0 {
        // DSD needs OPEN_DSD_*; float data only with 32-bit samples, which DDNet refuses
        // except malformed ones (DECISIONS.md 13.3)
        return None;
    }
    let bits_per_sample = (((first_flags & BYTES_STORED) + 1) * 8) as i32
        - ((first_flags & SHIFT_MASK) >> SHIFT_LSB) as i32;
    if ctx.sample_rate == 0 {
        ctx.sample_rate = if (first_flags & SRATE_MASK) == SRATE_MASK {
            44100
        } else {
            SAMPLE_RATES[((first_flags & SRATE_MASK) >> SRATE_LSB) as usize]
        };
    }

    // CSound::DecodeWV checks
    let num_channels = ctx.num_channels;
    if num_channels > 2 || bits_per_sample != 16 {
        return None;
    }
    let num_samples = total_samples;
    if num_samples <= 0 || num_samples > i32::MAX as i64 {
        return None;
    }
    let num_samples = num_samples as usize;
    let nch = num_channels as usize;
    // C++ callocs whatever the header says (up to INT_MAX frames) and crashes when that
    // fails; here a failed allocation is a failed decode
    let mut buffer = Vec::new();
    buffer.try_reserve_exact(num_samples * nch).ok()?;
    buffer.resize(num_samples * nch, 0i32);

    // WavpackUnpackSamples (unpack_utils.c:108) for all samples
    let mut samples = num_samples;
    let mut samples_unpacked = 0;
    let mut bptr = 0;
    while samples > 0 {
        if wps.hdr.block_samples == 0
            || wps.hdr.flags & INITIAL_BLOCK == 0
            || wps.sample_index >= wps.hdr.block_index() + wps.hdr.block_samples as i64
        {
            let Some((p, hdr)) = read_next_header(data, pos) else {
                break;
            };
            wps.hdr = hdr;
            let Some(b) = read_block(data, p, &hdr) else {
                break;
            };
            block = b;
            pos = p + block.len();
            if !verify_single_block(&block) {
                // C: the block shrinks to its header (ckSize 24, no samples)
                wps.hdr.ck_size = 24;
                wps.hdr.block_samples = 0;
                block.truncate(32);
                block[4..8].copy_from_slice(&24u32.to_le_bytes());
            }
            let bi = wps.hdr.block_index();
            wps.hdr.set_block_index(bi - initial_index);
            if wps.hdr.block_samples == 0 {
                wps.unpack_init(&mut ctx, &block);
                // a non-audio block: its init is done, the loop reads the next header
                continue;
            }
            wps.init_done = false;
        }
        if wps.hdr.block_samples == 0
            || wps.hdr.flags & INITIAL_BLOCK == 0
            || wps.sample_index >= wps.hdr.block_index() + wps.hdr.block_samples as i64
        {
            continue;
        }
        if wps.sample_index < wps.hdr.block_index() {
            let gap = (wps.hdr.block_index() - wps.sample_index) as u32 as usize;
            if gap == 0 || gap > 262144 {
                break;
            }
            let n = gap.min(samples);
            wps.sample_index += n as i64;
            samples_unpacked += n;
            samples -= n;
            bptr += n * nch; // already zero
            continue;
        }
        let samples_to_unpack = ((wps.hdr.block_index() + wps.hdr.block_samples as i64
            - wps.sample_index) as u32 as usize)
            .min(samples);
        if !wps.init_done {
            wps.init_done = true;
            wps.unpack_init(&mut ctx, &block);
        }
        if wps.hdr.flags & FINAL_BLOCK == 0 {
            // ponytail: multi-stream layouts (refused, DECISIONS.md 13.3)
            return None;
        } else if wps.hdr.flags & MONO_FLAG == 0 && num_channels == 1 {
            // a stereo block in a mono file: zeros
            wps.sample_index += samples_to_unpack as i64;
        } else {
            wps.unpack_samples(&mut buffer[bptr..], samples_to_unpack);
        }
        if wps.wvx {
            return None;
        }
        bptr += samples_to_unpack * nch;
        samples_unpacked += samples_to_unpack;
        samples -= samples_to_unpack;
        if wps.sample_index == total_samples {
            break;
        }
    }
    if samples_unpacked == 0 {
        return None;
    }
    let mut out = Vec::new();
    out.try_reserve_exact(buffer.len()).ok()?;
    out.extend(buffer.iter().map(|&v| v as i16));
    drop(buffer);
    Some(Decoded {
        data: out,
        frames: num_samples,
        rate: ctx.sample_rate,
        channels: nch,
        loop_start: 0,
    })
}
