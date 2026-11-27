//! M6.2: asset loading against the C++ code. Data from `rs_parity png` and
//! `rs_parity mapimages` (see PROGRESS.md for the regenerate commands).

use ddnet_rs::image::{self, Format};

fn repo(path: &str) -> String {
    format!("{}/../../{path}", env!("CARGO_MANIFEST_DIR"))
}

fn crc(data: &[u8]) -> String {
    let mut c = flate2::Crc::new();
    c.update(data);
    format!("{:08x}", c.sum())
}

/// Every PNG of the crafted corpus and of the repo's `data/` through the Rust loader:
/// same success, size, format, pixels and RGBA conversion as `CImageLoader::LoadPng`.
#[test]
fn png_loading_matches_cpp() {
    let dirs = [repo("rust-client/tests/golden/png"), repo("data")];
    let text = std::fs::read_to_string(repo("rust-client/tests/golden/png.txt")).unwrap();
    let mut wrong = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        let bytes =
            std::fs::read(format!("{}/{}", dirs[f[0].parse::<usize>().unwrap()], f[1])).unwrap();
        let got = match image::load_png(&bytes) {
            None => format!("{} {} fail", f[0], f[1]),
            Some(i) => format!(
                "{} {} {} {} {} {} {}",
                f[0],
                f[1],
                i.width,
                i.height,
                i.format as i32,
                crc(&i.data),
                crc(&i.to_rgba())
            ),
        };
        // the C++ line also has the pnglite compatibility flags (a warning only)
        let want = if f.len() > 3 {
            [&f[..5], &f[6..]].concat().join(" ")
        } else {
            line.to_string()
        };
        if got != want {
            wrong.push(format!("got  {got}\nwant {want}"));
        }
    }
    assert_eq!(text.lines().count(), 610);
    assert!(
        wrong.is_empty(),
        "{} differ:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn file_loader_rejects_gray() {
    let gray = std::fs::read(repo("rust-client/tests/golden/png/c0_d8.png")).unwrap();
    assert_eq!(image::load_png(&gray).unwrap().format, Format::R);
    assert!(image::load_png_file(&gray).is_none());
}

/// The map image decisions of `CMapImages::OnMapLoadImpl` on all 7 maps, unchanged and
/// with image/layer item ints changed like `rs_parity mapimages` did.
#[test]
fn map_images_match_cpp() {
    use ddnet::map::Map;
    use ddnet::map::images::{MapImage, map_images};
    let mut total = 0;
    for name in [
        "coverage",
        "LearnToPlay",
        "Gold Mine",
        "Tutorial",
        "Tsunami",
        "ctf1",
        "dm1",
    ] {
        let file = std::fs::read(repo(&format!("data/maps/{name}.map"))).unwrap();
        let text = std::fs::read_to_string(repo(&format!(
            "rust-client/tests/golden/maps/{}.mapimages.txt",
            name.replace(' ', "_")
        )))
        .unwrap();
        let mut lines = text.lines().peekable();
        while let Some(run) = lines.next() {
            let mut map = Map::load(file.clone()).unwrap();
            while let Some(set) = lines.next_if(|l| l.starts_with("set ")) {
                let v: Vec<i64> = set.split(' ').skip(1).map(|v| v.parse().unwrap()).collect();
                map.datafile.items[v[0] as usize].data[v[1] as usize] = v[2] as i32;
            }
            let usage = MapImage::usage(&map);
            let got: Vec<String> = map_images(&map)
                .iter()
                .enumerate()
                .map(|(i, img)| {
                    let what = match img {
                        MapImage::Unused => "unused".to_string(),
                        MapImage::NoName => "noname".into(),
                        MapImage::BadType => "type".into(),
                        MapImage::External { path } => format!("external {path}"),
                        MapImage::BadSize => "dims".into(),
                        MapImage::Embedded {
                            width,
                            height,
                            rgba,
                        } => {
                            format!("embedded {width} {height} {}", crc(rgba))
                        }
                        MapImage::NoData => "nodata".into(),
                        MapImage::Truncated => "truncated".into(),
                    };
                    format!("image {i} {} {what}", usage[i])
                })
                .collect();
            let mut want = Vec::new();
            while let Some(l) = lines.next_if(|l| l.starts_with("image ")) {
                want.push(l.to_string());
            }
            total += want.len();
            assert_eq!(got, want, "{name} {run}");
        }
    }
    assert!(total > 10_000, "{total}");
}

