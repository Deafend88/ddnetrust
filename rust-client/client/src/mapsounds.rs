//! C++ `CMapSounds` (game/client/components/mapsounds.cpp): the map's sounds (embedded or
//! `mapres/<name>.opus`), the sources of its sound layers, and per frame their voices:
//! started from the round's time and each source's delay, kept in step with
//! `SetVoiceTimeOffset`, placed by the position envelope and the group's parallax, scaled
//! by the sound envelope.

use crate::config::Config;
use crate::opus;
use crate::render::envelope::Envelopes;
use crate::sound::{FLAG_LOOP, FLAG_NO_PANNING, VoiceHandle};
use crate::sounds::{CHN_MAPSOUND, Sounds};
use crate::storage::Storage;
use ddnet::map::Map;
use ddnet::physics::vmath::{Vec2, vec2};
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering::Relaxed;

const MAPITEMTYPE_GROUP: i32 = 4;
const MAPITEMTYPE_LAYER: i32 = 5;
const MAPITEMTYPE_SOUND: i32 = 7;
const LAYERTYPE_SOUNDS: i32 = 10;
const LAYERFLAG_DETAIL: i32 = 1;
const MAX_MAPSOUNDS: usize = 64;
/// C++ `sizeof(CSoundSource)`: 13 ints (position 2, 8 ints, shape: type and a 2-int union).
const SOURCE_INTS: usize = 13;

fn fx2f(v: i32) -> f32 {
    v as f32 * (1.0 / 1024.0)
}

/// C++ `CSoundSource`.
#[derive(Clone, Copy, Debug)]
struct Source {
    pos: (i32, i32),
    looped: bool,
    pan: bool,
    time_delay: i32,
    falloff: i32,
    pos_env: i32,
    pos_env_offset: i32,
    sound_env: i32,
    sound_env_offset: i32,
    /// C++ `CSoundShape`: type 0 rectangle (w, h in 22.10), 1 circle (radius)
    shape: (i32, i32, i32),
}

/// C++ `CSourceQueueEntry` with its group's offset and parallax.
struct Entry {
    sound: usize,
    high_detail: bool,
    offset: (i32, i32),
    parallax: (i32, i32),
    source: Source,
    voice: VoiceHandle,
}

pub struct MapSounds {
    /// C++ `m_aSounds`: engine sample ids, -1 where loading failed.
    pub ids: Vec<i32>,
    queue: Vec<Entry>,
    envelopes: Envelopes,
}

/// C++ `static float s_Time` of `OnRender` (f32 bits): kept across maps, like the static.
static TIME: AtomicU32 = AtomicU32::new(0);

