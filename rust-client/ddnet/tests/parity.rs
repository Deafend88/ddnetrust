//! Parity with the C++ map loading and physics code, using data written by the C++
//! tool `src/tools/rs_parity.cpp` into `rust-client/tests/golden/maps/`. Maps are read
//! from the repository's `data/maps/`, not copied.
//!
//! Regenerate (repo root, after building the `rs_parity` CMake target), per map:
//!     build-rs/rs_parity.exe map "data/maps/<name>.map" rust-client/tests/golden/maps/<name>.txt <scratch dir>
//! with spaces in <name> replaced by '_' in the output file name.

use ddnet::map::{Layer, Map};
use ddnet::physics::collision::{Collision, Layers, clamp_vel};
use ddnet::physics::vmath::{PI, Vec2, direction, vec2};

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

fn golden_file(map: &str, kind: &str) -> String {
    let path = format!(
        "{}/../tests/golden/maps/{}.{kind}.txt",
        env!("CARGO_MANIFEST_DIR"),
        map.replace(' ', "_")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn load(name: &str) -> Map {
    Map::load(std::fs::read(repo_file(&format!("data/maps/{name}.map"))).unwrap()).unwrap()
}

fn bits(s: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(s, 16).unwrap())
}

/// Query values of one tick, in the order of `CollisionMode` in rs_parity.cpp.
#[derive(Default)]
struct Queries(Vec<i32>);

impl Queries {
    fn int(&mut self, v: impl Into<i64>) {
        self.0.push(v.into() as i32);
    }
    fn vec(&mut self, v: Vec2) {
        self.0.push(v.x.to_bits() as i32);
        self.0.push(v.y.to_bits() as i32);
    }
    fn crc(&self) -> u32 {
        let bytes: Vec<u8> = self.0.iter().flat_map(|v| v.to_le_bytes()).collect();
        crc(&bytes)
    }
}

/// Every tick of boxes driven through the map with `MoveBox`, plus the collision
/// queries the core and prediction use, against `rs_parity collision`.
fn check_collision(name: &str, map: &Map, col: &Collision, text: &str) {
    {
        let mut lines = text.lines();

        // static data
        let switch_types: Vec<u8> = if map.switch.and_then(|l| map.data(l.data_index)).is_some() {
            (0..col.width() * col.height())
                .flat_map(|i| col.get_switch_type(i).to_le_bytes())
                .collect()
        } else {
            Vec::new()
        };
        assert_eq!(
            format!(
                "static {} {:08x}",
                col.highest_switch_number,
                crc(&switch_types)
            ),
            lines.next().unwrap(),
            "{name}"
        );

        let switch_active = |n: u8| n.is_multiple_of(2);
        let mut ticks = 0;
        while let Some(body) = lines.next() {
            let b: Vec<&str> = body.split(' ').collect();
            assert_eq!(b[0], "body");
            let mut pos = vec2(bits(b[1]), bits(b[2]));
            let elasticity = vec2(bits(b[3]), bits(b[4]));
            let mut vel = vec2(0.0, 0.0);
            for tick in 0..300 {
                let line = lines.next().unwrap();
                let f: Vec<&str> = line.split(' ').collect();
                let n: Vec<i32> = f[..7].iter().map(|v| v.parse().unwrap()).collect();
                let (ax, jump, boost, bx, by, angle_deg, len) =
                    (n[0], n[1], n[2], n[3], n[4], n[5], n[6]);

                vel.y += 0.5;
                vel.x += ax as f32 * 1.5;
                if jump != 0 {
                    vel.y = -12.0;
                }
                if boost != 0 {
                    vel = vec2(bx as f32, by as f32);
                }
                let prev_pos = pos;
                let grounded = col.move_box(&mut pos, &mut vel, vec2(28.0, 28.0), elasticity);

                let mut q = Queries::default();
                q.int(grounded);
                q.int(col.is_on_ground(pos, 28.0));
                q.int(col.test_box(pos, vec2(28.0, 28.0)));
                q.int(col.check_point(pos));
                let restrictions = col.get_move_restrictions(None, pos, 18.0, -1);
                q.int(restrictions);
                q.int(col.get_move_restrictions(Some(&switch_active), pos, 18.0, -1));
                q.vec(clamp_vel(restrictions, vel));
                let pure = col.get_pure_map_index(pos);
                q.int(pure);
                q.int(col.get_tile_index(pure));
                q.int(col.get_front_tile_index(pure));
                q.int(col.get_tile_flags(pure));
                q.int(col.get_front_tile_flags(pure));
                q.int(col.is_tele_checkpoint(pure));
                q.int(col.is_speedup(pure));
                if col.is_speedup(pure) {
                    let (dir, force, max_speed, type_) = col.get_speedup(pure).unwrap();
                    q.vec(dir);
                    q.int(force);
                    q.int(max_speed);
                    q.int(type_);
                }
                q.int(col.is_tune(pure));
                q.int(col.get_switch_type(pure));
                q.int(col.get_switch_number(pure));
                q.int(col.get_switch_delay(pure));
                let map_index = col.get_map_index(pos);
                q.int(map_index);
                q.int(col.get_tile_index(map_index));
                q.int(col.is_tele_checkpoint(map_index));
                let indices = col.get_map_indices(prev_pos, pos, 0);
                q.int(indices.len() as i32);
                for i in indices {
                    q.int(i);
                }
                q.int(col.get_collision_at(pos.x, pos.y));
                q.int(col.get_front_collision_at(pos.x, pos.y));
                let end = pos + direction(angle_deg as f32 * (PI / 180.0)) * len as f32;
                let (hit, coll, before, tele_nr) = col.intersect_line_tele_hook(pos, end);
                q.int(hit);
                q.int(tele_nr);
                q.vec(coll);
                q.vec(before);
                let outs = col.tele_outs(tick % 8);
                q.int(outs.len() as i32);
                if let Some(&first) = outs.first() {
                    q.vec(first);
                }

                let got = format!(
                    "{:08x} {:08x} {:08x} {:08x} {:08x}",
                    pos.x.to_bits(),
                    pos.y.to_bits(),
                    vel.x.to_bits(),
                    vel.y.to_bits(),
                    q.crc()
                );
                let want = f[7..12].join(" ");
                assert_eq!(
                    got,
                    want,
                    "{name}, body {body}, tick {tick}: queries {}",
                    q.0.iter()
                        .map(|v| format!("{:x}", *v as u32))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
                ticks += 1;
            }
        }
        assert_eq!(ticks, 24 * 300, "{name}");
    }
}

#[test]
fn collision() {
    for name in MAPS {
        let map = load(name);
        check_collision(
            name,
            &map,
            &Collision::new(&map),
            &golden_file(name, "collision"),
        );
    }
}

/// The bundled maps have no hook teleporters, so both sides turn every tele-in tile
/// into one (`rs_parity collision ... hooktele`).
#[test]
fn collision_hook_teleporters() {
    for name in ["coverage", "LearnToPlay"] {
        let map = load(name);
        let mut layers = Layers::from_map(&map);
        for t in layers.tele.iter_mut().flatten() {
            if [10, 26, 31, 63].contains(&t.type_) {
                t.type_ = 15; // TILE_TELEINHOOK
            }
        }
        check_collision(
            name,
            &map,
            &Collision::from_layers(layers),
            &golden_file(name, "collision-hooktele"),
        );
    }
}
