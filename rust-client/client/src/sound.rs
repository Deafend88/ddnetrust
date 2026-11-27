//! The sound engine: C++ `CSound` (engine/client/sound.cpp) without the device. Samples,
//! voices and channels, `Play`/`Stop`/the voice setters, `RateConvert`, `AdvanceVoice`
//! and the integer `Mix`, all with C++'s arithmetic. The device side (cpal) feeds `mix`
//! in SDL's fixed chunks (`audio_out.rs`). `dbg_assert`s on bad ids become no-ops.

use crate::wavpack::Decoded;
use ddnet::physics::vmath::{Vec2, length, vec2};

pub const NUM_SAMPLES: usize = 512;
pub const NUM_VOICES: usize = 256;
pub const NUM_CHANNELS: usize = 16;

pub const FLAG_LOOP: i32 = 1;
pub const FLAG_POS: i32 = 2;
pub const FLAG_NO_PANNING: i32 = 4;
pub const FLAG_PREVIEW: i32 = 8;
pub const SHAPE_CIRCLE: i32 = 0;
pub const SHAPE_RECTANGLE: i32 = 1;

const SAMPLE_INDEX_USED: i32 = -2;
const SAMPLE_INDEX_FULL: i32 = -1;

/// C++ float to int conversion as x86 does it (cvttss2si): out of range and NaN give
/// `INT_MIN`.
fn f2i(f: f32) -> i32 {
    if (-2147483648.0..2147483648.0).contains(&f) {
        f as i32
    } else {
        i32::MIN
    }
}

/// C++ `std::clamp` (NaN passes through).
fn clampf(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if hi < v {
        hi
    } else {
        v
    }
}

/// C++ `CSample`.
#[derive(Clone, Default)]
pub struct Sample {
    index: i32,
    next_free: i32,
    data: Option<Vec<i16>>,
    pub num_frames: i32,
    pub rate: i32,
    pub channels: i32,
    pub loop_start: i32,
    pub paused_at: i32,
}

impl Sample {
    fn is_loaded(&self) -> bool {
        self.data.is_some()
    }

    fn total_time(&self) -> f32 {
        self.num_frames as f32 / self.rate as f32
    }
}

#[derive(Clone, Copy)]
struct Channel {
    vol: i32,
    pan: i32,
}

/// C++ `CVoice`; `shape_a` is the union's `m_Circle.m_Radius`/`m_Rectangle.m_Width`.
#[derive(Clone, Copy, Default)]
struct Voice {
    sample: Option<usize>,
    channel: usize,
    age: i32,
    tick: i32,
    vol: i32,
    flags: i32,
    position: Vec2,
    falloff: f32,
    shape: i32,
    shape_a: f32,
    shape_b: f32,
}

/// C++ `ISound::CVoiceHandle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoiceHandle {
    pub id: i32,
    pub age: i32,
}

impl Default for VoiceHandle {
    fn default() -> Self {
        VoiceHandle { id: -1, age: -1 }
    }
}

impl VoiceHandle {
    pub fn is_valid(&self) -> bool {
        self.id >= 0 && self.age >= 0
    }
}

/// C++ `CSound`'s state.
pub struct Sound {
    samples: Vec<Sample>,
    first_free: i32,
    voices: Vec<Voice>,
    channels: [Channel; NUM_CHANNELS],
    next_voice: usize,
    /// C++ `m_MaxFrames`: `snd_buffer_size * 2`
    pub max_frames: u32,
    listener: Vec2,
    /// C++ `m_SoundVolume`: `snd_volume`, 0 while muted
    pub volume: i32,
    pub mixing_rate: i32,
    mix_buffer: Vec<i32>,
}

