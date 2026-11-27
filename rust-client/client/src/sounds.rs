//! C++ `CSounds` (game/client/components/sounds.cpp) and the audio device of `CSound`
//! (engine/client/sound.cpp, SDL there, cpal here). The sound sets load on a thread like
//! `cl_threadsoundloading`, through the storage order (files in the user dir's `audio/`
//! replace DDNet's, PLAN §13.13 answer 2). The device pulls `CSound::Mix` in SDL's fixed
//! chunks of `snd_buffer_size` frames, so voices end and loop at the same frames as in C++.

use crate::config::Config;
use crate::render::content::SOUNDS;
use crate::sound::{FLAG_LOOP, Sound, VoiceHandle};
use crate::storage::Storage;
use crate::wavpack::{self, Decoded};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ddnet::physics::vmath::Vec2;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const CHN_GUI: usize = 0;
pub const CHN_MUSIC: usize = 1;
pub const CHN_WORLD: usize = 2;
pub const CHN_GLOBAL: usize = 3;
pub const CHN_MAPSOUND: usize = 4;

const QUEUE_SIZE: usize = 32;

type Loaded = Vec<Vec<Option<Decoded>>>;

pub struct Sounds {
    pub engine: Arc<Mutex<Sound>>,
    /// The device stream; `None` without a device (C++ keeps running silently too).
    stream: Option<cpal::Stream>,
    /// C++ `m_pSoundJob`/`m_WaitForSoundJob`: the decoding thread.
    job: Option<JoinHandle<Loaded>>,
    /// C++ `g_pData->m_aSounds[s].m_aSounds[i].m_Id` and `m_Last`.
    ids: Vec<Vec<i32>>,
    last: Vec<i32>,
    /// C++ `m_GuiSoundVolume`, `m_GameSoundVolume`, `m_MapSoundVolume`,
    /// `m_BackgroundMusicVolume`.
    volumes: [f32; 4],
    queue: Vec<(usize, i32)>,
    queue_wait: Instant,
    seed: u32,
    /// C++ `CEffects::m_SkidSoundTimer`.
    skid_timer: Option<Instant>,
}

impl Sounds {
    /// C++ `CSound::Init` (device) and `CSounds::OnInit` (threaded loading). Without
    /// `snd_enable` nothing loads and nothing plays, as in C++.
    pub fn new(storage: &Storage, cfg: &Config) -> Sounds {
        let (rate, stream, engine) = if cfg.snd_enable != 0 {
            open_device(cfg.snd_buffer_size.max(128) as u32)
        } else {
            let e = Arc::new(Mutex::new(Sound::new(
                cfg.snd_rate,
                cfg.snd_buffer_size as u32,
            )));
            (0, None, e)
        };
        let job = (rate != 0).then(|| {
            let storage = storage.clone();
            std::thread::spawn(move || {
                SOUNDS
                    .iter()
                    .map(|set| {
                        set.iter()
                            .map(|f| storage.read(f).and_then(|d| wavpack::decode(&d)))
                            .collect()
                    })
                    .collect()
            })
        });
        Sounds {
            engine,
            stream,
            job,
            ids: SOUNDS.iter().map(|s| vec![-1; s.len()]).collect(),
            last: vec![-1; SOUNDS.len()],
            volumes: [-1.0; 4],
            queue: Vec::new(),
            queue_wait: Instant::now(),
            seed: std::process::id().wrapping_mul(2_654_435_761),
            skid_timer: None,
        }
    }

    fn waiting(&mut self) -> bool {
        if self.job.as_ref().is_some_and(|j| j.is_finished()) {
            let loaded = self.job.take().unwrap().join().unwrap_or_default();
            let mut e = self.engine.lock().unwrap();
            for (set, files) in loaded.into_iter().enumerate() {
                for (i, d) in files.into_iter().enumerate() {
                    self.ids[set][i] = d.map_or(-1, |d| e.load(d));
                }
            }
            let ok = self.ids.iter().flatten().filter(|&&id| id != -1).count();
            let all = self.ids.iter().flatten().count();
            eprintln!(
                "sound: {ok} of {all} samples loaded, mixing at {} Hz",
                e.mixing_rate
            );
        }
        self.job.is_some()
    }

    pub fn enabled(&self) -> bool {
        self.stream.is_some()
    }