impl MapSounds {
    /// C++ `OnMapLoad`. `warning` is set when some sounds failed (C++ shows a warning).
    pub fn load(map: &Map, storage: &Storage, sounds: &mut Sounds, warning: &mut bool) -> Self {
        let df = &map.datafile;
        let mut ids = Vec::new();
        if sounds.enabled() {
            let range = df.get_type(MAPITEMTYPE_SOUND);
            for i in range.take(MAX_MAPSOUNDS) {
                let d = &df.items[i].data;
                let (external, name, data) = (
                    d.get(1).copied().unwrap_or(0),
                    d.get(2).copied().unwrap_or(-1),
                    d.get(3).copied().unwrap_or(-1),
                );
                let name = df
                    .data(name)
                    .map(|b| {
                        String::from_utf8_lossy(b.split(|&c| c == 0).next().unwrap_or(&[]))
                            .into_owned()
                    })
                    .filter(|n| !n.is_empty());
                let bytes = if external != 0 {
                    name.and_then(|n| storage.read(&format!("mapres/{n}.opus")))
                } else {
                    df.data(data)
                };
                let id = bytes
                    .and_then(|b| opus::decode(&b))
                    .map_or(-1, |s| sounds.engine.lock().unwrap().load(s));
                *warning |= id == -1;
                ids.push(id);
            }
        }
        // enqueue sound sources
        let layers = df.get_type(MAPITEMTYPE_LAYER);
        let mut queue = Vec::new();
        for g in df.get_type(MAPITEMTYPE_GROUP) {
            let gd = &df.items[g].data;
            let at = |i: usize| gd.get(i).copied().unwrap_or(0);
            let (offset, parallax) = ((at(1), at(2)), (at(3), at(4)));
            let (start, num) = (at(5), at(6));
            for l in 0..num.max(0) {
                let Some(index) = usize::try_from(start + l).ok().map(|i| layers.start + i) else {
                    continue;
                };
                if index >= layers.end {
                    continue;
                }
                let ld = &df.items[index].data;
                let li = |i: usize| ld.get(i).copied().unwrap_or(0);
                if li(1) != LAYERTYPE_SOUNDS || !(1..=2).contains(&li(3)) {
                    continue;
                }
                let sound = li(6);
                let Some(sound) = usize::try_from(sound)
                    .ok()
                    .filter(|&s| ids.get(s).is_some_and(|&id| id != -1))
                else {
                    continue;
                };
                let Some(data) = df.data(li(5)) else {
                    continue;
                };
                let ints: Vec<i32> = data
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| i32::from_le_bytes(*b))
                    .collect();
                let n = (li(4).max(0) as usize).min(ints.len() / SOURCE_INTS);
                for s in ints.as_chunks::<SOURCE_INTS>().0.iter().take(n) {
                    queue.push(Entry {
                        sound,
                        high_detail: li(2) & LAYERFLAG_DETAIL != 0,
                        offset,
                        parallax,
                        source: Source {
                            pos: (s[0], s[1]),
                            looped: s[2] != 0,
                            pan: s[3] != 0,
                            time_delay: s[4],
                            falloff: s[5],
                            pos_env: s[6],
                            pos_env_offset: s[7],
                            sound_env: s[8],
                            sound_env_offset: s[9],
                            shape: (s[10], s[11], s[12]),
                        },
                        voice: VoiceHandle::default(),
                    });
                }
            }
        }
        MapSounds {
            ids,
            queue,
            envelopes: Envelopes::from_map(map),
        }
    }

    /// The sources' voices (C++ `m_vSourceQueue[i].m_Voice`).
    pub fn voices(&self) -> impl Iterator<Item = VoiceHandle> + '_ {
        self.queue.iter().map(|e| e.voice)
    }

    /// C++ `Play`/`PlayAt` (Sv_MapSoundGlobal, NETEVENTTYPE_MAPSOUNDWORLD).
    pub fn play(
        &self,
        sounds: &mut Sounds,
        channel: usize,
        id: i32,
        pos: Option<Vec2>,
        cfg: &Config,
    ) {
        if cfg.snd_game == 0 || cfg.snd_enable == 0 {
            return;
        }
        if let Some(&sample) = usize::try_from(id).ok().and_then(|i| self.ids.get(i)) {
            sounds.play_sample(channel, sample, 0, 1.0, pos, cfg);
        }
    }

    /// C++ `OnRender`. `ticks`: (prev game tick, game tick, intra tick, round start tick if
    /// the snapshot has a game info); `center` the camera; `envelope_time` the map
    /// layers' envelope clock.
    pub fn on_render(
        &mut self,
        sounds: &mut Sounds,
        ticks: (i32, i32, f32, Option<i32>),
        center: Vec2,
        envelope_time: i64,
        cfg: &Config,
    ) {
        let (prev, cur, intra, round_start) = ticks;
        for i in 0..self.queue.len() {
            if let Some(start) = round_start {
                let a = (prev - start) as f32 / 50.0;
                let b = (cur - start) as f32 / 50.0;
                TIME.store((a + (b - a) * intra).to_bits(), Relaxed);
            }
            let e = &self.queue[i];
            let s = e.source;
            let offset = f32::from_bits(TIME.load(Relaxed)) - s.time_delay as f32;
            if offset >= 0.0 && cfg.snd_enable != 0 && (cfg.gfx_high_detail != 0 || !e.high_detail)
            {
                let voice = e.voice;
                let mut engine = sounds.engine.lock().unwrap();
                if voice.is_valid() {
                    // currently playing, set offset
                    engine.set_voice_time_offset(voice, offset);
                } else {
                    drop(engine);
                    let mut flags = 0;
                    if s.looped {
                        flags |= FLAG_LOOP;
                    }
                    if !s.pan {
                        flags |= FLAG_NO_PANNING;
                    }
                    let pos = vec2(fx2f(s.pos.0), fx2f(s.pos.1));
                    let v = sounds.play_sample(
                        CHN_MAPSOUND,
                        self.ids[e.sound],
                        flags,
                        1.0,
                        Some(pos),
                        cfg,
                    );
                    let mut engine = sounds.engine.lock().unwrap();
                    engine.set_voice_time_offset(v, offset);
                    engine.set_voice_falloff(v, s.falloff as f32 / 255.0);
                    match s.shape.0 {
                        1 => engine.set_voice_circle(v, s.shape.1 as f32),
                        0 => engine.set_voice_rectangle(v, fx2f(s.shape.1), fx2f(s.shape.2)),
                        _ => {}
                    }
                    self.queue[i].voice = v;
                }
            } else {
                // stop voice
                sounds.engine.lock().unwrap().stop_voice(e.voice);
                self.queue[i].voice = VoiceHandle::default();
            }
        }
        let mut engine = sounds.engine.lock().unwrap();
        for e in &self.queue {
            if !e.voice.is_valid() {
                continue;
            }
            let s = e.source;
            let mut pos = [0.0f32; 4];
            self.envelopes
                .eval(envelope_time, s.pos_env_offset, s.pos_env, &mut pos, 2);
            let mut x = fx2f(s.pos.0) + pos[0];
            let mut y = fx2f(s.pos.1) + pos[1];
            x += center.x * (1.0 - e.parallax.0 as f32 / 100.0);
            y += center.y * (1.0 - e.parallax.1 as f32 / 100.0);
            x -= e.offset.0 as f32;
            y -= e.offset.1 as f32;
            engine.set_voice_position(e.voice, vec2(x, y));
            let mut volume = [1.0f32, 0.0, 0.0, 0.0];
            self.envelopes.eval(
                envelope_time,
                s.sound_env_offset,
                s.sound_env,
                &mut volume,
                1,
            );
            engine.set_voice_volume(e.voice, volume[0].clamp(0.0, 1.0));
        }
    }

    /// C++ `Clear`: unload every sample.
    pub fn clear(self, sounds: &mut Sounds) {
        let mut engine = sounds.engine.lock().unwrap();
        for id in self.ids {
            engine.unload_sample(id);
        }
    }
}