mod storage {
    use ddnet_rs::storage::Storage;
    use std::path::PathBuf;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ddnet-rs-storage-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for sub in ["user/skins", "data/skins", "data/mapres", "extra"] {
            std::fs::create_dir_all(d.join(sub)).unwrap();
        }
        std::fs::write(d.join("user/skins/a.png"), "user a").unwrap();
        std::fs::write(d.join("data/skins/a.png"), "data a").unwrap();
        std::fs::write(d.join("data/skins/b.png"), "data b").unwrap();
        std::fs::write(d.join("extra/x"), "extra x").unwrap();
        d
    }

    #[test]
    fn default_order_user_data_current() {
        let d = tmp("order");
        let s = Storage::from_parts(Some(d.join("user")), Some(d.join("data")), None).unwrap();
        assert_eq!(s.paths, [d.join("user"), d.join("data"), PathBuf::new()]);
        assert_eq!(s.read("skins/a.png").unwrap(), b"user a"); // user dir wins
        assert_eq!(s.read("skins/b.png").unwrap(), b"data b");
        assert_eq!(s.read("skins/../skins/b.png"), None); // may not leave the dir
        let mut names: Vec<String> = s.list("skins").into_iter().map(|(n, _)| n).collect();
        names.sort();
        assert_eq!(names, ["a.png", "b.png"]); // each name once
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn storage_cfg_paths() {
        let d = tmp("cfg");
        let extra = d.join("extra");
        let cfg = format!(
            "# comment\nadd_path {}\r\nadd_path $DATADIR\nadd_path /no/such/dir\n",
            extra.display()
        );
        let s = Storage::from_parts(
            Some(d.join("user")),
            Some(d.join("data")),
            Some(cfg.as_bytes()),
        )
        .unwrap();
        assert_eq!(s.paths, [extra.clone(), d.join("data")]);
        assert_eq!(s.read("x").unwrap(), b"extra x");
        // a first path that fails stops the client (C++ LoadPathsFromFile returns false)
        assert!(
            Storage::from_parts(None, Some(d.join("data")), Some(b"add_path $USERDIR\n")).is_err()
        );
        // no usable path in the file: the defaults
        let s = Storage::from_parts(Some(d.join("user")), None, Some(b"nothing here\n")).unwrap();
        assert_eq!(s.paths, [d.join("user"), PathBuf::new()]);
        assert!(Storage::from_parts(None, None, None).is_err());
        let _ = std::fs::remove_dir_all(d);
    }
}

mod lookups {
    use ddnet_rs::assets::{self, GAME};
    use ddnet_rs::storage::Storage;
    use std::path::{Path, PathBuf};

    fn png(name: &str) -> Vec<u8> {
        std::fs::read(super::repo(&format!("rust-client/tests/golden/png/{name}"))).unwrap()
    }