    /// C++ `rand()`; any generator will do, the pick is random in C++ too.
    fn rand(&mut self) -> u32 {
        self.seed = self.seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        self.seed >> 16
    }

    /// C++ `GetSampleId`: a random sample of the set, never the last one.
    fn sample_id(&mut self, set: i32, cfg: &Config) -> i32 {
        if cfg.snd_enable == 0 || !self.enabled() || self.waiting() {
            return -1;
        }
        let Some(ids) = usize::try_from(set).ok().and_then(|s| self.ids.get(s)) else {
            return -1;
        };
        match ids.len() {
            0 => -1,
            1 => ids[0],
            n => {
                let set = set as usize;
                let id = loop {
                    let id = (self.rand() as usize % n) as i32;
                    if id != self.last[set] {
                        break id;
                    }
                };
                self.last[set] = id;
                self.ids[set][id as usize]
            }
        }
    }

    /// C++ `UpdateChannels`.
    fn update_channels(&mut self, cfg: &Config) {
        let mut e = self.engine.lock().unwrap();
        let wanted = [
            cfg.snd_chat_volume,
            cfg.snd_game_volume,
            cfg.snd_ambient_volume,
            cfg.snd_background_music_volume,
        ]
        .map(|v| v as f32 / 100.0);
        if wanted[0] != self.volumes[0] {
            e.set_channel(CHN_GUI, wanted[0], 0.0);
        }
        if wanted[1] != self.volumes[1] {
            e.set_channel(CHN_WORLD, 0.9 * wanted[1], 1.0);
            e.set_channel(CHN_GLOBAL, wanted[1], 0.0);
        }
        if wanted[2] != self.volumes[2] {
            e.set_channel(CHN_MAPSOUND, wanted[2], 1.0);
        }
        if wanted[3] != self.volumes[3] {
            e.set_channel(CHN_MUSIC, wanted[3], 1.0);
        }
        self.volumes = wanted;
    }

    /// C++ `CSounds::OnRender` and `CSound::Update`'s volume: listener at the camera,
    /// channel volumes, the queue (one sound per 300 ms).
    pub fn on_render(&mut self, listener: Vec2, window_active: bool, cfg: &Config) {
        if self.waiting() {
            return;
        }
        {
            let mut e = self.engine.lock().unwrap();
            e.volume = if !window_active && cfg.snd_nonactive_mute != 0 {
                0
            } else {
                cfg.snd_volume
            };
            e.set_listener_position(listener);
        }
        self.update_channels(cfg);
        let now = Instant::now();
        if !self.queue.is_empty() && self.queue_wait <= now {
            let (channel, set) = self.queue.remove(0);
            self.play(channel, set, 1.0, cfg);
            self.queue_wait = now + Duration::from_millis(300);
        }
    }

    /// C++ `OnReset` while online: stop everything, clear the queue.
    pub fn reset(&mut self) {
        self.engine.lock().unwrap().stop_all();
        self.queue.clear();
        self.queue_wait = Instant::now();
    }

    /// C++ `Enqueue`.
    pub fn enqueue(&mut self, channel: usize, set: i32) {
        if self.queue.len() < QUEUE_SIZE {
            self.queue.push((channel, set));
        }
    }

    /// C++ `Play`.
    pub fn play(&mut self, channel: usize, set: i32, volume: f32, cfg: &Config) {
        let id = self.sample_id(set, cfg);
        self.play_sample(channel, id, 0, volume, None, cfg);
    }

    /// C++ `PlayAt`.
    pub fn play_at(&mut self, channel: usize, set: i32, volume: f32, pos: Vec2, cfg: &Config) {
        let id = self.sample_id(set, cfg);
        self.play_sample(channel, id, 0, volume, Some(pos), cfg);
    }

    /// C++ `PlaySample`/`PlaySampleAt`.
    pub fn play_sample(
        &mut self,
        channel: usize,
        id: i32,
        mut flags: i32,
        volume: f32,
        pos: Option<Vec2>,
        cfg: &Config,
    ) -> VoiceHandle {
        if (channel == CHN_MUSIC && cfg.snd_enable_music == 0) || id == -1 {
            return VoiceHandle::default();
        }
        if channel == CHN_MUSIC {
            flags |= FLAG_LOOP;
        }
        let mut e = self.engine.lock().unwrap();
        match pos {
            Some(p) => e.play_at(channel, id, flags, volume, p),
            None => e.play(
                channel,
                id,
                flags,
                volume,
                ddnet::physics::vmath::vec2(0.0, 0.0),
            ),
        }
    }

