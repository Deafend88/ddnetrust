//! Ogg Opus decoding as C++ `CSound::DecodeOpus` (engine/client/sound.cpp:464) does it with
//! opusfile 0.12 and libopus 1.3.1 (float builds): `op_open_memory`, `op_read` to 16 bits
//! until the end, the `LOOP_START=` tag. The Ogg pages, the Opus decoder and the pre-skip
//! and end trimming come from `opus-pure`; on top of that this ports what `op_read` does
//! to the float output: libopus `opus_pcm_soft_clip` per decoded packet and opusfile's
//! `op_float2short_filter` (triangular dither with IIR noise shaping, seeded with the
//! stream's serial number). A float decoder on x86 cannot be bit-exact with libopus, so
//! samples may differ by a few steps (DECISIONS.md 13.3); lengths, channels and loop
//! starts are exact.

use crate::wavpack::Decoded;
use opus_pure::{OggOpusReader, Trim};

/// C: opusfile.c `op_rand`.
fn op_rand(seed: u32) -> u32 {
    seed.wrapping_mul(96314165).wrapping_add(907633515)
}

const OP_GAIN: f32 = 32753.0;
const OP_PRNG_GAIN: f32 = 1.0 / 0xFFFF_FFFFu32 as f32;
const OP_FCOEF_B: [f32; 4] = [2.2374, -0.7339, -0.1251, -0.6033];
const OP_FCOEF_A: [f32; 4] = [0.9030, 0.0116, -0.5853, -0.2571];

/// opusfile's dither state (`dither_a/b`, `dither_mute`, `dither_seed`).
struct Dither {
    a: [[f32; 4]; 2],
    b: [[f32; 4]; 2],
    mute: i32,
    seed: u32,
    first: bool,
}

/// C: opusfile.c `op_float2short_filter` with `OP_SOFT_CLIP` (the dither is on).
fn float2short(
    d: &mut Dither,
    clip: &mut [f32; 2],
    src: &mut [f32],
    ch: usize,
    out: &mut Vec<i16>,
) {
    if d.first {
        *clip = [0.0; 2];
    }
    soft_clip(src, ch, clip);
    let mut mute = if d.first { 65 } else { d.mute };
    if mute > 64 {
        d.a = [[0.0; 4]; 2];
    }
    for frame in src.chunks_exact(ch) {
        let mut silent = true;
        for (ci, &x) in frame.iter().enumerate() {
            silent &= x == 0.0;
            let mut s = x * OP_GAIN;
            let mut err = 0.0f32;
            for j in 0..4 {
                err += OP_FCOEF_B[j] * d.b[ci][j] - OP_FCOEF_A[j] * d.a[ci][j];
            }
            d.a[ci].copy_within(0..3, 1);
            d.b[ci].copy_within(0..3, 1);
            d.a[ci][0] = err;
            s -= err;
            let r = if mute > 16 {
                0.0
            } else {
                d.seed = op_rand(d.seed);
                let r = d.seed as f32 * OP_PRNG_GAIN;
                d.seed = op_rand(d.seed);
                r - d.seed as f32 * OP_PRNG_GAIN
            };
            // op_float2int is lrintf (round half to even)
            let si = (s + r).clamp(-32768.0, 32767.0).round_ties_even() as i32;
            out.push(si as i16);
            d.b[ci][0] = if mute > 16 {
                0.0
            } else {
                (si as f32 - s).clamp(-1.5, 1.5)
            };
        }
        mute += 1;
        if !silent {
            mute = 0;
        }
    }
    d.mute = mute.min(65);
    d.first = false;
}

/// C: libopus 1.3.1 opus.c `opus_pcm_soft_clip`, on interleaved `x` of `c` channels.
fn soft_clip(x: &mut [f32], c: usize, declip_mem: &mut [f32; 2]) {
    let n = x.len() / c;
    if n < 1 {
        return;
    }
    for v in x.iter_mut() {
        *v = v.clamp(-2.0, 2.0);
    }
    for (ch, mem) in declip_mem.iter_mut().enumerate().take(c) {
        let at = |i: usize| i * c + ch;
        let mut a = *mem;
        for i in 0..n {
            if x[at(i)] * a >= 0.0 {
                break;
            }
            x[at(i)] += a * x[at(i)] * x[at(i)];
        }
        let mut curr = 0;
        let x0 = x[at(0)];
        loop {
            let Some(i) = (curr..n).find(|&i| x[at(i)] > 1.0 || x[at(i)] < -1.0) else {
                a = 0.0;
                break;
            };
            let mut peak_pos = i;
            let (mut start, mut end) = (i, i);
            let mut maxval = x[at(i)].abs();
            while start > 0 && x[at(i)] * x[at(start - 1)] >= 0.0 {
                start -= 1;
            }
            while end < n && x[at(i)] * x[at(end)] >= 0.0 {
                if x[at(end)].abs() > maxval {
                    maxval = x[at(end)].abs();
                    peak_pos = end;
                }
                end += 1;
            }
            let special = start == 0 && x[at(i)] * x[at(0)] >= 0.0;
            a = (maxval - 1.0) / (maxval * maxval);
            a += a * 2.4e-7;
            if x[at(i)] > 0.0 {
                a = -a;
            }
            for k in start..end {
                x[at(k)] += a * x[at(k)] * x[at(k)];
            }
            if special && peak_pos >= 2 {
                let mut offset = x0 - x[at(0)];
                let delta = offset / peak_pos as f32;
                for k in curr..peak_pos {
                    offset -= delta;
                    x[at(k)] += offset;
                    x[at(k)] = x[at(k)].clamp(-1.0, 1.0);
                }
            }
            curr = end;
            if curr == n {
                break;
            }
        }
        *mem = a;
    }
}

/// C++ `CSound::DecodeOpus`: `None` where C++ logs an error and returns false. Map sounds
/// come from servers, so a panic inside the third-party decoder is a failed decode too.
/// A damaged Ogg page fails the whole sound (opusfile skips the page and plays on).
pub fn decode(data: &[u8]) -> Option<Decoded> {
    std::panic::catch_unwind(|| decode_inner(data))
        .ok()
        .flatten()
}

fn decode_inner(data: &[u8]) -> Option<Decoded> {
    let mut reader = OggOpusReader::new(data).ok()?;
    let head = reader.head().clone();
    let ch = head.channel_count as usize;
    if ch > 2 {
        return None;
    }
    let mut decoder = head.decoder(48000).ok()?;
    let mut trim = Trim::new(&head, 48000, ch).ok()?;
    let mut dither = Dither {
        a: [[0.0; 4]; 2],
        b: [[0.0; 4]; 2],
        mute: 0,
        seed: reader.serial(),
        first: true,
    };
    let mut clip = [0.0f32; 2];
    let mut pcm = vec![0f32; 5760 * ch];
    let mut out = Vec::new();
    while let Some(packet) = reader.read_packet().ok()? {
        let n = decoder.decode(&packet.data, 5760, &mut pcm).ok()?;
        let range = trim.keep_range(&packet, n * ch);
        if !range.is_empty() {
            float2short(&mut dither, &mut clip, &mut pcm[range], ch, &mut out);
        }
    }
    let frames = out.len() / ch;
    // C++: the first LOOP_START= comment decides; bad or out of range values keep 0
    let loop_start = reader
        .tags()
        .comments
        .iter()
        .find_map(|c| c.strip_prefix("LOOP_START="))
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|&v| v >= 0 && (v as usize) < frames)
        .unwrap_or(0) as usize;
    Some(Decoded {
        data: out,
        frames,
        rate: 48000,
        channels: ch,
        loop_start,
    })
}
