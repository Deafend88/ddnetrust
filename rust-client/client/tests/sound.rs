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