impl Sound {
    /// C++ `CSound::Init` without the device (the sample free list, `m_MaxFrames`).
    pub fn new(mixing_rate: i32, buffer_size: u32) -> Sound {
        let samples = (0..NUM_SAMPLES)
            .map(|i| Sample {
                index: i as i32,
                next_free: if i + 1 < NUM_SAMPLES {
                    i as i32 + 1
                } else {
                    SAMPLE_INDEX_FULL
                },
                ..Default::default()
            })
            .collect();
        let max_frames = buffer_size * 2;
        Sound {
            samples,
            first_free: 0,
            voices: vec![Voice::default(); NUM_VOICES],
            // C++ `= {{255, 0}}`: only channel 0 starts at full volume
            channels: std::array::from_fn(|i| Channel {
                vol: if i == 0 { 255 } else { 0 },
                pan: 0,
            }),
            next_voice: 0,
            max_frames,
            listener: vec2(0.0, 0.0),
            volume: 100,
            mixing_rate,
            mix_buffer: vec![0; max_frames as usize * 2],
        }
    }

    pub fn sample(&self, id: i32) -> Option<&Sample> {
        self.samples.get(usize::try_from(id).ok()?)
    }

    /// C++ `AllocSample`.
    fn alloc_sample(&mut self) -> Option<usize> {
        if self.first_free == SAMPLE_INDEX_FULL {
            return None;
        }
        let i = self.first_free as usize;
        self.first_free = self.samples[i].next_free;
        self.samples[i].next_free = SAMPLE_INDEX_USED;
        Some(i)
    }

    /// C++ `LoadWVFromMem`/`LoadOpusFromMem` after the decode: allocate, fill,
    /// `RateConvert`. -1 when every slot is taken.
    pub fn load(&mut self, d: Decoded) -> i32 {
        let Some(i) = self.alloc_sample() else {
            return -1;
        };
        let s = &mut self.samples[i];
        s.data = Some(d.data);
        s.num_frames = d.frames as i32;
        s.rate = d.rate as i32;
        s.channels = d.channels as i32;
        s.loop_start = d.loop_start as i32;
        s.paused_at = 0;
        let rate = self.mixing_rate;
        rate_convert(&mut self.samples[i], rate);
        i as i32
    }

    /// C++ `UnloadSample`.
    pub fn unload_sample(&mut self, id: i32) {
        let Ok(i) = usize::try_from(id) else {
            return;
        };
        if i >= NUM_SAMPLES {
            return;
        }
        if self.samples[i].is_loaded() {
            for v in &mut self.voices {
                if v.sample == Some(i) {
                    v.sample = None;
                }
            }
            self.samples[i].data = None;
        }
        if self.samples[i].next_free == SAMPLE_INDEX_USED {
            self.samples[i].next_free = self.first_free;
            self.first_free = self.samples[i].index;
        }
    }

    pub fn sample_total_time(&self, id: i32) -> f32 {
        self.sample(id).map_or(0.0, Sample::total_time)
    }

    /// C++ `GetSampleCurrentTime`.
    pub fn sample_current_time(&self, id: i32) -> f32 {
        let Some(s) = self.sample(id) else {
            return 0.0;
        };
        let i = id as usize;
        match self.voices.iter().find(|v| v.sample == Some(i)) {
            Some(v) => v.tick as f32 / s.rate as f32,
            None => s.paused_at as f32 / s.rate as f32,
        }
    }

    /// C++ `SetSampleCurrentTime`.
    pub fn set_sample_current_time(&mut self, id: i32, time: f32) {
        let Some(s) = self.sample(id) else {
            return;
        };
        let tick = f2i(s.num_frames as f32 * time);
        let i = id as usize;
        match self.voices.iter_mut().find(|v| v.sample == Some(i)) {
            Some(v) => v.tick = tick,
            None => self.samples[i].paused_at = tick,
        }
    }

    /// C++ `SetChannel`.
    pub fn set_channel(&mut self, id: usize, vol: f32, pan: f32) {
        if let Some(c) = self.channels.get_mut(id) {
            c.vol = f2i(vol * 255.0);
            c.pan = f2i(pan * 255.0);
        }
    }

    pub fn set_listener_position(&mut self, pos: Vec2) {
        self.listener = pos;
    }

