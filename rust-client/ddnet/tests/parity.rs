//! Parity with the C++ map loading and physics code, using data written by the C++
//! tool `src/tools/rs_parity.cpp` into `rust-client/tests/golden/maps/`. Maps are read
//! from the repository's `data/maps/`, not copied.
//!
//! Regenerate (repo root, after building the `rs_parity` CMake target), per map:
//!     build-rs/rs_parity.exe map "data/maps/<name>.map" rust-client/tests/golden/maps/<name>.txt <scratch dir>
//! with spaces in <name> replaced by '_' in the output file name.

use std::collections::BTreeMap;
use std::rc::Rc;

use ddnet::map::{Layer, Map};
use ddnet::physics::collision::{Collision, Layers, clamp_vel};
use ddnet::physics::core::{CharacterCore, WorldCore};
use ddnet::physics::teams::NUM_DDRACE_TEAMS;
use ddnet::physics::teams::TeamsCore;
use ddnet::physics::tuning::{Param, Tuning};
use ddnet::physics::vmath::{PI, Vec2, direction, round_to_int, vec2};
use ddnet::physics::world::{
    Character, GameWorld, NUM_TUNE_ZONES, TUNE_ZONE_OVERRIDE_NONE, WorldConfig,
};
use ddnet::protocol::generated::{
    self as g, ObjCharacter, ObjDDNetCharacter, ObjEntityEx, ObjPlayerInput,
};

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
        let mut movers = Queries::default();
        for i in 0..col.width() * col.height() {
            let (x, y) = ((i % col.width()) * 32 + 5, (i / col.width()) * 32 + 30);
            if let Some((index, speed)) = col.mover_speed(x, y) {
                movers.int(i);
                movers.int(index);
                movers.vec(speed);
            }
        }
        assert_eq!(
            format!("movers {} {:08x}", movers.0.len() / 4, movers.crc()),
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
                // weapons (M8.2)
                for (hit, coll, before) in [
                    col.intersect_line(pos, end),
                    col.intersect_line_tele_weapon(pos, end),
                ] {
                    q.int(hit);
                    q.vec(coll);
                    q.vec(before);
                }
                let (mut point_pos, mut point_vel) = (pos, (end - pos) * 0.05);
                let bounces = col.move_point(&mut point_pos, &mut point_vel, elasticity.x);
                q.vec(point_pos);
                q.vec(point_vel);
                q.int(bounces);
                let (mut point_pos, mut point_vel) = (pos, vel * 4.0);
                col.move_point(&mut point_pos, &mut point_vel, 1.0);
                q.vec(point_pos);
                q.vec(point_vel);
                let (mover, speed) = col
                    .mover_speed(round_to_int(pos.x), round_to_int(pos.y))
                    .unwrap_or((0, vec2(0.0, 0.0)));
                q.int(mover);
                q.vec(speed);

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
        check_collision(
            name,
            &map,
            &Collision::from_layers(make_movers(hook_tele_layers(&map))),
            &golden_file(name, "collision-hooktele"),
        );
    }
}

/// rs_parity's `MakeMovers` (collision "hooktele" variant): every 7th empty game tile
/// becomes a mover, with the four rotations and one unknown flag value.
fn make_movers(mut layers: Layers) -> Layers {
    let flags = [0, 8, 3, 11, 1]; // ROTATION_0/90/180/270, TILEFLAG_XFLIP
    let mut made = 0;
    for t in layers.tiles.iter_mut().step_by(7) {
        if t.index == 0 {
            t.index = if made % 2 == 1 { 65 } else { 64 }; // TILE_CP_F, TILE_CP
            t.flags = flags[made % 5];
            made += 1;
        }
    }
    layers
}

/// The "hooktele" variant of rs_parity: every tele-in tile becomes a hook tele-in.
fn hook_tele_layers(map: &Map) -> Layers {
    let mut layers = Layers::from_map(map);
    for t in layers.tele.iter_mut().flatten() {
        if [10, 26, 31, 63].contains(&t.type_) {
            t.type_ = 15; // TILE_TELEINHOOK
        }
    }
    layers
}

/// rs_parity's `MakeNewSpeedups` for the world mode: every second speedup tile becomes
/// the new type.
fn world_collision(mut layers: Layers) -> Collision {
    for t in layers.speedup.iter_mut().flatten().skip(1).step_by(2) {
        t.type_ = 29; // TILE_SPEED_BOOST
    }
    Collision::from_layers(layers)
}

/// The state rs_parity's `CoreState` writes, in its order.
fn core_state(c: &CharacterCore) -> Queries {
    let mut q = Queries::default();
    for v in &c.write().to_ints()[1..] {
        q.int(*v); // without m_Tick
    }
    for v in [c.pos, c.vel, c.hook_pos, c.hook_dir, c.hook_tele_base] {
        q.vec(v);
    }
    for v in [
        c.hook_tick,
        c.hook_state,
        c.hooked_player(),
        c.jumped,
        c.jumped_total,
        c.jumps,
        c.direction,
        c.angle,
        c.triggered_events,
        c.colliding,
    ] {
        q.int(v);
    }
    q.int(c.left_wall);
    q.int(c.new_hook);
    q.int(c.reset);
    q.int(c.attached_players.len() as i32);
    for &id in &c.attached_players {
        q.int(id);
    }
    q
}

