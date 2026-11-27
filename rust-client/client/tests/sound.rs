//! PLAN §13.3: the sound decoders against the C++ client's.
//! `wv.txt` comes from `rs_parity wv`, which runs word-for-word copies of `CSound::DecodeWV`
//! and its reader on the real libwavpack (src/tools/rs_parity_sound.cpp): every
//! `data/audio/*.wv`, as it is and damaged 4 ways.

use ddnet_rs::wavpack;

fn repo_file(rel: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn fnv(data: &[i16]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &v in data {
        for b in (v as u16).to_le_bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

/// Runs one `rs_parity wv` dump; returns (files checked, decodes that succeeded).
fn check_wv(golden: &str) -> (usize, usize) {
    let text = std::fs::read_to_string(golden).unwrap();
    let (mut dir, mut checked, mut ok) = (String::new(), 0, 0);
    let mut orig = (String::new(), Vec::new());
    for line in text.lines() {
        let w: Vec<&str> = line.split(' ').collect();
        if w[0] == "dir" {
            dir = w[1..w.len() - 1].join(" ");
            continue;
        }
        let name = w[1];
        if orig.0 != name {
            let path = repo_file(&dir).join(name);
            orig = (name.to_string(), std::fs::read(path).unwrap());
        }
        let mut data = orig.1.clone();
        data.truncate(w[3].parse().unwrap());
        let sets: usize = w[4].parse().unwrap();
        for s in &w[5..5 + sets] {
            let (pos, val) = s.split_once(':').unwrap();
            data[pos.parse::<usize>().unwrap()] = val.parse().unwrap();
        }
        let got = match wavpack::decode(&data) {
            Some(d) => format!(
                "ok {} {} {} {:016x}",
                d.frames,
                d.rate,
                d.channels,
                fnv(&d.data)
            ),
            None => "fail".into(),
        };
        assert_eq!(got, w[5 + sets..].join(" "), "{line}");
        checked += 1;
        ok += got.starts_with("ok") as usize;
    }
    (checked, ok)
}

#[test]
fn wavpack_matches_libwavpack() {
    let (checked, ok) = check_wv(
        repo_file("rust-client/tests/golden/wv.txt")
            .to_str()
            .unwrap(),
    );
    assert_eq!((checked, ok), (655, 483));
}

/// Damaged files never panic the decoder (overflow checks are on in tests).
#[test]
fn wavpack_survives_damage() {
    let mut seed: u32 = 99;
    let mut rand = move || {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        (seed >> 8) as usize
    };
    for name in [
        "wp_laser_bnce-01.wv",
        "sfx_msg-client.wv",
        "wp_noammo-01.wv",
    ] {
        let orig = std::fs::read(repo_file(&format!("data/audio/{name}"))).unwrap();
        for _ in 0..400 {
            let mut data = orig.clone();
            for _ in 0..1 + rand() % 16 {
                let i = rand() % data.len();
                data[i] = rand() as u8;
            }
            data.truncate(rand() % (data.len() + 64));
            wavpack::decode(&data);
        }
    }
}

/// Map sounds against opusfile: `RS_OPUS_REF` is an `rs_parity opus <dir> <corpus>` output
/// directory, `RS_OPUS_CORPUS` its corpus. Everything must be identical.
#[test]
fn opus_matches_opusfile_corpus() {
    let (Ok(refdir), Ok(corpus)) = (
        std::env::var("RS_OPUS_REF"),
        std::env::var("RS_OPUS_CORPUS"),
    ) else {
        return;
    };
    let text = std::fs::read_to_string(format!("{refdir}/opus.txt")).unwrap();
    for line in text.lines() {
        let w: Vec<&str> = line.split(' ').collect();
        let got = ddnet_rs::opus::decode(&std::fs::read(format!("{corpus}/{}", w[1])).unwrap());
        let Some(s) = got else {
            assert_eq!(w[2], "fail", "{line}");
            continue;
        };
        assert_eq!(
            format!("ok {} {} {} {}", s.frames, s.rate, s.channels, s.loop_start),
            w[2..].join(" "),
            "{line}"
        );
        let pcm = std::fs::read(format!("{refdir}/{}.pcm", w[1])).unwrap();
        let want: Vec<i16> = pcm
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect();
        assert!(want == s.data, "{}: samples differ", w[1]);
    }
}

/// Sunny Side Up's map sound against opusfile (from ddnet-libs): identical samples.
#[test]
fn opus_matches_opusfile() {
    let map = ddnet::map::datafile::Datafile::parse(
        std::fs::read(repo_file("data/maps/Sunny Side Up.map")).unwrap(),
    )
    .unwrap();
    let item = &map.items[map.get_type(7).start]; // MAPITEMTYPE_SOUND
    let s = ddnet_rs::opus::decode(&map.data(item.data[3]).unwrap()).unwrap();
    let text =
        std::fs::read_to_string(repo_file("rust-client/tests/golden/opus_sunny.txt")).unwrap();
    let want = text.lines().find(|l| !l.starts_with('#')).unwrap();
    assert_eq!(
        format!(
            "ok {} {} {} {} {:016x}",
            s.frames,
            s.rate,
            s.channels,
            s.loop_start,
            fnv(&s.data)
        ),
        want
    );
}

/// Damaged map sounds (they come from servers) never panic the Opus path.
#[test]
fn opus_survives_damage() {
    let map = ddnet::map::datafile::Datafile::parse(
        std::fs::read(repo_file("data/maps/Sunny Side Up.map")).unwrap(),
    )
    .unwrap();
    let item = &map.items[map.get_type(7).start];
    let orig = map.data(item.data[3]).unwrap()[..40000].to_vec();
    let mut seed: u32 = 7;
    let mut rand = move || {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        (seed >> 8) as usize
    };
    for _ in 0..300 {
        let mut data = orig.clone();
        for _ in 0..1 + rand() % 24 {
            let i = rand() % data.len();
            data[i] = rand() as u8;
        }
        data.truncate(rand() % (data.len() + 64));
        ddnet_rs::opus::decode(&data);
    }
}

fn f32h(h: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(h, 16).unwrap())
}

/// The engine against copies of C++ `CSound` (`rs_parity mixer`): loads with rate
/// conversion at 48 and 44.1 kHz, 5000 random calls per rate, every result and every mix.
#[test]
fn mixer_matches_cpp() {
    use ddnet::physics::vmath::vec2;
    use ddnet_rs::sound::{Sound, VoiceHandle};
    let text = std::fs::read_to_string(repo_file("rust-client/tests/golden/mixer.txt")).unwrap();
    let (mut sound, mut decoded, mut mixes) = (None::<Sound>, Vec::new(), 0);
    for line in text.lines() {
        let (call, result) = line.split_once(" = ").unwrap_or((line, ""));
        let w: Vec<&str> = call.split(' ').collect();
        let int = |i: usize| w[i].parse::<i32>().unwrap();
        if w[0] == "run" {
            sound = Some(Sound::new(int(1), int(2) as u32));
            decoded.clear();
            continue;
        }
        let s = sound.as_mut().unwrap();
        let got = match w[0] {
            "load" => {
                let mut d = wavpack::decode(
                    &std::fs::read(repo_file(&format!("data/audio/{}", w[1]))).unwrap(),
                )
                .unwrap();
                d.loop_start = int(2) as usize;
                decoded.push(d.clone());
                assert_eq!(s.load(d).to_string(), w[3], "{line}");
                continue;
            }
            "reload" => s.load(decoded[int(1) as usize].clone()).to_string(),
            "play" => {
                let (ch, id, flags, vol) = (int(2) as usize, int(3), int(4), f32h(w[5]));
                let pos = vec2(f32h(w[6]), f32h(w[7]));
                let h = match int(1) {
                    0 => s.play(ch, id, flags, vol, vec2(0.0, 0.0)),
                    1 => s.play_at(ch, id, flags, vol, pos),
                    _ => s.play(ch, id, flags, vol, pos),
                };
                format!("{} {}", h.id, h.age)
            }
            "voice" => {
                let h = VoiceHandle {
                    id: int(2),
                    age: int(3),
                };
                let (a, b, c) = (f32h(w[4]), f32h(w[5]), f32h(w[6]));
                match int(1) {
                    0 => s.set_voice_volume(h, a),
                    1 => s.set_voice_falloff(h, a),
                    2 => s.set_voice_position(h, vec2(b, c)),
                    3 => s.set_voice_time_offset(h, b),
                    4 => s.set_voice_circle(h, b),
                    _ => s.set_voice_rectangle(h, b, c),
                }
                String::new()
            }
            "channel" => {
                s.set_channel(int(1) as usize, f32h(w[2]), f32h(w[3]));
                String::new()
            }
            "listener" => {
                s.set_listener_position(vec2(f32h(w[1]), f32h(w[2])));
                String::new()
            }
            "sample" => {
                let id = int(2);
                let r = match int(1) {
                    0 => {
                        s.stop(id);
                        0.0
                    }
                    1 => {
                        s.pause(id);
                        0.0
                    }
                    2 => s.is_playing(id) as i32 as f32,
                    3 => {
                        s.set_sample_current_time(id, f32h(w[3]));
                        0.0
                    }
                    4 => s.sample_current_time(id),
                    _ => s.sample_total_time(id),
                };
                format!("{:08x}", r.to_bits())
            }
            "stopvoice" => {
                s.stop_voice(VoiceHandle {
                    id: int(1),
                    age: int(2),
                });
                String::new()
            }
            "stopall" => {
                s.stop_all();
                String::new()
            }
            "unload" => {
                s.unload_sample(int(1));
                String::new()
            }
            "volume" => {
                s.volume = int(1);
                String::new()
            }
            "mix" => {
                let frames = int(1) as u32;
                let mut out = vec![0x5555i16; frames as usize * 2];
                s.mix(&mut out, frames);
                mixes += 1;
                format!("{:016x}", fnv(&out))
            }
            _ => panic!("{line}"),
        };
        assert_eq!(got, result, "{line}");
    }
    assert_eq!(mixes, 3301);
}

/// Replays one `rs_parity mapsounds` dump (copies of C++ `CMapSounds::OnMapLoad`/`OnRender`
/// and `CSounds::PlaySampleAt` on the copied engine) on `MapSounds`: the loaded sounds, then
/// per frame the source voices and the mix. Returns the frames checked.
fn check_mapsounds(map: &std::path::Path, golden: &std::path::Path) -> usize {
    use ddnet::physics::vmath::vec2;
    use ddnet_rs::config::Config;
    use ddnet_rs::mapsounds::MapSounds;
    use ddnet_rs::sound::Sound;
    use ddnet_rs::sounds::{CHN_MAPSOUND, Sounds};
    let text = std::fs::read_to_string(golden).unwrap();
    let mut lines = text.lines();
    let map = ddnet::map::Map::load(std::fs::read(map).unwrap()).unwrap();
    // no mapres/ here, as the C++ stand-in's LoadOpus fails
    let storage =
        ddnet_rs::storage::Storage::from_parts(None, None, Some(b"add_path $CURRENTDIR")).unwrap();
    let mut sounds = Sounds::with_engine(Sound::new(48000, 512));
    sounds
        .engine
        .lock()
        .unwrap()
        .set_channel(CHN_MAPSOUND, 30.0 / 100.0, 1.0);
    let mut warning = false;
    let mut ms = MapSounds::load(&map, &storage, &mut sounds, &mut warning);
    let ids: Vec<String> = std::iter::once(ms.ids.len() as i32)
        .chain(ms.ids.iter().copied())
        .map(|i| i.to_string())
        .collect();
    let head = format!("sounds {} sources {}", ids.join(" "), ms.voices().count());
    assert_eq!(head, lines.next().unwrap());
    let mut cfg = Config::default();
    let mut frames = 0;
    for line in lines {
        let (call, want) = line.split_once(" = ").unwrap();
        let w: Vec<&str> = call.split(' ').collect();
        let int = |i: usize| w[i].parse::<i64>().unwrap();
        cfg.snd_enable = int(9) as i32;
        cfg.gfx_high_detail = int(10) as i32;
        let round_start = (int(4) != 0).then_some(int(5) as i32);
        let ticks = (int(1) as i32, int(2) as i32, f32h(w[3]), round_start);
        let center = vec2(f32h(w[6]), f32h(w[7]));
        ms.on_render(&mut sounds, ticks, center, int(8), &cfg);
        let mut engine = sounds.engine.lock().unwrap();
        engine.set_listener_position(center);
        let n = int(11) as u32;
        let mut out = vec![0x5555i16; n as usize * 2];
        engine.mix(&mut out, n);
        drop(engine);
        let voices: Vec<String> = ms.voices().map(|v| format!("{}:{}", v.id, v.age)).collect();
        let got = format!("{} {:016x}", voices.join(" "), fnv(&out));
        assert_eq!(got, want, "{line}");
        frames += 1;
    }
    ms.clear(&mut sounds);
    frames
}

/// Sunny Side Up's sound layer against the C++ `CMapSounds` copies.
#[test]
fn mapsounds_match_cpp() {
    let frames = check_mapsounds(
        &repo_file("data/maps/Sunny Side Up.map"),
        &repo_file("rust-client/tests/golden/mapsounds_sunny.txt"),
    );
    assert_eq!(frames, 3000);
    // more maps: `RS_MAPSOUNDS_REF` holds `rs_parity mapsounds` dumps named
    // `<map>.txt` of the maps in `RS_MAPSOUNDS_MAPS` (run in this test: `TIME` is shared)
    let (Ok(refdir), Ok(maps)) = (
        std::env::var("RS_MAPSOUNDS_REF"),
        std::env::var("RS_MAPSOUNDS_MAPS"),
    ) else {
        return;
    };
    let mut total = 0;
    for e in std::fs::read_dir(&refdir).unwrap() {
        let golden = e.unwrap().path();
        let map = std::path::Path::new(&maps)
            .join(golden.file_stem().unwrap())
            .with_extension("map");
        total += check_mapsounds(&map, &golden);
    }
    eprintln!("mapsounds corpus: {total} frames identical");
}
