//! Parity with the C++ map loading and physics code, using data written by the C++
//! tool `src/tools/rs_parity.cpp` into `rust-client/tests/golden/maps/`. Maps are read
//! from the repository's `data/maps/`, not copied.
//!
//! Regenerate (repo root, after building the `rs_parity` CMake target), per map:
//!     build-rs/rs_parity.exe map "data/maps/<name>.map" rust-client/tests/golden/maps/<name>.txt <scratch dir>
//! with spaces in <name> replaced by '_' in the output file name.

use ddnet::map::{Layer, Map};
use ddnet::physics::collision::{Collision, Layers, clamp_vel};
use ddnet::physics::core::{CharacterCore, WorldCore};
use ddnet::physics::teams::NUM_DDRACE_TEAMS;
use ddnet::physics::teams::TeamsCore;
use ddnet::physics::tuning::{Param, Tuning};
use ddnet::physics::vmath::{PI, Vec2, direction, vec2};
use ddnet::physics::world::{
    Character, GameWorld, NUM_TUNE_ZONES, TUNE_ZONE_OVERRIDE_NONE, WorldConfig,
};
use ddnet::protocol::generated::{self as g, ObjCharacter, ObjDDNetCharacter, ObjPlayerInput};

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
        check_collision(
            name,
            &map,
            &Collision::from_layers(hook_tele_layers(&map)),
            &golden_file(name, "collision-hooktele"),
        );
    }
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
fn check_world(name: &str, col: &Collision, text: &str) {
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
        };
        let mut world = GameWorld::new(col, config);
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
            &world_collision(Layers::from_map(&map)),
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
            &world_collision(hook_tele_layers(&map)),
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
/// `net_obj_add_laser`, `net_obj_end`): the collision door grid after each snapshot
/// must equal `rs_parity doors`.
#[test]
fn doors() {
    for name in MAPS {
        let col = Collision::from_layers(front_door_layers(&load(name)));
        let text = golden_file(name, "doors");
        let mut world = GameWorld::new(&col, WorldConfig::default());
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
                world.net_obj_add_laser(v[0], type_id, &data, ex.as_ref());
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