    fn voice(&mut self, h: VoiceHandle) -> Option<&mut Voice> {
        if !h.is_valid() {
            return None;
        }
        let v = self.voices.get_mut(h.id as usize)?;
        (v.age == h.age).then_some(v)
    }

    /// C++ `SetVoiceVolume`.
    pub fn set_voice_volume(&mut self, h: VoiceHandle, volume: f32) {
        if let Some(v) = self.voice(h) {
            v.vol = f2i(clampf(volume, 0.0, 1.0) * 255.0);
        }
    }

    /// C++ `SetVoiceFalloff`.
    pub fn set_voice_falloff(&mut self, h: VoiceHandle, falloff: f32) {
        if let Some(v) = self.voice(h) {
            v.falloff = clampf(falloff, 0.0, 1.0);
        }
    }

    pub fn set_voice_position(&mut self, h: VoiceHandle, pos: Vec2) {
        if let Some(v) = self.voice(h) {
            v.position = pos;
        }
    }

    /// C++ `SetVoiceTimeOffset`.
    pub fn set_voice_time_offset(&mut self, h: VoiceHandle, time_offset: f32) {
        let max_frames = self.max_frames;
        let Some(&mut Voice {
            sample: Some(si),
            flags,
            ..
        }) = self.voice(h)
        else {
            return;
        };
        let s = &self.samples[si];
        let is_looping = flags & FLAG_LOOP != 0;
        // C++ float to uint64_t (negative offsets never reach here)
        let tick_offset = (s.rate as f32 * time_offset) as u64;
        let tick: i32 = if s.num_frames > 0 && is_looping {
            let (loop_start, num_frames) = (s.loop_start, s.num_frames);
            if tick_offset < num_frames as u64 {
                tick_offset as i32
            } else {
                let loop_length = num_frames - loop_start;
                if loop_length > 0 {
                    (loop_start as u64)
                        .wrapping_add((tick_offset - num_frames as u64) % loop_length as u64)
                        as i32
                } else {
                    loop_start
                }
            }
        } else {
            tick_offset.min(s.num_frames.max(0) as u64) as i32
        };
        let threshold = (0.2 * s.rate as f32).max(max_frames as f32);
        let num_frames = s.num_frames;
        let v = &mut self.voices[h.id as usize];
        if (v.tick.wrapping_sub(tick)).wrapping_abs() as f32 > threshold
            && !(is_looping
                && (v.tick.min(tick) + num_frames - v.tick.max(tick)) as f32 <= threshold)
        {
            v.tick = tick;
        }
    }

    /// C++ `SetVoiceCircle`.
    pub fn set_voice_circle(&mut self, h: VoiceHandle, radius: f32) {
        if let Some(v) = self.voice(h) {
            v.shape = SHAPE_CIRCLE;
            v.shape_a = 0f32.max(radius);
        }
    }

    /// C++ `SetVoiceRectangle`.
    pub fn set_voice_rectangle(&mut self, h: VoiceHandle, width: f32, height: f32) {
        if let Some(v) = self.voice(h) {
            v.shape = SHAPE_RECTANGLE;
            v.shape_a = 0f32.max(width);
            v.shape_b = 0f32.max(height);
        }
    }

    /// C++ `Play(ChannelId, SampleId, Flags, Volume, Position)`; `PlayAt` adds `FLAG_POS`.
    pub fn play(
        &mut self,
        channel: usize,
        sample: i32,
        flags: i32,
        volume: f32,
        pos: Vec2,
    ) -> VoiceHandle {
        let Ok(si) = usize::try_from(sample) else {
            return VoiceHandle::default();
        };
        if channel >= NUM_CHANNELS || si >= NUM_SAMPLES {
            return VoiceHandle::default();
        }
        let Some(id) = (0..NUM_VOICES)
            .map(|i| (self.next_voice + i) % NUM_VOICES)
            .find(|&i| self.voices[i].sample.is_none())
        else {
            return VoiceHandle { id: -1, age: -1 };
        };
        self.next_voice = id + 1;
        let tick = if flags & FLAG_LOOP != 0 {
            self.samples[si].paused_at
        } else if flags & FLAG_PREVIEW != 0 {
            let t = self.samples[si].paused_at;
            self.samples[si].paused_at = 0;
            t
        } else {
            0
        };
        let v = &mut self.voices[id];
        v.sample = Some(si);
        v.channel = channel;
        v.tick = tick;
        v.vol = f2i(clampf(volume, 0.0, 1.0) * 255.0);
        v.flags = flags;
        v.position = pos;
        v.falloff = 0.0;
        v.shape = SHAPE_CIRCLE;
        v.shape_a = 1500.0;
        VoiceHandle {
            id: id as i32,
            age: v.age,
        }
    }

