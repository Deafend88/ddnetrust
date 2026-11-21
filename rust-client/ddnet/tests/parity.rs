//! Parity with the C++ map loading and physics code, using data written by the C++
//! tool `src/tools/rs_parity.cpp` into `rust-client/tests/golden/maps/`. Maps are read
//! from the repository's `data/maps/`, not copied.
//!
//! Regenerate (repo root, after building the `rs_parity` CMake target), per map:
//!     build-rs/rs_parity.exe map "data/maps/<name>.map" rust-client/tests/golden/maps/<name>.txt <scratch dir>
//! with spaces in <name> replaced by '_' in the output file name.

use ddnet::map::{Layer, Map};

/// The maps of PLAN.md §5 (M4).
const MAPS: [&str; 7] = [
    "coverage",
    "LearnToPlay",
    "Gold Mine",
    "Tutorial",
    "Tsunami",
    "ctf1",
    "dm1",
];

fn repo_file(path: &str) -> String {
    format!("{}/../../{path}", env!("CARGO_MANIFEST_DIR"))
}

fn golden(map: &str) -> String {
    let path = format!(
        "{}/../tests/golden/maps/{}.txt",
        env!("CARGO_MANIFEST_DIR"),
        map.replace(' ', "_")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn crc(data: &[u8]) -> u32 {
    let mut c = flate2::Crc::new();
    c.update(data);
    c.sum()
}

fn dump_layers(map: &Map) -> Vec<String> {
    let layers: [(&str, Option<Layer>); 6] = [
        ("game", Some(map.game)),
        ("tele", map.tele),
        ("speedup", map.speedup),
        ("front", map.front),
        ("switch", map.switch),
        ("tune", map.tune),
    ];
    layers
        .iter()
        .map(|(name, layer)| match layer {
            None => format!("layer {name} -"),
            Some(l) => match map.data(l.data_index) {
                None => format!("layer {name} {} {} ERR", l.width, l.height),
                Some(d) => format!(
                    "layer {name} {} {} {} {:08x}",
                    l.width,
                    l.height,
                    d.len(),
                    crc(&d)
                ),
            },
        })
        .collect()
}

fn dump(map: &Map) -> Vec<String> {
    let df = &map.datafile;
    let mut out = vec![format!("file {} {:08x}", df.size(), df.crc())];
    for (index, item) in df.items.iter().enumerate() {
        let bytes: Vec<u8> = item.data.iter().flat_map(|v| v.to_le_bytes()).collect();
        let (type_, uuid) = match df.item_uuid(index) {
            None => (item.type_(), "-".to_string()),
            Some(u) => {
                let h: String = u.iter().map(|b| format!("{b:02x}")).collect();
                (
                    -1,
                    format!(
                        "{}-{}-{}-{}-{}",
                        &h[..8],
                        &h[8..12],
                        &h[12..16],
                        &h[16..20],
                        &h[20..]
                    ),
                )
            }
        };
        out.push(format!(
            "item {index} {type_} {uuid} {} {} {:08x}",
            item.id(),
            bytes.len(),
            crc(&bytes)
        ));
    }
    for index in 0..df.num_data() as i32 {
        out.push(match map.data(index) {
            None => format!("data {index} ERR"),
            Some(d) => format!("data {index} {} {:08x}", d.len(), crc(&d)),
        });
    }
    out.extend(dump_layers(map));
    out
}

fn assert_lines(got: &[String], want: &[&str], what: &str) {
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert_eq!(g, w, "{what}, line {}", i + 1);
    }
    assert_eq!(got.len(), want.len(), "{what}: line count");
}

#[test]
fn map_loading() {
    for name in MAPS {
        let bytes = std::fs::read(repo_file(&format!("data/maps/{name}.map"))).unwrap();
        let text = golden(name);
        let lines: Vec<&str> = text.lines().collect();
        let first_mut = lines
            .iter()
            .position(|l| l.starts_with("mut "))
            .unwrap_or(lines.len());
        let map = Map::load(bytes.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_lines(&dump(&map), &lines[..first_mut], name);

        // mutated copies: same accept/reject decision and same physics layers
        let mut i = first_mut;
        let mut mutations = 0;
        while i < lines.len() {
            let f: Vec<&str> = lines[i].split(' ').collect();
            let (offset, value): (usize, i32) = (f[2].parse().unwrap(), f[3].parse().unwrap());
            let mut m = bytes.clone();
            match f[1] {
                "t" => m.truncate(offset),
                "x" => m[offset] ^= value as u8,
                "w" => m[offset..offset + 4].copy_from_slice(&value.to_le_bytes()),
                k => panic!("bad mutation {k}"),
            }
            let what = format!("{name}: {}", lines[i]);
            match (Map::load(m), f[4]) {
                (Err(_), "fail") => i += 1,
                (Ok(map), "ok") => {
                    assert_lines(&dump_layers(&map), &lines[i + 1..i + 7], &what);
                    i += 7;
                }
                (got, _) => panic!("{what}: Rust got {:?}", got.map(|_| "ok")),
            }
            mutations += 1;
        }
        assert_eq!(mutations, 150, "{name}");
    }
}