    /// C++ `Stop`.
    pub fn stop(&mut self, set: i32) {
        if self.waiting() {
            return;
        }
        if let Some(ids) = usize::try_from(set).ok().and_then(|s| self.ids.get(s)) {
            let mut e = self.engine.lock().unwrap();
            ids.iter()
                .filter(|&&id| id != -1)
                .for_each(|&id| e.stop(id));
        }
    }

    /// C++ `IsPlaying`.
    pub fn is_playing(&mut self, set: i32) -> bool {
        if self.waiting() {
            return false;
        }
        let e = self.engine.lock().unwrap();
        usize::try_from(set)
            .ok()
            .and_then(|s| self.ids.get(s))
            .is_some_and(|ids| ids.iter().any(|&id| id != -1 && e.is_playing(id)))
    }

    /// C++ `CEffects::SkidTrail`'s sound: at most every 100 ms.
    pub fn skid(&mut self, pos: Vec2, volume: f32, cfg: &Config) {
        if cfg.snd_game == 0 {
            return;
        }
        let now = Instant::now();
        if self
            .skid_timer
            .is_none_or(|t| now - t > Duration::from_millis(100))
        {
            self.skid_timer = Some(now);
            self.play_at(
                CHN_WORLD,
                ddnet::protocol::sounds::SOUND_PLAYER_SKID,
                volume,
                pos,
                cfg,
            );
        }
    }
}

/// Opens the default output device and its stream; the engine mixes at the device's rate
/// (SDL is opened with `SDL_AUDIO_ALLOW_FREQUENCY_CHANGE`). Returns rate 0 without one.
fn open_device(buffer_size: u32) -> (i32, Option<cpal::Stream>, Arc<Mutex<Sound>>) {
    let fallback = || {
        (
            0,
            None,
            Arc::new(Mutex::new(Sound::new(48000, buffer_size))),
        )
    };
    let Some(device) = cpal::default_host().default_output_device() else {
        return fallback();
    };
    let Ok(config) = device.default_output_config() else {
        return fallback();
    };
    let rate = config.sample_rate() as i32;
    let engine = Arc::new(Mutex::new(Sound::new(rate, buffer_size)));
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&device, config.into(), &engine, buffer_size),
        cpal::SampleFormat::I16 => build::<i16>(&device, config.into(), &engine, buffer_size),
        cpal::SampleFormat::I32 => build::<i32>(&device, config.into(), &engine, buffer_size),
        cpal::SampleFormat::U16 => build::<u16>(&device, config.into(), &engine, buffer_size),
        _ => None,
    };
    match stream {
        Some(s) => (rate, Some(s), engine),
        None => fallback(),
    }
}

/// The stream: SDL's callback, `Mix` in chunks of `snd_buffer_size` frames (SDL's
/// `samples`), converted to the device's format and channels (stereo into the first two,
/// mono as their mean, like SDL's converters).
fn build<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    engine: &Arc<Mutex<Sound>>,
    chunk: u32,
) -> Option<cpal::Stream> {
    let engine = Arc::clone(engine);
    let channels = config.channels as usize;
    let mut pending: Vec<i16> = Vec::new();
    let mut at = 0;
    let stream = device
        .build_output_stream(
            config,
            move |out: &mut [T], _| {
                for frame in out.chunks_mut(channels) {
                    if at >= pending.len() {
                        pending.resize(chunk as usize * 2, 0);
                        // a poisoned lock (a panic elsewhere) plays silence
                        match engine.lock() {
                            Ok(mut e) => e.mix(&mut pending, chunk),
                            Err(_) => pending.fill(0),
                        }
                        at = 0;
                    }
                    let (l, r) = (
                        pending[at] as f32 / 32768.0,
                        pending[at + 1] as f32 / 32768.0,
                    );
                    at += 2;
                    for (c, s) in frame.iter_mut().enumerate() {
                        let v = match (channels, c) {
                            (1, _) => (l + r) * 0.5,
                            (_, 0) => l,
                            (_, 1) => r,
                            _ => 0.0,
                        };
                        *s = T::from_sample(v);
                    }
                }
            },
            |e| eprintln!("sound: {e}"),
            None,
        )
        .ok()?;
    stream.play().ok()?;
    Some(stream)
}