    /// C++ `PlayAt`.
    pub fn play_at(
        &mut self,
        channel: usize,
        sample: i32,
        flags: i32,
        volume: f32,
        pos: Vec2,
    ) -> VoiceHandle {
        self.play(channel, sample, flags | FLAG_POS, volume, pos)
    }

    /// C++ `Pause`.
    pub fn pause(&mut self, sample: i32) {
        let Ok(si) = usize::try_from(sample) else {
            return;
        };
        for v in &mut self.voices {
            if v.sample == Some(si) {
                self.samples[si].paused_at = v.tick;
                v.sample = None;
            }
        }
    }

    /// C++ `Stop`.
    pub fn stop(&mut self, sample: i32) {
        let Ok(si) = usize::try_from(sample) else {
            return;
        };
        for v in &mut self.voices {
            if v.sample == Some(si) {
                self.samples[si].paused_at = if v.flags & FLAG_LOOP != 0 { v.tick } else { 0 };
                v.sample = None;
            }
        }
    }

    /// C++ `StopAll`.
    pub fn stop_all(&mut self) {
        for v in &mut self.voices {
            if let Some(si) = v.sample {
                self.samples[si].paused_at = if v.flags & FLAG_LOOP != 0 { v.tick } else { 0 };
            }
            v.sample = None;
        }
    }

    /// C++ `StopVoice`.
    pub fn stop_voice(&mut self, h: VoiceHandle) {
        if let Some(v) = self.voice(h) {
            v.sample = None;
            v.age = v.age.wrapping_add(1);
        }
    }

    /// C++ `IsPlaying`.
    pub fn is_playing(&self, sample: i32) -> bool {
        usize::try_from(sample).is_ok_and(|si| self.voices.iter().any(|v| v.sample == Some(si)))
    }

    /// C++ `AdvanceVoice`.
    fn advance_voice(samples: &[Sample], v: &mut Voice, frames: u32) -> u32 {
        let s = &samples[v.sample.unwrap_or_default()];
        let advanced = frames.min(s.num_frames.wrapping_sub(v.tick) as u32);
        v.tick = v.tick.wrapping_add(advanced as i32);
        if v.tick == s.num_frames {
            if v.flags & FLAG_LOOP != 0 {
                v.tick = s.loop_start;
            } else {
                v.sample = None;
                v.age = v.age.wrapping_add(1);
            }
        }
        advanced
    }