/// Four interacting character cores, against `rs_parity core`.
fn check_core(name: &str, col: &Collision, text: &str) {
    const CHARS: usize = 4;
    let mut lines = text.lines().peekable();
    let mut ticks = 0;
    while let Some(run_line) = lines.next() {
        let r: Vec<i32> = run_line
            .split(' ')
            .skip(1)
            .map(|v| v.parse().unwrap())
            .collect();
        let (run, index) = (r[0], r[1]);
        let mut world = WorldCore::default();
        world.init_switchers(col.highest_switch_number);
        let mut teams = TeamsCore::default();
        let mut tuning = Tuning::default();
        if run == 1 {
            for (p, v) in [
                (Param::Gravity, 0.3),
                (Param::HookLength, 500.0),
                (Param::HookFireSpeed, 120.0),
                (Param::HookDragAccel, 5.0),
                (Param::HookDragSpeed, 20.0),
                (Param::GroundControlSpeed, 13.0),
                (Param::AirFriction, 0.9),
                (Param::VelrampStart, 300.0),
                (Param::HookDuration, 2.5),
                (Param::GroundElasticityX, 0.5),
                (Param::GroundElasticityY, 0.3),
            ] {
                tuning.set(p, v);
            }
            teams.set_team(2, 1);
            teams.set_solo(3, true);
        }
        let w = col.width();
        for (i, jumps) in [2, 3, 1, 0].into_iter().enumerate() {
            world.characters[i] = Some(CharacterCore::new());
            world.reset(i);
            let c = world.characters[i].as_mut().unwrap();
            c.id = i as i32;
            c.tuning = tuning;
            c.jumps = jumps;
            c.pos = vec2(
                (index % w) as f32 * 32.0 + 16.0 + (i as i32 * 30) as f32 - 45.0,
                (index / w) as f32 * 32.0 + 16.0,
            );
        }

        while lines.peek().is_some_and(|l| !l.starts_with("run ")) {
            let line = lines.next().unwrap();
            let (inputs, states) = line.split_once(" |").unwrap();
            let inputs: Vec<i32> = inputs.split(' ').map(|v| v.parse().unwrap()).collect();
            for i in 0..CHARS {
                let input = &mut world.characters[i].as_mut().unwrap().input;
                *input = Default::default();
                input.direction = inputs[i * 5];
                input.jump = inputs[i * 5 + 1];
                input.hook = inputs[i * 5 + 2];
                input.target_x = inputs[i * 5 + 3];
                input.target_y = inputs[i * 5 + 4];
            }
            for i in 0..CHARS {
                world.tick(i, col, &teams, true, true);
            }
            for i in 0..CHARS {
                world.move_(i, col, &teams);
                world.quantize(i);
            }
            let state = |i: usize| core_state(world.characters[i].as_ref().unwrap());
            let got: Vec<String> = (0..CHARS)
                .map(|i| {
                    let c = world.characters[i].as_ref().unwrap();
                    format!(
                        "{:08x} {:08x} {:08x}",
                        c.pos.x.to_bits(),
                        c.pos.y.to_bits(),
                        state(i).crc()
                    )
                })
                .collect();
            let want = states.trim();
            if got.join(" ") != want {
                let dump: Vec<String> = (0..CHARS)
                    .map(|i| {
                        state(i)
                            .0
                            .iter()
                            .map(|v| format!("{:x}", *v as u32))
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .collect();
                panic!(
                    "{name}, run {run}, tick {ticks}:\n got  {}\n want {want}\n states:\n {}",
                    got.join(" "),
                    dump.join("\n ")
                );
            }
            ticks += 1;
        }
    }
    assert_eq!(ticks, 2 * 600, "{name}");
}

#[test]
fn character_core() {
    for name in MAPS {
        let map = load(name);
        check_core(name, &Collision::new(&map), &golden_file(name, "core"));
    }
}

#[test]
fn character_core_hook_teleporters() {
    for name in ["coverage", "LearnToPlay"] {
        let map = load(name);
        check_core(
            name,
            &Collision::from_layers(hook_tele_layers(&map)),
            &golden_file(name, "core-hooktele"),
        );
    }
}

/// C++: `ZoneTuning` in rs_parity.cpp.
fn zone_tuning(zone: usize) -> Tuning {
    let mut t = Tuning::default();
    let z = zone as i32;
    if z > 0 {
        t.set(Param::Gravity, 0.5 + (z % 5) as f32 * 0.1);
        t.set(Param::GroundControlSpeed, 10.0 + (z % 4) as f32);
        t.set(Param::AirJumpImpulse, 12.0 - (z % 3) as f32);
        t.set(Param::JetpackStrength, 400.0 + (z * 10) as f32);
        t.set(Param::VelrampCurvature, 1.4 + (z % 3) as f32 * 0.3);
        t.set(Param::PlayerCollision, (z % 4 != 1) as i32 as f32);
        t.set(Param::PlayerHooking, (z % 4 != 2) as i32 as f32);
    }
    t
}

/// Same layout as `WorldState` in rs_parity.cpp.
fn world_state(ch: &Character, c: &CharacterCore, teams: &TeamsCore) -> Queries {
    let mut q = core_state(c);
    q.int(c.active_weapon);
    for w in &c.weapons {
        q.int(w.got);
        q.int(w.ammo);
    }
    q.vec(c.ninja.activation_dir);
    q.int(c.ninja.activation_tick);
    q.int(c.ninja.current_move_time);
    q.int(c.ninja.old_vel_amount);
    for flag in [
        c.solo,
        c.jetpack,
        c.collision_disabled,
        c.endless_hook,
        c.endless_jump,
        c.hammer_hit_disabled,
        c.grenade_hit_disabled,
        c.laser_hit_disabled,
        c.shotgun_hit_disabled,
        c.hook_hit_disabled,
        c.super_,
        c.invincible,
        c.has_telegun_gun,
        c.has_telegun_grenade,
        c.has_telegun_laser,
        c.is_in_freeze,
        c.deep_frozen,
        c.live_frozen,
    ] {
        q.int(flag);
    }
    q.int(c.freeze_start);
    q.int(c.freeze_end);
    for v in c.tuning.0 {
        q.int(v);
    }
    q.int(ch.freeze_time);
    q.int(ch.frozen_last_tick);
    q.int(ch.tele_checkpoint);
    q.int(ch.tile_index);
    q.int(ch.tile_f_index);
    q.int(ch.last_refill_jumps);
    q.vec(ch.prev_pos);
    q.vec(ch.prev_prev_pos);
    q.vec(ch.pos);
    q.int(ch.last_weapon);
    q.int(ch.interfering);
    q.int(ch.tune_zone);
    q.int(ch.overridden_tune_zone());
    q.int(teams.team(ch.id));
    q.int(teams.is_solo(ch.id));
    q
}

/// Four characters in the client prediction world, against `rs_parity world`.
fn check_world(name: &str, col: Rc<Collision>, text: &str) {
    const IDS: [usize; 4] = [3, 0, 6, 1]; // insertion order
    const SORTED: [usize; 4] = [0, 1, 3, 6];
    const START_TICK: i32 = 10000;
    let tunings: Vec<Tuning> = (0..NUM_TUNE_ZONES).map(zone_tuning).collect();
    let mut lines = text.lines().peekable();
    let mut ticks = 0;
    while let Some(run_line) = lines.next() {
        let r: Vec<i32> = run_line
            .split(' ')
            .skip(1)
            .map(|v| v.parse().unwrap())
            .collect();
        let (run, index) = (r[0], r[1]);
        let mode = run % 4;
        let config = WorldConfig {
            infinite_ammo: true,
            predict_tiles: mode != 3,
            predict_freeze: mode != 2,
            predict_ddrace: mode != 3,
            use_tune_zones: mode != 2,
            bug_ddrace_input: mode == 3,
            no_weak_hook_and_bounce: mode % 2 == 1,
            freeze_delay: 3,
            antiping_players: 1,
            ..Default::default()
        };
        let mut world = GameWorld::new(Rc::clone(&col), config);
        world.tunings = tunings.clone();
        world.game_tick = START_TICK;
        if mode == 1 {
            world.teams.set_team(IDS[2] as i32, 1);
        }
        let w = col.width();
        for (i, &id) in IDS.iter().enumerate() {
            let n = i as i32;
            let (x, y) = ((index % w) * 32 + 16 + n * 30 - 45, (index / w) * 32 + 16);
            let mut obj = ObjCharacter {
                x,
                y,
                hook_x: x,
                hook_y: y,
                hooked_player: -1,
                angle: n * 100,
                weapon: g::WEAPON_GUN,
                ammo_count: 10,
                attack_tick: START_TICK - 3,
                ..Default::default()
            };
            let mut ext = ObjDDNetCharacter {
                flags: g::CHARACTERFLAG_WEAPON_HAMMER | g::CHARACTERFLAG_WEAPON_GUN,
                jumps: 2 + n % 2,
                jumped_total: -1,
                ninja_activation_tick: -1,
                freeze_start: -1,
                strong_weak_id: n,
                tune_zone_override: TUNE_ZONE_OVERRIDE_NONE,
                ..Default::default()
            };
            match (mode, i) {
                (0, 1) => ext.freeze_end = START_TICK + 60,
                (0, 2) => {
                    ext.flags |= g::CHARACTERFLAG_ENDLESS_HOOK | g::CHARACTERFLAG_ENDLESS_JUMP
                }
                (1, 3) => ext.flags |= g::CHARACTERFLAG_SUPER,
                (2, 0) => ext.flags |= g::CHARACTERFLAG_JETPACK,
                (2, 1) => {
                    ext.flags |= g::CHARACTERFLAG_WEAPON_NINJA;
                    ext.ninja_activation_tick = START_TICK - 700;
                    obj.weapon = g::WEAPON_NINJA;
                }
                (2, 2) => {
                    ext.flags |=
                        g::CHARACTERFLAG_COLLISION_DISABLED | g::CHARACTERFLAG_WEAPON_SHOTGUN;
                    ext.tune_zone_override = 3;
                    ext.target_x = 100;
                    ext.target_y = -50;
                }
                (3, 3) => ext.flags |= g::CHARACTERFLAG_SOLO,
                (3, 1) => ext.freeze_end = -1,
                _ => {}
            }
            world.add_character(id, &obj, &ext);
            if i == 0 {
                world.read_character(id, &obj, &ext, true); // the local player
            }
        }

        let mut tick = 0;
        while lines.peek().is_some_and(|l| !l.starts_with("run ")) {
            tick += 1;
            let line = lines.next().unwrap();
            let (inputs, rest) = line.split_once(" |").unwrap();
            let (states, switches) = rest.rsplit_once(" | ").unwrap();
            let v: Vec<i32> = inputs.split(' ').map(|v| v.parse().unwrap()).collect();
            let input = |k: usize| {
                let v = &v[k * 10..k * 10 + 10];
                ObjPlayerInput {
                    direction: v[0],
                    jump: v[1],
                    hook: v[2],
                    target_x: v[3],
                    target_y: v[4],
                    fire: v[5],
                    next_weapon: v[6],
                    prev_weapon: v[7],
                    wanted_weapon: v[8],
                    player_flags: v[9],
                }
            };
            for (k, &id) in SORTED.iter().enumerate() {
                world.on_direct_input(id, &input(k));
            }
            world.game_tick = START_TICK + tick;
            for (k, &id) in SORTED.iter().enumerate() {
                world.on_predicted_input(id, &input(k));
            }
            world.tick();

            let state = |id: usize| {
                let (ch, c) = world.character(id).unwrap();
                world_state(ch, c, &world.teams)
            };
            let got: Vec<String> = SORTED
                .iter()
                .map(|&id| {
                    let pos = world.character(id).unwrap().0.pos;
                    format!(
                        "{:08x} {:08x} {:08x}",
                        pos.x.to_bits(),
                        pos.y.to_bits(),
                        state(id).crc()
                    )
                })
                .collect();
            let mut sw = Queries::default();
            for s in &world.core.switchers {
                for t in 0..NUM_DDRACE_TEAMS {
                    sw.int(s.status[t]);
                    sw.int(s.end_tick[t]);
                    sw.int(s.type_[t]);
                    sw.int(s.last_update_tick[t]);
                }
            }
            let got = format!("{} | {:08x}", got.join(" "), sw.crc());
            let want = format!("{} | {switches}", states.trim());
            if got != want {
                let dump: Vec<String> = SORTED
                    .iter()
                    .map(|&id| {
                        state(id)
                            .0
                            .iter()
                            .map(|v| format!("{:x}", *v as u32))
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .collect();
                panic!(
                    "{name}, run {run}, tick {tick}:\n got  {got}\n want {want}\n states:\n {}",
                    dump.join("\n ")
                );
            }
            ticks += 1;
        }
    }
    assert_eq!(ticks, 24 * 100, "{name}");
}

#[test]
fn world() {
    for name in MAPS {
        let map = load(name);
        check_world(
            name,
            Rc::new(world_collision(Layers::from_map(&map))),
            &golden_file(name, "world"),
        );
    }
}

#[test]
fn world_hook_teleporters() {
    for name in ["coverage", "LearnToPlay"] {
        let map = load(name);
        check_world(
            name,
            Rc::new(world_collision(hook_tele_layers(&map))),
            &golden_file(name, "world-hooktele"),
        );
    }
}

/// rs_parity's `MakeFrontDoors`: every game/switch-layer door tile is copied to the front
/// layer with its laser tiles, every second door one laser length further.
fn front_door_layers(map: &Map) -> Layers {
    const DOOR: u8 = 240; // ENTITY_OFFSET + ENTITY_DOOR
    const SHORT: u8 = 210; // ENTITY_OFFSET + ENTITY_LASER_SHORT
    const SIDES: [(i32, i32); 8] = [
        (0, 1),
        (1, 1),
        (1, 0),
        (1, -1),
        (0, -1),
        (-1, -1),
        (-1, 0),
        (-1, 1),
    ];
    let mut l = Layers::from_map(map);
    let (w, h) = (l.width, l.height);
    let Some(mut front) = l.front.take() else {
        return l;
    };
    let mut doors = 0;
    for i in 0..w * h {
        for layer in 0..2 {
            let source = |index: i32| match (layer, &l.switch) {
                (0, _) => l.tiles[index as usize].index,
                (_, Some(s)) => s[index as usize].type_,
                _ => 0,
            };
            if source(i) != DOOR {
                continue;
            }
            doors += 1;
            front[i as usize].index = DOOR;
            for (dx, dy) in SIDES {
                let (x, y) = (i % w + dx, i / w + dy);
                if x < 0 || x >= w || y < 0 || y >= h {
                    continue;
                }
                let laser = source(y * w + x);
                if !(SHORT..=SHORT + 2).contains(&laser) {
                    continue;
                }
                front[(y * w + x) as usize].index = if doors % 2 == 1 {
                    laser
                } else {
                    SHORT + (laser - SHORT + 1) % 3
                };
            }
        }
    }
    l.front = Some(front);
    l
}

/// Door lasers snapshot by snapshot through `GameWorld` (`net_obj_begin`,
/// `net_obj_add`, `net_obj_end`): the collision door grid after each snapshot
/// must equal `rs_parity doors`.
#[test]
fn doors() {
    for name in MAPS {
        let col = Rc::new(Collision::from_layers(front_door_layers(&load(name))));
        let text = golden_file(name, "doors");
        let mut world = GameWorld::new(Rc::clone(&col), WorldConfig::default());
        let mut teams = TeamsCore::default();
        teams.set_team(2, 1);
        let mut lines = text.lines().skip(1); // "pool ..."
        let mut snaps = 0;
        while let Some(snap) = lines.next() {
            let s: Vec<i32> = snap
                .split(' ')
                .skip(1)
                .map(|v| v.parse().unwrap())
                .collect();
            world.config.predict_tiles = s[1] != 0;
            world.net_obj_begin(teams.clone(), 0);
            for _ in 0..s[2] {
                let line = lines.next().unwrap();
                let (item, ex) = line.split_once(" ex ").unwrap_or((line, ""));
                let (type_name, item) = item.split_once(' ').unwrap();
                let type_id = match type_name {
                    "laser" => g::NETOBJTYPE_LASER,
                    _ => g::NETOBJTYPE_DDNETLASER,
                };
                let v: Vec<i32> = item.split(' ').map(|v| v.parse().unwrap()).collect();
                let ex: Vec<i32> = ex.split(' ').filter_map(|v| v.parse().ok()).collect();
                let ex = (!ex.is_empty()).then(|| g::ObjEntityEx::from_ints(&ex));
                let mut data = v[1..].to_vec();
                data.resize(10, 0);
                world.net_obj_add(v[0], type_id, &data, ex.as_ref());
            }
            world.net_obj_end();
            let mut q = Queries::default();
            let mut tiles = 0;
            for i in 0..col.width() * col.height() {
                let d = col.get_door_tile(i);
                q.int(d.index);
                q.int(d.flags);
                q.int(d.number);
                tiles += (d.index != 0) as i32;
            }
            assert_eq!(
                lines.next().unwrap(),
                format!("grid {tiles} {:08x}", q.crc()),
                "{name} snapshot {}",
                s[0]
            );
            snaps += 1;
        }
        assert_eq!(snaps, 60, "{name}");
    }
}

/// M6.6: snapshot entities through `ExtractProjectileInfo`, `ExtractPickupInfo` and
/// `ExtractLaserInfo`, and `CalcPos`: bit-identical to `rs_parity items` on coverage
/// (with and without EntityEx, with and without tune zones).
#[test]
fn items_match_cpp() {
    use ddnet::items::{calc_pos, extract_laser, extract_pickup, extract_projectile};
    use g::ObjEntityEx;

    let map = Map::load(std::fs::read(repo_file("data/maps/coverage.map")).unwrap()).unwrap();
    let col = Collision::new(&map);
    let text = std::fs::read_to_string(repo_file("rust-client/tests/golden/items.txt")).unwrap();
    let b = |f: f32| format!("{:08x}", f.to_bits());
    let fl = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
    let mut n = 0;
    for line in text.lines() {
        let (input, want) = line.split_once(" => ").unwrap();
        let f: Vec<&str> = input.split(' ').collect();
        let type_id = match f[1] {
            "projectile" => g::NETOBJTYPE_PROJECTILE,
            "ddraceprojectile" => g::NETOBJTYPE_DDRACEPROJECTILE,
            "ddnetprojectile" => g::NETOBJTYPE_DDNETPROJECTILE,
            "pickup" => g::NETOBJTYPE_PICKUP,
            "ddnetpickup" => g::NETOBJTYPE_DDNETPICKUP,
            "laser" => g::NETOBJTYPE_LASER,
            _ => g::NETOBJTYPE_DDNETLASER,
        };
        let ex_at = f.iter().position(|&s| s == "ex").unwrap();
        let ints: Vec<i32> = f[2..ex_at].iter().map(|v| v.parse().unwrap()).collect();
        let e: Vec<i32> = f[ex_at + 1..ex_at + 5]
            .iter()
            .map(|v| v.parse().unwrap())
            .collect();
        let ex = (e[0] != 0).then(|| ObjEntityEx::from_ints(&e[1..]));
        let tunes = (f[ex_at + 6] == "1").then_some(&col);
        let got = match f[0] {
            "proj" => {
                let d = extract_projectile(type_id, &ints, tunes, ex.as_ref());
                let w: Vec<&str> = want.split(' ').collect();
                let calc = w.iter().position(|&s| s == "calc").unwrap();
                let (curv, speed, ct) = (fl(w[calc + 1]), fl(w[calc + 2]), fl(w[calc + 3]));
                let pos = calc_pos(d.start_pos, d.start_vel, curv, speed, ct);
                let prev = calc_pos(d.start_pos, d.start_vel, curv, speed, ct - 0.001);
                format!(
                    "{} {} {} {} {} {} {} {} {} {} {} {} {} calc {} {} {} {} {} {} {}",
                    b(d.start_pos.x),
                    b(d.start_pos.y),
                    b(d.start_vel.x),
                    b(d.start_vel.y),
                    d.type_,
                    d.start_tick,
                    d.extra_info as i32,
                    d.owner,
                    d.explosive as i32,
                    d.bouncing,
                    d.freeze as i32,
                    d.switch_number,
                    d.tune_zone,
                    w[calc + 1],
                    w[calc + 2],
                    w[calc + 3],
                    b(pos.x),
                    b(pos.y),
                    b(prev.x),
                    b(prev.y)
                )
            }
            "pickup" => {
                let d = extract_pickup(type_id, &ints, ex.as_ref());
                format!(
                    "{} {} {} {} {} {}",
                    b(d.pos.x),
                    b(d.pos.y),
                    d.type_,
                    d.subtype,
                    d.switch_number,
                    d.flags
                )
            }
            "laser" => {
                let d = extract_laser(type_id, &ints, tunes, ex.as_ref());
                format!(
                    "{} {} {} {} {} {} {} {} {} {} {} {}",
                    b(d.from.x),
                    b(d.from.y),
                    b(d.to.x),
                    b(d.to.y),
                    d.start_tick,
                    d.extra_info as i32,
                    d.owner,
                    d.type_,
                    d.switch_number,
                    d.subtype,
                    d.tune_zone,
                    d.predict as i32
                )
            }
            _ => panic!("{line}"),
        };
        assert_eq!(got, want, "{input}");
        n += 1;
    }
    assert_eq!(n, 12000);
}

fn tunings_crc(list: &[Tuning]) -> u32 {
    let bytes: Vec<u8> = list
        .iter()
        .flat_map(|t| t.0.iter().flat_map(|v| v.to_le_bytes()))
        .collect();
    crc(&bytes)
}

fn unhex(s: &str) -> Vec<u8> {
    if s == "-" {
        return Vec::new();
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// The game messages that change the prediction world, `UpdateLocalTuning` and the map's
/// tune settings, against `rs_parity msgs` (word-for-word copies of the CGameClient code
/// driving the real CUnpacker, CTeamsCore, CCharOrder, CGameWorld and CConsole).
#[test]
fn messages_match_cpp() {
    use ddnet::net::client::{PredictMsg, read_ints};
    use ddnet::physics::teams::MAX_CLIENTS;
    use ddnet::predict::{Predictor, apply_map_settings, map_tunings};
    use ddnet::protocol::packer::Unpacker;

    let map = load("coverage");
    let col = Rc::new(Collision::new(&map));
    let path = format!("{}/../tests/golden/msgs.txt", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let num = |s: &str| s.parse::<i32>().unwrap();
    let mut pred: Option<Predictor> = None;
    let mut counts = std::collections::BTreeMap::new();
    for line in text.lines() {
        let (input, want) = line.split_once(" = ").unwrap_or((line, ""));
        let w: Vec<&str> = input.split(' ').collect();
        *counts.entry(w[0]).or_insert(0) += 1;
        let got = match w[0] {
            "mapsettings" => format!("{:08x}", tunings_crc(&map_tunings(&map))),
            "settings" => {
                let mut list = vec![Tuning::default(); NUM_TUNE_ZONES];
                apply_map_settings(&mut list, &unhex(w[1]));
                format!("{:08x}", tunings_crc(&list))
            }
            "run" => {
                let mut p = Predictor::new(Rc::clone(&col), &map);
                let world = p.world_mut();
                world.tunings = vec![Tuning::default(); NUM_TUNE_ZONES];
                world.game_tick = 1000;
                world.config = WorldConfig {
                    infinite_ammo: true,
                    predict_freeze: true,
                    predict_ddrace: true,
                    predict_tiles: true,
                    is_fng: w[2] == "1",
                    freeze_delay: 3,
                    antiping_players: 1,
                    ..Default::default()
                };
                pred = Some(p);
                continue;
            }
            "char" => {
                let (obj, ext) = input.split_once(" |").unwrap();
                let obj: Vec<i32> = obj.split(' ').skip(2).map(num).collect();
                let ext: Vec<i32> = ext.split_whitespace().map(num).collect();
                pred.as_mut().unwrap().world_mut().add_character(
                    num(w[1]) as usize,
                    &ObjCharacter::from_ints(&obj),
                    &ObjDDNetCharacter::from_ints(&ext),
                );
                continue;
            }
            _ => {
                let p = pred.as_mut().unwrap();
                match w[0] {
                    "tune" => {
                        let bytes = unhex(w[1]);
                        let ints = read_ints(
                            &mut Unpacker::new(&bytes),
                            ddnet::physics::tuning::NUM_PARAMS,
                        );
                        p.on_message(PredictMsg::TuneParams(ints));
                        let t = p.received_tuning();
                        format!(
                            "{:08x} {}",
                            tunings_crc(std::slice::from_ref(t)),
                            p.tune_state().0 as i32
                        )
                    }
                    "teams" => {
                        let bytes = unhex(w[1]);
                        let ints = read_ints(&mut Unpacker::new(&bytes), MAX_CLIENTS);
                        p.teams_mut().num_ddrace_teams = NUM_DDRACE_TEAMS as i32;
                        p.on_message(PredictMsg::TeamsState(ints));
                        let t = p.teams_mut();
                        let teams: Vec<String> = (0..MAX_CLIENTS as i32)
                            .map(|i| t.team(i).to_string())
                            .collect();
                        format!("{} {}", teams.join(" "), t.num_ddrace_teams)
                    }
                    "kill" | "killteam" => {
                        p.on_message(if w[0] == "kill" {
                            PredictMsg::KillMsg(g::MsgSvKillMsg {
                                victim: num(w[1]),
                                weapon: num(w[2]),
                                ..Default::default()
                            })
                        } else {
                            PredictMsg::KillMsgTeam(g::MsgSvKillMsgTeam {
                                team: num(w[1]),
                                first: num(w[2]),
                            })
                        });
                        let order: Vec<String> =
                            p.char_order().iter().map(|i| i.to_string()).collect();
                        let world = p.world_mut();
                        let chars: Vec<String> = (0..MAX_CLIENTS)
                            .filter_map(|i| {
                                let (ch, c) = world.character(i)?;
                                Some(format!(
                                    "{i}:{:08x}",
                                    world_state(ch, c, &world.teams).crc()
                                ))
                            })
                            .collect();
                        format!("{} | {}", order.join(" "), chars.join(" "))
                    }
                    "pre" => {
                        let v: Vec<i32> = w[1..12].iter().map(|s| num(s)).collect();
                        p.on_message(PredictMsg::PreInput(g::MsgSvPreInput {
                            direction: v[0],
                            target_x: v[1],
                            target_y: v[2],
                            jump: v[3],
                            fire: v[4],
                            hook: v[5],
                            wanted_weapon: v[6],
                            next_weapon: v[7],
                            prev_weapon: v[8],
                            owner: v[9],
                            intended_tick: v[10],
                        }));
                        match p.pre_input(num(w[13]) as usize, num(w[14])) {
                            Some(m) => [
                                m.direction,
                                m.target_x,
                                m.target_y,
                                m.jump,
                                m.fire,
                                m.hook,
                                m.wanted_weapon,
                                m.next_weapon,
                                m.prev_weapon,
                                m.owner,
                                m.intended_tick,
                            ]
                            .map(|v| v.to_string())
                            .join(" "),
                            None => "none".into(),
                        }
                    }
                    "snap" => {
                        let (has_local, has_spec) = (w[1] == "1", w[2] == "1");
                        let pos = vec2(num(w[3]) as f32, num(w[4]) as f32);
                        let zone = num(w[5]);
                        p.world_mut().config.use_tune_zones = w[6] == "1";
                        p.update_local_tuning(
                            (has_local || has_spec).then_some(pos),
                            (zone != -2).then_some(zone),
                        );
                        let (received, local, expecting, since) = p.tune_state();
                        format!(
                            "{:08x} {} {local} {expecting} {since}",
                            tunings_crc(&p.world_mut().tunings),
                            received as i32
                        )
                    }
                    other => panic!("unknown line {other}"),
                }
            }
        };
        assert_eq!(got, want, "{input}");
    }
    let n = |k: &str| counts.get(k).copied().unwrap_or(0);
    assert_eq!(n("settings"), 400);
    assert_eq!(n("run"), 30);
    assert!(
        ["tune", "teams", "kill", "killteam", "pre", "snap"]
            .iter()
            .all(|k| n(k) > 100),
        "{counts:?}"
    );
}

/// Every word-for-word copy of C++ code in the parity tools ("C++ copy: <file>:<a>-<b>"
/// ... "end of copy") is still identical, whitespace aside, to those source lines.
#[test]
fn cpp_copies_match_source() {
    let normalize = |lines: &[&str]| -> Vec<String> {
        lines
            .iter()
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|l| !l.is_empty())
            .collect()
    };
    let mut copies = 0;
    for tool in [
        "src/tools/rs_parity_msgs.cpp",
        "src/tools/rs_parity_antiping.cpp",
        "src/tools/rs_parity_chat.cpp",
        "src/tools/rs_parity_scoreboard.cpp",
        "src/tools/rs_parity_sound.cpp",
        "src/tools/rs_parity_assets.cpp",
        "src/tools/rs_parity_entities.cpp",
        "src/tools/rs_parity_menus.cpp",
    ] {
        let text = std::fs::read_to_string(repo_file(tool)).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            let Some((_, at)) = lines[i].split_once("// C++ copy: ") else {
                i += 1;
                continue;
            };
            let (file, range) = at.trim().rsplit_once(':').unwrap();
            let (a, b) = range.split_once('-').unwrap();
            let (a, b): (usize, usize) = (a.parse().unwrap(), b.parse().unwrap());
            let end = (i..lines.len())
                .find(|&j| lines[j].trim() == "// end of copy")
                .unwrap_or_else(|| panic!("{tool}:{}: copy without end", i + 1));
            let source = std::fs::read_to_string(repo_file(&format!("src/{file}"))).unwrap();
            let source: Vec<&str> = source.lines().collect();
            assert_eq!(
                normalize(&lines[i + 1..end]),
                normalize(&source[a - 1..b]),
                "{tool}:{}: copy of {file}:{a}-{b} differs",
                i + 1
            );
            copies += 1;
            i = end + 1;
        }
    }
    assert_eq!(copies, 58);
}

/// rs_parity's `WeaponZoneTuning`: `ZoneTuning` plus weapon parameters per zone.
fn weapon_zone_tuning(zone: usize) -> Tuning {
    let mut t = zone_tuning(zone);
    match zone % 3 {
        1 => {
            t.set(Param::LaserReach, 400.0);
            t.set(Param::GrenadeCurvature, 4.0);
            t.set(Param::ExplosionStrength, 3.0);
            t.set(Param::LaserBounceNum, 1.0);
        }
        2 => {
            t.set(Param::GunSpeed, 1400.0);
            t.set(Param::ShotgunStrength, 25.0);
            t.set(Param::HammerStrength, 3.0);
            t.set(Param::GunFireDelay, 200.0);
        }
        _ => {}
    }
    t
}

/// `WeaponState` of rs_parity: `world_state` plus the weapon state.
fn weapon_char_state(ch: &Character, c: &CharacterCore, teams: &TeamsCore) -> Queries {
    let mut q = world_state(ch, c, teams);
    q.int(ch.reload_timer);
    q.int(ch.attack_tick);
    q.int(ch.queued_weapon);
    q.int(ch.pain_sound_timer);
    q.int(ch.hit_objects.len() as i32);
    q.int(ch.move_restrictions);
    q.int(ch.game_team);
    q.int(ch.last_snap_weapon);
    q
}

/// `WeaponEntityState` of rs_parity.
fn weapon_entity_state(w: &GameWorld) -> Queries {
    let mut q = Queries::default();
    for p in &w.projectiles {
        q.int(p.id);
        q.vec(p.pos);
        q.vec(p.direction);
        q.int(p.life_span);
        q.int(p.owner);
        q.int(p.type_);
        q.int(p.start_tick);
        q.int(p.explosive);
        q.int(p.layer);
        q.int(p.number);
        q.int(p.bouncing);
        q.int(p.freeze);
        q.int(p.tune_zone);
    }
    q.int(-1);
    for l in &w.lasers {
        q.int(l.id);
        q.vec(l.pos);
        q.vec(l.from);
        q.vec(l.dir);
        q.vec(l.prev_pos);
        q.0.push(l.energy.to_bits() as i32);
        q.int(l.bounces);
        q.int(l.eval_tick);
        q.int(l.owner);
        q.int(l.type_);
        q.int(l.zero_energy_bounce_in_last_tick);
        q.int(l.tune_zone);
    }
    q.int(-1);
    for d in &w.draggers {
        q.int(d.id);
        q.vec(d.pos);
        q.vec(d.core);
        q.0.push(d.strength.to_bits() as i32);
        q.int(d.ignore_walls);
        q.int(d.target_id);
        q.int(d.number);
        q.int(d.layer);
    }
    q.int(-1);
    for p in &w.plasmas {
        q.int(p.id);
        q.vec(p.pos);
        q.vec(p.core);
        q.int(p.freeze);
        q.int(p.explosive);
        q.int(p.for_client_id);
        q.int(p.eval_tick);
        q.int(p.life_time);
        q.int(p.number);
        q.int(p.layer);
    }
    q.int(-1);
    for p in &w.pickups {
        q.int(p.id);
        q.vec(p.pos);
        q.int(p.type_);
        q.int(p.subtype);
        q.int(p.flags);
        q.int(p.number);
        q.int(p.layer);
        q.vec(p.core);
        q.int(p.is_core_active);
    }
    q
}

/// Weapons and entities in the prediction world (M8.3) against `rs_parity weapons`.
fn check_weapons(name: &str, col: Rc<Collision>, text: &str) {
    use ddnet::items::{LaserData, PickupData, ProjectileData};
    use ddnet::physics::pickup::{Dragger, Pickup, Plasma};
    use ddnet::physics::projectile::Projectile;

    const SORTED: [usize; 4] = [0, 1, 3, 6];
    const START_TICK: i32 = 10000;
    let zones: Vec<Tuning> = (0..NUM_TUNE_ZONES).map(weapon_zone_tuning).collect();
    let num = |s: &str| s.parse::<i32>().unwrap();
    let mut world: Option<GameWorld> = None;
    let (mut ticks, mut runs) = (0, 0);
    let mut tick = 0;
    let (mut seen_events, mut events_checked) = (0, 0);
    let mut mode = 0;
    for line in text.lines() {
        let w: Vec<&str> = line.split(' ').collect();
        match w[0] {
            "run" => {
                mode = num(w[3]);
                tick = 0;
                seen_events = 0;
                runs += 1;
                let config = WorldConfig {
                    is_ddrace: mode != 2 && mode != 3,
                    is_vanilla: mode == 2,
                    is_fng: mode == 3,
                    infinite_ammo: mode != 4,
                    predict_tiles: mode != 2,
                    predict_freeze: mode != 2,
                    predict_weapons: true,
                    predict_ddrace: mode != 2 && mode != 3,
                    is_solo: mode == 4,
                    use_tune_zones: mode != 5,
                    bug_ddrace_input: false,
                    no_weak_hook_and_bounce: mode == 1,
                    old_laser: mode == 1,
                    freeze_delay: 3,
                    antiping_players: 1,
                    predict_events: true,
                    snd_enable: true,
                };
                let mut wd = GameWorld::new(Rc::clone(&col), config);
                wd.tunings = zones.clone();
                wd.game_tick = START_TICK;
                if mode == 1 {
                    wd.teams.set_team(6, 1);
                }
                world = Some(wd);
            }
            "tune0" => {
                let ints: Vec<i32> = w[1..].iter().map(|s| num(s)).collect();
                world.as_mut().unwrap().tunings[0] = Tuning(ints.try_into().unwrap());
            }
            "char" => {
                let wd = world.as_mut().unwrap();
                let (obj, ext) = line.split_once(" |").unwrap();
                let obj: Vec<i32> = obj.split(' ').skip(3).map(num).collect();
                let ext: Vec<i32> = ext.split_whitespace().map(num).collect();
                let (obj, ext) = (
                    ObjCharacter::from_ints(&obj),
                    ObjDDNetCharacter::from_ints(&ext),
                );
                let id = num(w[1]) as usize;
                wd.add_character(id, &obj, &ext);
                wd.character_mut(id).unwrap().game_team = num(w[2]);
                if id == 3 {
                    wd.read_character(id, &obj, &ext, true); // the local player
                }
            }
            "pickup" => {
                let d = PickupData {
                    pos: vec2(bits(w[2]), bits(w[3])),
                    type_: num(w[4]),
                    subtype: num(w[5]),
                    switch_number: num(w[6]),
                    flags: 0,
                };
                world
                    .as_mut()
                    .unwrap()
                    .pickups
                    .push(Pickup::from_data(num(w[1]), &d));
            }
            "dragger" | "plasma" => {
                let wd = world.as_mut().unwrap();
                let from = vec2(bits(w[2]), bits(w[3]));
                let plasma = w[0] == "plasma";
                let d = LaserData {
                    from,
                    to: from,
                    start_tick: if plasma { wd.game_tick } else { START_TICK },
                    extra_info: true,
                    owner: if plasma { num(w[4]) } else { -1 },
                    type_: if plasma {
                        g::LASERTYPE_PLASMA
                    } else {
                        g::LASERTYPE_DRAGGER
                    },
                    switch_number: if plasma { 0 } else { num(w[5]) },
                    subtype: num(w[if plasma { 5 } else { 4 }]),
                    tune_zone: 0,
                    predict: true,
                };
                if plasma {
                    let p = Plasma::from_data(wd, num(w[1]), &d);
                    wd.plasmas.insert(0, p);
                } else {
                    wd.draggers.insert(0, Dragger::from_data(num(w[1]), &d));
                }
            }
            "proj" => {
                let wd = world.as_mut().unwrap();
                let d = ProjectileData {
                    start_pos: vec2(bits(w[2]), bits(w[3])),
                    start_vel: vec2(bits(w[4]), bits(w[5])),
                    type_: num(w[6]),
                    start_tick: num(w[7]),
                    extra_info: w[8] == "1",
                    owner: num(w[9]),
                    explosive: w[10] == "1",
                    bouncing: num(w[11]),
                    freeze: w[12] == "1",
                    switch_number: num(w[13]),
                    tune_zone: 0,
                };
                let p = Projectile::from_data(wd, num(w[1]), &d);
                wd.projectiles.insert(0, p);
            }
            "totals" => {}
            "events" => {
                // the predicted events the tick added (M9 13.3)
                let wd = world.as_ref().unwrap();
                let total: usize = num(w[1]) as usize;
                let new: Vec<String> = wd.predicted_events[seen_events..]
                    .iter()
                    .map(|e| {
                        format!(
                            "{}:{:08x}:{:08x}:{}:{}:{}",
                            e.event_id,
                            e.pos.x.to_bits(),
                            e.pos.y.to_bits(),
                            e.id,
                            e.tick,
                            e.extra_info
                        )
                    })
                    .collect();
                assert_eq!(
                    (wd.predicted_events.len(), new),
                    (
                        total,
                        w[2..].iter().map(|s| s.to_string()).collect::<Vec<_>>()
                    ),
                    "{name}, run {runs} (mode {mode}), tick {tick}: predicted events"
                );
                seen_events = total;
                events_checked += total;
            }
            _ => {
                let wd = world.as_mut().unwrap();
                tick += 1;
                let (inputs, rest) = line.split_once(" |").unwrap();
                let (states, entities) = rest.rsplit_once(" | ").unwrap();
                let v: Vec<i32> = inputs.split(' ').map(num).collect();
                let input = |k: usize| {
                    let v = &v[k * 10..k * 10 + 10];
                    ObjPlayerInput {
                        direction: v[0],
                        jump: v[1],
                        hook: v[2],
                        target_x: v[3],
                        target_y: v[4],
                        fire: v[5],
                        next_weapon: v[6],
                        prev_weapon: v[7],
                        wanted_weapon: v[8],
                        player_flags: v[9],
                    }
                };
                for (k, &id) in SORTED.iter().enumerate() {
                    wd.on_direct_input(id, &input(k));
                }
                wd.game_tick = START_TICK + tick;
                for (k, &id) in SORTED.iter().enumerate() {
                    wd.on_predicted_input(id, &input(k));
                }
                wd.tick();

                let state = |id: usize| {
                    let (ch, c) = wd.character(id).unwrap();
                    weapon_char_state(ch, c, &wd.teams)
                };
                let chars: Vec<String> = SORTED
                    .iter()
                    .map(|&id| {
                        let pos = wd.character(id).unwrap().0.pos;
                        let crc = state(id).crc();
                        format!("{:08x} {:08x} {crc:08x}", pos.x.to_bits(), pos.y.to_bits())
                    })
                    .collect();
                let e = weapon_entity_state(wd);
                let got = format!(
                    "{} | {} {} {} {} {} {:08x}",
                    chars.join(" "),
                    wd.projectiles.len(),
                    wd.lasers.len(),
                    wd.draggers.len(),
                    wd.plasmas.len(),
                    wd.pickups.len(),
                    e.crc()
                );
                let want = format!("{} | {entities}", states.trim());
                assert_eq!(
                    got, want,
                    "{name}, run {runs} (mode {mode}), tick {tick}; entities {:x?}",
                    e.0
                );
                ticks += 1;
            }
        }
    }
    assert_eq!(runs, 30, "{name}");
    assert_eq!(ticks, 30 * 150, "{name}");
    assert!(events_checked > 0, "{name}: no predicted events");
}

#[test]
fn weapons() {
    for name in MAPS {
        let map = load(name);
        let layers = make_movers(Layers::from_map(&map));
        check_weapons(
            name,
            Rc::new(Collision::from_layers(layers)),
            &golden_file(name, "weapons"),
        );
    }
}

/// `DumpNetWorld` of rs_parity.
fn dump_net_world(tag: &str, w: &GameWorld) -> String {
    let chars: Vec<String> = (0..ddnet::physics::teams::MAX_CLIENTS)
        .filter_map(|i| {
            let (ch, c) = w.character(i)?;
            Some(format!(
                "{i}:{:08x}",
                weapon_char_state(ch, c, &w.teams).crc()
            ))
        })
        .collect();
    format!(
        "{tag} {} | {} {} {} {} {} {} {:08x}",
        chars.join(" "),
        w.projectiles.len(),
        w.lasers.len(),
        w.doors().len(),
        w.draggers.len(),
        w.plasmas.len(),
        w.pickups.len(),
        weapon_entity_state(w).crc()
    )
}

/// Snapshot entities into the prediction world (M8.4) against `rs_parity netents`.
fn check_netents(name: &str, col: Rc<Collision>, text: &str) {
    use ddnet::physics::snap::EntityRef;

    const SORTED: [usize; 4] = [0, 1, 3, 6];
    const LOCAL: i32 = 3;
    let zones: Vec<Tuning> = (0..NUM_TUNE_ZONES).map(weapon_zone_tuning).collect();
    let num = |s: &str| s.parse::<i32>().unwrap();
    let mut world: Option<GameWorld> = None;
    let mut teams = TeamsCore::default();
    let mut exts: BTreeMap<usize, (ObjDDNetCharacter, i32)> = BTreeMap::new();
    let mut inputs = [ObjPlayerInput::default(); 4];
    let mut items: Vec<(i32, i32, Vec<i32>, Option<ObjEntityEx>)> = Vec::new();
    let mut netchars: Vec<(usize, ObjCharacter)> = Vec::new();
    let (mut snaps, mut matched) = (0, 0);
    let tick = |w: &mut GameWorld, inputs: &[ObjPlayerInput; 4]| {
        for (k, &id) in SORTED.iter().enumerate() {
            if w.character(id).is_some() {
                w.on_direct_input(id, &inputs[k]);
            }
        }
        w.game_tick += 1;
        for (k, &id) in SORTED.iter().enumerate() {
            if w.character(id).is_some() {
                w.on_predicted_input(id, &inputs[k]);
            }
        }
        w.tick();
    };
    for line in text.lines() {
        let w: Vec<&str> = line.split(' ').collect();
        match w[0] {
            "run" => {
                let mode = num(w[3]);
                let config = WorldConfig {
                    is_ddrace: mode < 2,
                    is_vanilla: mode == 2,
                    is_fng: mode == 3,
                    infinite_ammo: true,
                    predict_tiles: mode != 2,
                    predict_freeze: true,
                    predict_weapons: true,
                    predict_ddrace: mode < 2,
                    use_tune_zones: mode != 1,
                    freeze_delay: 3,
                    antiping_players: 1,
                    ..Default::default()
                };
                let mut wd = GameWorld::new(Rc::clone(&col), config);
                wd.tunings = zones.clone();
                wd.game_tick = 10000;
                teams = TeamsCore::default();
                if mode == 1 {
                    teams.set_team(6, 1);
                }
                wd.teams = teams.clone();
                world = Some(wd);
                inputs = [ObjPlayerInput::default(); 4];
                exts.clear();
            }
            "char" => {
                let wd = world.as_mut().unwrap();
                let (obj, ext) = line.split_once(" |").unwrap();
                let obj =
                    ObjCharacter::from_ints(&obj.split(' ').skip(3).map(num).collect::<Vec<_>>());
                let ext = ObjDDNetCharacter::from_ints(
                    &ext.split_whitespace().map(num).collect::<Vec<_>>(),
                );
                let (id, game_team) = (num(w[1]) as usize, num(w[2]));
                wd.add_character(id, &obj, &ext);
                wd.character_mut(id).unwrap().game_team = game_team;
                if id as i32 == LOCAL {
                    wd.read_character(id, &obj, &ext, true);
                }
                exts.insert(id, (ext, game_team));
            }
            "t" => {
                let v: Vec<i32> = w[1..].iter().map(|s| num(s)).collect();
                for (k, input) in inputs.iter_mut().enumerate() {
                    let v = &v[k * 7..k * 7 + 7];
                    *input = ObjPlayerInput {
                        direction: v[0],
                        jump: v[1],
                        hook: v[2],
                        target_x: v[3],
                        target_y: v[4],
                        fire: v[5],
                        wanted_weapon: v[6],
                        ..Default::default()
                    };
                }
                tick(world.as_mut().unwrap(), &inputs);
            }
            "item" => {
                let (data, ex) = match line.split_once(" | ") {
                    Some((d, e)) => {
                        let e: Vec<i32> = e.split(' ').map(num).collect();
                        (d, Some(ObjEntityEx::from_ints(&e)))
                    }
                    None => (line, None),
                };
                let mut v: Vec<i32> = data.split(' ').skip(3).map(num).collect();
                v.resize(16, 0);
                let type_id = match w[1] {
                    "projectile" => g::NETOBJTYPE_PROJECTILE,
                    "ddraceprojectile" => g::NETOBJTYPE_DDRACEPROJECTILE,
                    "ddnetprojectile" => g::NETOBJTYPE_DDNETPROJECTILE,
                    "pickup" => g::NETOBJTYPE_PICKUP,
                    "ddnetpickup" => g::NETOBJTYPE_DDNETPICKUP,
                    "laser" => g::NETOBJTYPE_LASER,
                    "ddnetlaser" => g::NETOBJTYPE_DDNETLASER,
                    other => panic!("{name}: object type {other}"),
                };
                items.push((type_id, num(w[2]), v, ex));
            }
            "netchar" => {
                let obj =
                    ObjCharacter::from_ints(&w[2..].iter().map(|s| num(s)).collect::<Vec<_>>());
                netchars.push((num(w[1]) as usize, obj));
            }
            "world" => {
                let wd = world.as_mut().unwrap();
                wd.net_obj_begin(teams.clone(), LOCAL);
                for (id, obj) in netchars.drain(..) {
                    let (ext, game_team) = exts[&id];
                    wd.net_char_add(id, &obj, &ext, game_team, id as i32 == LOCAL);
                }
                for (type_id, id, data, ex) in &items {
                    wd.net_obj_add(*id, *type_id, data, ex.as_ref());
                }
                wd.net_obj_end();
                snaps += 1;
                assert_eq!(
                    dump_net_world("world", wd),
                    line,
                    "{name}, snapshot {snaps}"
                );
            }
            "match" => {
                let wd = world.as_ref().unwrap();
                let got: Vec<String> = items
                    .drain(..)
                    .map(
                        |(type_id, id, data, _)| match wd.find_match(id, type_id, &data) {
                            None => "-".into(),
                            Some(r) => {
                                matched += 1;
                                let (t, i) = match r {
                                    EntityRef::Projectile(i) => (0, i),
                                    EntityRef::Laser(i) => (1, i),
                                    EntityRef::Door(i) => (2, i),
                                    EntityRef::Dragger(i) => (3, i),
                                    EntityRef::Plasma(i) => (6, i),
                                    EntityRef::Pickup(i) => (7, i),
                                };
                                format!("{t}:{i}")
                            }
                        },
                    )
                    .collect();
                let got = if got.is_empty() {
                    "match".to_string()
                } else {
                    format!("match {}", got.join(" "))
                };
                assert_eq!(got, line, "{name}, snapshot {snaps}");
            }
            "copy" => {
                let mut copy = world.as_ref().unwrap().copy_world();
                tick(&mut copy, &inputs);
                assert_eq!(
                    dump_net_world("copy", &copy),
                    line,
                    "{name}, snapshot {snaps} (copy)"
                );
            }
            other => panic!("{name}: unknown line {other}"),
        }
    }
    assert_eq!(snaps, 4 * 60, "{name}");
    assert!(matched > 1000, "{name}: {matched} matches");
}

#[test]
fn netents() {
    for name in MAPS {
        let map = load(name);
        let layers = make_movers(Layers::from_map(&map));
        check_netents(
            name,
            Rc::new(Collision::from_layers(layers)),
            &golden_file(name, "netents"),
        );
    }
}

/// `NetWorldInts` of rs_parity: every character and entity of a world.
fn net_world_ints(w: &GameWorld, q: &mut Queries) {
    for i in 0..ddnet::physics::teams::MAX_CLIENTS {
        if let Some((ch, c)) = w.character(i) {
            q.int(i as i32);
            q.int(weapon_char_state(ch, c, &w.teams).crc());
        }
    }
    for n in [
        w.projectiles.len(),
        w.lasers.len(),
        w.doors().len(),
        w.draggers.len(),
        w.plasmas.len(),
        w.pickups.len(),
    ] {
        q.int(n as i32);
    }
    q.int(weapon_entity_state(w).crc());
}

/// rs_parity antiping's `DDNetFields`: what `read_ddnet` sets.
fn ddnet_fields(q: &mut Queries, c: &CharacterCore) {
    for b in [
        c.solo,
        c.jetpack,
        c.collision_disabled,
        c.hammer_hit_disabled,
        c.shotgun_hit_disabled,
        c.grenade_hit_disabled,
        c.laser_hit_disabled,
        c.hook_hit_disabled,
        c.super_,
        c.invincible,
        c.endless_hook,
        c.endless_jump,
        c.deep_frozen,
        c.live_frozen,
        c.has_telegun_grenade,
        c.has_telegun_gun,
        c.has_telegun_laser,
    ] {
        q.int(b);
    }
    for w in &c.weapons {
        q.int(w.got);
    }
    q.int(c.freeze_end);
    q.int(c.jumps);
    q.int(c.jumped_total);
}

/// rs_parity antiping's `PrintState`: what UpdatePrediction and OnPredict leave behind.
/// `seen`: the clients with a DDNetCharacter in a snapshot of this run.
/// `frame`: the timing of a frame line (GetSmoothPos of every client is compared too).
fn antiping_state(
    p: &ddnet::predict::Predictor,
    empty: &GameWorld,
    seen: &[bool],
    frame: Option<&ddnet::net::client::Timing>,
) -> String {
    use ddnet::physics::teams::MAX_CLIENTS;
    let in_world = |i| (p.predicted_world.as_ref()).is_some_and(|w| w.character(i).is_some());
    let mut cores = Queries::default();
    for (i, (prev, cur)) in p.prev_predicted.iter().zip(&p.predicted).enumerate() {
        cores.vec(prev.pos);
        cores.vec(cur.pos);
        if in_world(i) {
            cores.0.extend(core_state(prev).0);
            cores.0.extend(core_state(cur).0);
        } else if seen[i] {
            ddnet_fields(&mut cores, cur);
        }
    }
    let mut pred_pos = Queries::default();
    for i in 0..MAX_CLIENTS {
        for k in 0..200 {
            pred_pos.vec(p.pred_pos[i][k]);
            pred_pos.int(p.pred_pos_tick[i][k]);
        }
    }
    let mut smooth = Queries::default();
    for i in 0..MAX_CLIENTS {
        for j in 0..2 {
            for v in [p.smooth_start[i][j], p.smooth_len[i][j]] {
                smooth.int(v as i32);
                smooth.int((v >> 32) as i32);
            }
        }
        smooth.vec(p.last_pos[i]);
        smooth.int(p.last_active[i]);
        if let Some(t) = frame {
            smooth.vec(p.smooth_pos(i, t));
        }
    }
    let world = |w: Option<&GameWorld>| {
        let mut q = Queries::default();
        net_world_ints(w.unwrap_or(empty), &mut q);
        q.crc()
    };
    // UpdateRenderedCharacters
    let mut render = Queries::default();
    for r in frame.map(|t| p.rendered(t)).unwrap_or_default() {
        render.int(r.id);
        render.0.extend(r.prev.to_ints());
        render.0.extend(r.cur.to_ints());
        render.int(r.predicted);
        render.int(r.predicted_local);
        render.vec(r.pos);
    }
    // CItems: projectile and laser timings in both worlds
    let mut item_time = Queries::default();
    if let Some(t) = frame {
        let local_in_game = t.pred_tick % 3 != 0;
        let last_tick_time = t.pred_intra_tick * 0.02;
        for w in [p.prev_predicted_world.as_ref().unwrap_or(empty), p.world()] {
            for pr in &w.projectiles {
                let ct = p.projectile_time(&pr.get_data(), t, local_in_game, last_tick_time);
                item_time.0.push(ct.to_bits() as i32);
            }
            for l in &w.lasers {
                let d = l.get_data();
                for ty in [d.type_.clamp(-1, g::NUM_LASERTYPES - 1), g::LASERTYPE_DOOR] {
                    for predicted in [false, true] {
                        let (ticks, head) = p.laser_ticks(&d, ty, predicted, t);
                        item_time.0.push(ticks.to_bits() as i32);
                        item_time.0.push(head.to_bits() as i32);
                    }
                }
            }
        }
    }
    format!(
        "{:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {} {:08x} {:08x}",
        cores.crc(),
        pred_pos.crc(),
        smooth.crc(),
        world(Some(p.world())),
        world(p.predicted_world.as_ref()),
        world(p.prev_predicted_world.as_ref()),
        p.predicted_tick(),
        render.crc(),
        item_time.crc()
    )
}

#[test]
fn antiping_timing_matches_cpp() {
    use ddnet::net::client::{prediction_tick, prediction_time, smooth_tick};
    let path = format!(
        "{}/../tests/golden/antiping-timing.txt",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut count = 0;
    for line in text.lines() {
        let (input, want) = line.split_once(" = ").unwrap();
        let w: Vec<&str> = input.split(' ').collect();
        let int = |i: usize| w[i].parse::<i64>().unwrap();
        let (game_now, pred_now) = (int(3), int(4));
        let mix = f32::from_bits(u32::from_str_radix(w[7], 16).unwrap());
        let time = prediction_time(game_now, pred_now);
        let tick = prediction_tick(
            time,
            int(5) as i32,
            int(6) as i32,
            int(1) as i32,
            int(2) as i32,
        );
        let (smooth, intra) = smooth_tick(game_now, pred_now, mix);
        let got = format!("{time} {tick} {smooth} {:08x}", intra.to_bits());
        assert_eq!(got, want, "{input}");
        count += 1;
    }
    assert_eq!(count, 3000);
}

/// The prediction loop (M8.5) against `rs_parity antiping`: snapshots into
/// UpdatePrediction, then frames that predict ahead.
fn check_antiping(name: &str, map: &Map, col: Rc<Collision>, text: &str) {
    use ddnet::net::client::{PredictMsg, Timing};
    use ddnet::predict::{PredictConfig, Predictor, SoundEvent};
    use ddnet::protocol::snapshot::Item;
    let empty = GameWorld::new(Rc::clone(&col), WorldConfig::default());
    let mut p: Option<Predictor> = None;
    let mut inputs: BTreeMap<i32, ObjPlayerInput> = BTreeMap::new();
    let (mut items, mut prev_items): (Vec<Item>, Vec<Item>) = (Vec::new(), Vec::new());
    let (mut tick, mut prev_tick) = (0, None);
    let (mut frames, mut predicted, mut smoothed) = (0, 0, 0);
    let mut seen = [false; ddnet::physics::teams::MAX_CLIENTS];
    for (n, line) in text.lines().enumerate() {
        let (input, want) = line.split_once(" = ").unwrap_or((line, ""));
        let w: Vec<&str> = input.split(' ').collect();
        let v: Vec<i32> = w[1..].iter().map(|s| s.parse().unwrap_or(0)).collect();
        match w[0] {
            "run" => {
                let mut pr = Predictor::new(Rc::clone(&col), map);
                pr.world_mut().tunings = vec![Tuning::default(); NUM_TUNE_ZONES];
                pr.config = PredictConfig {
                    predict: true,
                    antiping: v[2] != 0,
                    antiping_players: v[3],
                    antiping_weapons: v[4] != 0,
                    antiping_preinput: v[5] != 0,
                    antiping_smooth: v[6] != 0,
                    antiping_limit: v[7],
                    antiping_percent: v[8],
                    predict_freeze: v[9],
                    antiping_grenade: v[10] != 0,
                    antiping_gunfire: v[11] != 0,
                    snd_gun: v[12] != 0,
                    snd_long_pain: v[13] != 0,
                    predict_events: v[14] != 0,
                    ..Default::default()
                };
                p = Some(pr);
                inputs.clear();
                items.clear();
                seen = [false; ddnet::physics::teams::MAX_CLIENTS];
                prev_tick = None;
                continue;
            }
            "teams" => {
                p.as_mut().unwrap().on_message(PredictMsg::TeamsState(v));
                continue;
            }
            "pre" => {
                p.as_mut()
                    .unwrap()
                    .on_message(PredictMsg::PreInput(g::MsgSvPreInput {
                        direction: v[0],
                        target_x: v[1],
                        target_y: v[2],
                        jump: v[3],
                        fire: v[4],
                        hook: v[5],
                        wanted_weapon: v[6],
                        next_weapon: v[7],
                        prev_weapon: v[8],
                        owner: v[9],
                        intended_tick: v[10],
                    }));
                continue;
            }
            "snap" => {
                prev_items = std::mem::take(&mut items);
                tick = v[0];
                continue;
            }
            "item" => {
                if v[0] == g::NETOBJTYPE_DDNETCHARACTER {
                    seen[v[1] as usize] = true;
                }
                items.push(Item {
                    type_id: v[0],
                    id: v[1],
                    data: v[2..].to_vec(),
                });
                continue;
            }
            "input" => {
                inputs.insert(v[0], ObjPlayerInput::from_ints(&v[1..]));
                continue;
            }
            _ => {}
        }
        let pr = p.as_mut().unwrap();
        let input_at = |t: i32| inputs.range(..=t).next_back().map(|(_, i)| *i);
        let mut frame = None;
        match w[0] {
            "update" => {
                pr.on_new_snapshot(&items, &prev_items, prev_tick, tick, &input_at);
                prev_tick = Some(tick);
            }
            "frame" => {
                let long = |i: usize| w[i].parse::<i64>().unwrap();
                let float = |i: usize| f32::from_bits(u32::from_str_radix(w[i], 16).unwrap());
                let t = Timing {
                    game_tick: v[0],
                    prev_game_tick: v[1],
                    pred_tick: v[2],
                    game_now: long(4),
                    pred_now: long(5),
                    now: long(6),
                    intra_tick: float(7),
                    pred_intra_tick: float(8),
                };
                pr.new_tick = w[9] == "1";
                pr.intra_tick = t.intra_tick;
                // CClient::Update: only sane predictions (MaxLatencyTicks 50)
                if t.pred_tick > t.game_tick && t.pred_tick < t.game_tick + 50 {
                    let windows = pr.smooth_start.clone();
                    pr.predict(&t, &input_at);
                    predicted += 1;
                    smoothed += (pr.smooth_start != windows) as u32;
                }
                frames += 1;
                frame = Some(t);
            }
            other => panic!("unknown line {other}"),
        }
        let mut got = antiping_state(pr, &empty, &seen, frame.as_ref());
        // the sounds played (CSounds/CMapSounds calls and CEffects' own)
        for e in std::mem::take(&mut pr.sound_events) {
            let (m, id, pos) = match e {
                SoundEvent::World(id, pos) => ("", id, pos),
                SoundEvent::MapWorld(id, pos) => ("m", id, pos),
            };
            got += &format!(" {m}{id}:{:08x}:{:08x}", pos.x.to_bits(), pos.y.to_bits());
        }
        if got != want {
            // compare with `RS_VERBOSE=1 rs_parity antiping` (stderr)
            let w = pr.world();
            for i in 0..ddnet::physics::teams::MAX_CLIENTS {
                if let Some((ch, c)) = w.character(i) {
                    let q = weapon_char_state(ch, c, &w.teams).0;
                    let q: Vec<String> = q.iter().map(|v| v.to_string()).collect();
                    eprintln!("# {i} {}", q.join(" "));
                }
            }
        }
        assert_eq!(got, want, "{name}:{}: {input}", n + 1);
    }
    assert!(
        frames > 1000 && predicted * 10 > frames * 9 && smoothed > 20,
        "{frames} {predicted} {smoothed}"
    );
}

#[test]
fn antiping_loop_matches_cpp() {
    for name in MAPS {
        let map = load(name);
        let col = Rc::new(Collision::new(&map));
        check_antiping(name, &map, col, &golden_file(name, "antiping"));
    }
}

/// The antiping gates (gameclient.cpp:955-997, `AntiPingPlayers/Grenade/Weapons/Gunfire`)
/// for every switch combination, spectating or not.
#[test]
fn antiping_gates() {
    use ddnet::predict::{PredictConfig, Predictor};
    let map = load("coverage");
    let mut p = Predictor::new(Rc::new(Collision::new(&map)), &map);
    for bits in 0..32u32 {
        let on = |b: u32| bits & (1 << b) != 0;
        for players in 0..4 {
            p.config = PredictConfig {
                antiping: on(0),
                antiping_grenade: on(1),
                antiping_weapons: on(2),
                antiping_gunfire: on(3),
                antiping_players: players,
                ..Default::default()
            };
            p.spectating = on(4);
            let active = on(0) && !on(4);
            let want_players = if active && players != 0 { players } else { 0 };
            assert_eq!(p.antiping_players(), want_players, "{bits} {players}");
            assert_eq!(p.antiping_grenade(), active && on(1));
            assert_eq!(p.antiping_weapons(), active && on(2));
            assert_eq!(p.antiping_gunfire(), active && on(1) && on(2) && on(3));
        }
    }
}