    fn put(root: &Path, file: &str, bytes: &[u8]) {
        let p = root.join(file);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    fn setup(tag: &str) -> (PathBuf, Storage) {
        let d = std::env::temp_dir().join(format!("ddnet-rs-assets-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (user, data) = (d.join("user"), d.join("data"));
        put(&data, "game.png", &png("c6_d8.png")); // 7x5
        put(&user, "assets/game/file.png", &png("c6_d8_33x3.png"));
        put(&user, "assets/game/dir/game.png", &png("c6_d8_1x1.png"));
        put(&user, "assets/game/gray.png", &png("c0_d8.png")); // the file loader rejects gray
        put(&data, "skins/default.png", &png("c6_d8.png"));
        put(&user, "skins/zz.png", &png("c6_d8_1x1.png"));
        put(&user, "skins/ bad.png", &png("c6_d8.png")); // leading space: invalid name
        put(&user, "skins/notes.txt", b"x");
        put(&user, "skins/folder.png/x", b"x"); // a directory
        put(&user, "downloadedskins/dl.png", &png("c2_d8.png"));
        let s = Storage::from_parts(Some(user), Some(data), None).unwrap();
        (d, s)
    }

    #[test]
    fn asset_fallback_chain() {
        let (d, s) = setup("chain");
        let size = |a: &assets::Asset| a.image.as_ref().map(|i| (i.width, i.height));
        let a = assets::load_asset(&s, GAME, "default");
        assert_eq!(
            (a.path.as_str(), size(&a), a.fallback.is_none()),
            ("game.png", Some((7, 5)), true)
        );
        let a = assets::load_asset(&s, GAME, "file");
        assert_eq!(
            (a.path.as_str(), size(&a)),
            ("assets/game/file.png", Some((33, 3)))
        );
        assert_eq!(a.fallback.map(|i| i.width), Some(7));
        let a = assets::load_asset(&s, GAME, "dir");
        assert_eq!(
            (a.path.as_str(), size(&a)),
            ("assets/game/dir/game.png", Some((1, 1)))
        );
        for missing in ["gray", "nothing"] {
            let a = assets::load_asset(&s, GAME, missing);
            assert_eq!(
                (a.path.as_str(), size(&a)),
                ("game.png", Some((7, 5))),
                "{missing}"
            );
        }
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn skins() {
        let (d, s) = setup("skins");
        assert_eq!(assets::list_skins(&s), ["default", "zz"]);
        assert_eq!(assets::load_skin(&s, "zz").map(|i| i.width), Some(1));
        assert_eq!(assets::load_skin(&s, "dl").map(|i| i.width), Some(7)); // already downloaded
        assert!(assets::load_skin(&s, "missing").is_none());
        assert!(assets::load_skin(&s, "../game").is_none());
        let _ = std::fs::remove_dir_all(d);
    }
}

/// With the repo's data dir every image the 7 maps use loads (external ones from
/// `mapres/`); with no data dir nothing panics and the external ones are missing.
#[test]
fn map_images_load_from_data_dir() {
    use ddnet::map::Map;
    use ddnet::map::images::{MapImage, map_images};
    use ddnet_rs::assets::{GAME, load_asset, load_map_images};
    use ddnet_rs::storage::Storage;
    let data = std::path::PathBuf::from(repo("data"));
    let with = Storage::from_parts(Some(data.clone()), Some(data), None).unwrap();
    let without =
        Storage::from_parts(Some(std::env::temp_dir().join("ddnet-rs-none")), None, None).unwrap();
    let mut external = 0;
    for name in [
        "coverage",
        "LearnToPlay",
        "Gold Mine",
        "Tutorial",
        "Tsunami",
        "ctf1",
        "dm1",
    ] {
        let map =
            Map::load(std::fs::read(repo(&format!("data/maps/{name}.map"))).unwrap()).unwrap();
        let kinds = map_images(&map);
        let loaded = load_map_images(&with, &map);
        let missing = load_map_images(&without, &map);
        for ((k, l), m) in kinds.iter().zip(&loaded).zip(&missing) {
            let used = matches!(k, MapImage::External { .. } | MapImage::Embedded { .. });
            assert_eq!(l.is_some(), used, "{name} {k:?}");
            if let MapImage::External { .. } = k {
                external += 1;
                assert!(m.is_none());
            }
        }
    }
    assert!(external > 10, "{external}");
    assert!(load_asset(&without, GAME, "default").image.is_none());
}

fn fnv(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
    })
}

/// Our pack loading (`assets::load_asset`, the grid and RGBA checks, `sprite_image`) for
/// every pack of `rs_parity assets <golden> <root>...` against its copies of the C++
/// loader: the file the chain loads, its validity, and every sprite of the kind's set.
fn check_packs(golden: &str, roots: &[std::path::PathBuf]) -> usize {
    use ddnet_rs::render::content::{SPRITES, SPRITESETS};
    let storage = ddnet_rs::storage::Storage {
        paths: roots.to_vec(),
        user_dir: None,
        data_dir: None,
    };
    let kinds = [
        ddnet_rs::assets::GAME,
        ddnet_rs::assets::PARTICLES,
        ddnet_rs::assets::EMOTICONS,
        ddnet_rs::assets::HUD,
        ddnet_rs::assets::EXTRAS,
    ];
    let text = std::fs::read_to_string(golden).unwrap();
    let mut want = text.lines().peekable();
    let mut packs = 0;
    while let Some(line) = want.next() {
        let (_, rest) = line.split_once("pack ").unwrap();
        let (dir, name) = rest.split_once(' ').unwrap();
        let kind = *kinds.iter().find(|k| k.0 == dir).unwrap();
        let asset = ddnet_rs::assets::load_asset(&storage, kind, name);
        let set_index = SPRITESETS.iter().position(|s| s.name == dir);
        let mut got = Vec::new();
        let loaded = asset.image.is_some();
        let valid = asset.image.and_then(|mut i| {
            let set = &SPRITESETS[set_index?];
            (i.check_divisibility(set.gridx as u32, set.gridy as u32, true)
                && i.format == image::Format::Rgba)
                .then_some(i)
        });
        got.push(format!(
            "asset {} {} {}",
            asset.path,
            loaded as i32,
            valid.is_some() as i32
        ));
        if let Some(img) = &valid {
            for s in SPRITES.iter().filter(|s| Some(s.set) == set_index) {
                let sprite = ddnet_rs::skin::sprite_image(img, asset.fallback.as_ref(), s);
                got.push(format!(
                    "sprite {} {} {} {:016x}",
                    s.name,
                    sprite.width,
                    sprite.height,
                    fnv(&sprite.data)
                ));
            }
        }
        for g in got {
            assert_eq!(Some(g.as_str()), want.next(), "{line}");
        }
        assert!(want.peek().is_none_or(|l| l.starts_with("pack ")), "{line}");
        packs += 1;
    }
    packs
}

#[test]
fn asset_packs_match_cpp() {
    let packs = check_packs(
        &repo("rust-client/tests/golden/assets.txt"),
        &[std::path::PathBuf::from(repo("data"))],
    );
    assert_eq!(packs, 11);
}

/// More packs: `RS_ASSETS_EXTRA` = `<rs_parity assets output>;<root>;<root>...`.
#[test]
fn asset_packs_match_cpp_extra() {
    let Ok(v) = std::env::var("RS_ASSETS_EXTRA") else {
        return;
    };
    let mut parts = v.split(';');
    let golden = parts.next().unwrap().to_string();
    let roots: Vec<std::path::PathBuf> = parts.map(std::path::PathBuf::from).collect();
    println!("{} packs", check_packs(&golden, &roots));
}

/// The entities textures (`render::entities`) for every pack, mod and mask of
/// `rs_parity entities <golden> <root>...` against its copies of `CMapImages::GetEntities`.
fn check_entities(golden: &str, roots: &[std::path::PathBuf]) -> usize {
    use ddnet_rs::render::entities;
    let storage = ddnet_rs::storage::Storage {
        paths: roots.to_vec(),
        user_dir: None,
        data_dir: None,
    };
    let text = std::fs::read_to_string(golden).unwrap();
    let mut lines = text.lines().peekable();
    let mut n = 0;
    while let Some(line) = lines.next() {
        let w: Vec<&str> = line.split(' ').collect();
        let (pack, mod_type, masked) = (w[1], w[2].parse().unwrap(), w[3] == "1");
        let (path, img) = entities::load_image(&storage, pack, mod_type);
        let got: Vec<String> = img.map_or(Vec::new(), |img| {
            entities::build(img, masked, mod_type)
                .iter()
                .map(|i| {
                    format!(
                        "layer {path} {} {} {} {:016x}",
                        i.width,
                        i.height,
                        i.format as i32,
                        fnv(&i.data)
                    )
                })
                .collect()
        });
        let mut want = Vec::new();
        while lines.peek().is_some_and(|l| l.starts_with("layer ")) {
            want.push(lines.next().unwrap().to_string());
        }
        assert_eq!(got, want, "{line}");
        n += 1;
    }
    n
}

#[test]
fn entities_textures_match_cpp() {
    let n = check_entities(
        &repo("rust-client/tests/golden/entities.txt"),
        &[std::path::PathBuf::from(repo("data"))],
    );
    assert_eq!(n, 28);
}

/// More packs: `RS_ENTITIES_EXTRA` = `<rs_parity entities output>;<root>;<root>...`.
#[test]
fn entities_textures_match_cpp_extra() {
    let Ok(v) = std::env::var("RS_ENTITIES_EXTRA") else {
        return;
    };
    let mut parts = v.split(';');
    let golden = parts.next().unwrap().to_string();
    let roots: Vec<std::path::PathBuf> = parts.map(std::path::PathBuf::from).collect();
    println!("{} entities textures", check_entities(&golden, &roots));
}
