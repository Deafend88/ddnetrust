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
/// directory, `RS_OPUS_CORPUS` its corpus. Lengths, channels and loop starts must match;
/// samples are compared for the largest and the mean difference.
#[test]
fn opus_matches_opusfile_corpus() {
    let (Ok(refdir), Ok(corpus)) = (
        std::env::var("RS_OPUS_REF"),
        std::env::var("RS_OPUS_CORPUS"),
    ) else {
        return;
    };
    let text = std::fs::read_to_string(format!("{refdir}/opus.txt")).unwrap();
    let (mut worst, mut sum, mut count, mut differ) = (0i32, 0f64, 0usize, 0usize);
    for line in text.lines() {
        let w: Vec<&str> = line.split(' ').collect();
        let got = ddnet_rs::opus::decode(&std::fs::read(format!("{corpus}/{}", w[1])).unwrap());
        let Some(got) = got else {
            assert_eq!(w[2], "fail", "{line}");
            continue;
        };
        let s = &got;
        assert_eq!(
            format!("ok {} {} {} {}", s.frames, s.rate, s.channels, s.loop_start),
            w[2..].join(" "),
            "{line}"
        );
        let pcm = std::fs::read(format!("{refdir}/{}.pcm", w[1])).unwrap();
        let mut file_worst = 0;
        for (i, b) in pcm.as_chunks::<2>().0.iter().enumerate() {
            let d = (i16::from_le_bytes([b[0], b[1]]) as i32 - s.data[i] as i32).abs();
            file_worst = file_worst.max(d);
            sum += d as f64;
            differ += (d != 0) as usize;
        }
        count += pcm.len() / 2;
        worst = worst.max(file_worst);
        println!("{} worst {file_worst}", w[1]);
    }
    println!(
        "opus: {count} samples, {differ} differ, worst {worst}, mean {:.4}",
        sum / count as f64
    );
}

/// Sunny Side Up's map sound against opusfile: exact length, channels and loop start;
/// 4 windows of samples within the tolerance measured on the corpus (DECISIONS.md 13.3).
#[test]
fn opus_matches_opusfile() {
    let map = ddnet::map::datafile::Datafile::parse(
        std::fs::read(repo_file("data/maps/Sunny Side Up.map")).unwrap(),
    )
    .unwrap();
    let item = &map.items[map.get_type(7).start]; // MAPITEMTYPE_SOUND
    let got = ddnet_rs::opus::decode(&map.data(item.data[3]).unwrap()).unwrap();
    let text =
        std::fs::read_to_string(repo_file("rust-client/tests/golden/opus_sunny.txt")).unwrap();
    let mut lines = text.lines().filter(|l| !l.starts_with('#'));
    let s = &got;
    assert_eq!(
        format!("ok {} {} {} {}", s.frames, s.rate, s.channels, s.loop_start),
        lines.next().unwrap()
    );
    let pcm = std::fs::read(repo_file("rust-client/tests/golden/opus_sunny.pcm")).unwrap();
    let mut want = pcm
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes([b[0], b[1]]));
    let (mut worst, mut sum, mut n) = (0, 0, 0);
    for off in lines.next().unwrap().split(' ') {
        let off: usize = off.parse().unwrap();
        for &v in &s.data[off * 2..(off + 4800) * 2] {
            let d = (want.next().unwrap() as i32 - v as i32).abs();
            worst = worst.max(d);
            sum += d;
            n += 1;
        }
    }
    assert!(
        worst <= 9 && sum as f64 / n as f64 <= 1.0,
        "worst {worst}, mean {}",
        sum as f64 / n as f64
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