    /// C++ `Mix`: interleaved stereo into `out` (`frames` frames, at most `m_MaxFrames`).
    pub fn mix(&mut self, out: &mut [i16], frames: u32) {
        let frames = frames.min(self.max_frames) as usize;
        let buf = &mut self.mix_buffer[..frames * 2];
        buf.fill(0);
        let master_vol = self.volume;
        for v in self.voices.iter_mut() {
            let Some(si) = v.sample else {
                continue;
            };
            let s = &self.samples[si];
            let step = s.channels.max(1) as usize;
            let start = v.tick.max(0) as usize * step;
            let right = if s.channels == 1 { 0 } else { 1 };
            let end = Self::advance_voice(&self.samples, v, frames as u32) as usize;
            let s = &self.samples[si];
            let ch = self.channels[v.channel];
            let mut volume_r = f2i(ch.vol as f32 * (v.vol as f32 / 255.0));
            let mut volume_l = volume_r;
            if v.flags & FLAG_POS != 0 && ch.pan != 0 {
                let delta = v.position - self.listener;
                let (mut fx, mut fy) = (0.0f32, 0.0f32);
                let mut range_x = 0.0f32;
                let mut in_field = false;
                if v.shape == SHAPE_CIRCLE {
                    let radius = v.shape_a;
                    range_x = radius;
                    let dist = length(delta);
                    if dist < radius {
                        in_field = true;
                        let falloff_distance = radius * v.falloff;
                        fx = if dist > falloff_distance {
                            (radius - dist) / (radius - falloff_distance)
                        } else {
                            1.0
                        };
                        fy = fx;
                    }
                } else if v.shape == SHAPE_RECTANGLE {
                    let (ax, ay) = (delta.x.abs(), delta.y.abs());
                    let w = v.shape_a / 2.0;
                    let h = v.shape_b / 2.0;
                    range_x = w;
                    if ax < w && ay < h {
                        in_field = true;
                        let (fdx, fdy) = (w * v.falloff, h * v.falloff);
                        fx = if ax > fdx { (w - ax) / (w - fdx) } else { 1.0 };
                        fy = if ay > fdy { (h - ay) / (h - fdy) } else { 1.0 };
                    }
                }
                if in_field {
                    if v.flags & FLAG_NO_PANNING == 0 {
                        if delta.x > 0.0 {
                            volume_l = f2i(((range_x - delta.x.abs()) * volume_l as f32) / range_x);
                        } else {
                            volume_r = f2i(((range_x - delta.x.abs()) * volume_r as f32) / range_x);
                        }
                    }
                    volume_l = f2i(volume_l as f32 * (fx * fy));
                    volume_r = f2i(volume_r as f32 * (fx * fy));
                } else {
                    volume_l = 0;
                    volume_r = 0;
                }
            }
            let data = s.data.as_deref().unwrap_or(&[]);
            for (k, o) in buf.as_chunks_mut::<2>().0[..end].iter_mut().enumerate() {
                let at = start + k * step;
                let l = *data.get(at).unwrap_or(&0) as i32;
                let r = *data.get(at + right).unwrap_or(&0) as i32;
                o[0] = o[0].wrapping_add(l.wrapping_mul(volume_l));
                o[1] = o[1].wrapping_add(r.wrapping_mul(volume_r));
            }
        }
        for (o, &m) in out.iter_mut().zip(buf.iter()) {
            *o = ((m.wrapping_mul(master_vol) / 101) >> 8).clamp(i16::MIN as i32, i16::MAX as i32)
                as i16;
        }
    }
}

/// C++ `CSound::RateConvert`.
fn rate_convert(s: &mut Sample, mixing_rate: i32) {
    if s.rate == mixing_rate {
        return;
    }
    let num_frames = f2i((s.num_frames as f32 / s.rate as f32) * mixing_rate as f32);
    let old = s.data.take().unwrap_or_default();
    let ch = s.channels as usize;
    let mut new = vec![0i16; num_frames.max(0) as usize * ch];
    for i in 0..num_frames.max(0) as usize {
        let a = i as f32 / num_frames as f32;
        let mut f = f2i(a * s.num_frames as f32);
        if f >= s.num_frames {
            f = s.num_frames - 1;
        }
        let f = f.max(0) as usize;
        if ch == 1 {
            new[i] = *old.get(f).unwrap_or(&0);
        } else if ch == 2 {
            new[i * 2] = *old.get(f * 2).unwrap_or(&0);
            new[i * 2 + 1] = *old.get(f * 2 + 1).unwrap_or(&0);
        }
    }
    let factor = mixing_rate as f64 / s.rate as f64;
    s.loop_start = (s.loop_start as f64 * factor).round() as i32;
    s.data = Some(new);
    s.num_frames = num_frames;
    s.rate = mixing_rate;
}
