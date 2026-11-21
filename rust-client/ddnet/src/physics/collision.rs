//! Tile collision. C++: `CCollision` (`src/game/collision.cpp`), the parts the game core
//! and the client prediction use. Weapon-only queries (`IntersectLine`, `MovePoint`,
//! the laser intersections) come with weapons.

use std::cell::Cell;
use std::collections::BTreeMap;

use super::vmath::{Vec2, direction, distance, mix, round_to_int, trunc_i32, vec2};
use crate::map::Map;

// Tile indices and flags. C++: game/mapitems.h
pub const TILE_SOLID: i32 = 1;
pub const TILE_DEATH: i32 = 2;
pub const TILE_NOHOOK: i32 = 3;
pub const TILE_NOLASER: i32 = 4;
pub const TILE_THROUGH_CUT: i32 = 5;
pub const TILE_THROUGH: i32 = 6;
pub const TILE_JUMP: i32 = 7;
pub const TILE_FREEZE: i32 = 9;
pub const TILE_TELEINEVIL: i32 = 10;
pub const TILE_TELEINHOOK: i32 = 15;
pub const TILE_SUBTRACT_TIME: i32 = 95;
pub const TILE_TELEIN: i32 = 26;
pub const TILE_TELEOUT: i32 = 27;
pub const TILE_TELECHECK: i32 = 29;
pub const TILE_TELECHECKIN: i32 = 31;
pub const TILE_STOP: i32 = 60;
pub const TILE_STOPS: i32 = 61;
pub const TILE_STOPA: i32 = 62;
pub const TILE_TELECHECKINEVIL: i32 = 63;
pub const TILE_THROUGH_ALL: i32 = 66;
pub const TILE_THROUGH_DIR: i32 = 67;
pub const TILE_ALLOW_TELE_GUN: i32 = 98;
pub const TILE_ALLOW_BLUE_TELE_GUN: i32 = 99;
pub const TILE_NPH_ENABLE: i32 = 107;
pub const TILE_TELE_LASER_DISABLE: i32 = 129;
pub const TILE_LFREEZE: i32 = 144;
pub const TILE_LUNFREEZE: i32 = 145;
pub const TILEFLAG_XFLIP: i32 = 1;
pub const TILEFLAG_YFLIP: i32 = 2;
pub const TILEFLAG_ROTATE: i32 = 8;
pub const ROTATION_0: i32 = 0;
pub const ROTATION_90: i32 = TILEFLAG_ROTATE;
pub const ROTATION_180: i32 = TILEFLAG_XFLIP | TILEFLAG_YFLIP;
pub const ROTATION_270: i32 = TILEFLAG_XFLIP | TILEFLAG_YFLIP | TILEFLAG_ROTATE;

/// C++: `ENTITY_OFFSET` (mapitems.h).
const ENTITY_OFFSET: i32 = 255 - 16 * 4;
/// C++: `LAYER_GAME`, `LAYER_FRONT`, `LAYER_SWITCH`: the layers `entity` reads.
pub const LAYER_GAME: i32 = 0;
pub const LAYER_FRONT: i32 = 1;
pub const LAYER_SWITCH: i32 = 4;

pub const CANTMOVE_LEFT: i32 = 1 << 0;
pub const CANTMOVE_RIGHT: i32 = 1 << 1;
pub const CANTMOVE_UP: i32 = 1 << 2;
pub const CANTMOVE_DOWN: i32 = 1 << 3;

/// C++: `CTile` (only the fields physics reads).
#[derive(Clone, Copy, Debug, Default)]
pub struct Tile {
    pub index: u8,
    pub flags: u8,
}

/// C++: `CTeleTile`.
#[derive(Clone, Copy, Debug, Default)]
pub struct TeleTile {
    pub number: u8,
    pub type_: u8,
}

/// C++: `CSpeedupTile`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SpeedupTile {
    pub force: u8,
    pub max_speed: u8,
    pub type_: u8,
    pub angle: i16,
}

/// C++: `CSwitchTile`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SwitchTile {
    pub number: u8,
    pub type_: u8,
    pub flags: u8,
    pub delay: u8,
}

/// C++: `CTuneTile`.
#[derive(Clone, Copy, Debug, Default)]
pub struct TuneTile {
    pub number: u8,
    pub type_: u8,
}

/// C++: `CDoorTile`.
#[derive(Clone, Copy, Debug, Default)]
pub struct DoorTile {
    pub index: u8,
    pub flags: u8,
    pub number: u8,
}

/// C++: `ClampVel` (collision.cpp:19).
pub fn clamp_vel(move_restriction: i32, mut vel: Vec2) -> Vec2 {
    if vel.x > 0.0 && move_restriction & CANTMOVE_RIGHT != 0 {
        vel.x = 0.0;
    }
    if vel.x < 0.0 && move_restriction & CANTMOVE_LEFT != 0 {
        vel.x = 0.0;
    }
    if vel.y > 0.0 && move_restriction & CANTMOVE_DOWN != 0 {
        vel.y = 0.0;
    }
    if vel.y < 0.0 && move_restriction & CANTMOVE_UP != 0 {
        vel.y = 0.0;
    }
    vel
}

/// C++: `ThroughOffset` (collision.cpp:1117).
fn through_offset(pos0: Vec2, pos1: Vec2) -> (i32, i32) {
    let (x, y) = (pos0.x - pos1.x, pos0.y - pos1.y);
    if x.abs() > y.abs() {
        if x < 0.0 { (-32, 0) } else { (32, 0) }
    } else if y < 0.0 {
        (0, -32)
    } else {
        (0, 32)
    }
}

const MR_DIRECTIONS: [Vec2; 5] = [
    vec2(0.0, 0.0),
    vec2(1.0, 0.0),
    vec2(0.0, 1.0),
    vec2(-1.0, 0.0),
    vec2(0.0, -1.0),
];
const MR_DIR_HERE: usize = 0;

/// C++: `GetMoveRestrictionsRaw` (collision.cpp:196).
fn move_restrictions_raw(tile: i32, flags: i32) -> i32 {
    let flags = flags & (TILEFLAG_XFLIP | TILEFLAG_YFLIP | TILEFLAG_ROTATE);
    match tile {
        TILE_STOP => match flags {
            ROTATION_0 => CANTMOVE_DOWN,
            ROTATION_90 => CANTMOVE_LEFT,
            ROTATION_180 => CANTMOVE_UP,
            ROTATION_270 => CANTMOVE_RIGHT,
            f if f == TILEFLAG_YFLIP ^ ROTATION_0 => CANTMOVE_UP,
            f if f == TILEFLAG_YFLIP ^ ROTATION_90 => CANTMOVE_RIGHT,
            f if f == TILEFLAG_YFLIP ^ ROTATION_180 => CANTMOVE_DOWN,
            f if f == TILEFLAG_YFLIP ^ ROTATION_270 => CANTMOVE_LEFT,
            _ => 0,
        },
        TILE_STOPS => match flags {
            f if [
                ROTATION_0,
                ROTATION_180,
                TILEFLAG_YFLIP ^ ROTATION_0,
                TILEFLAG_YFLIP ^ ROTATION_180,
            ]
            .contains(&f) =>
            {
                CANTMOVE_DOWN | CANTMOVE_UP
            }
            f if [
                ROTATION_90,
                ROTATION_270,
                TILEFLAG_YFLIP ^ ROTATION_90,
                TILEFLAG_YFLIP ^ ROTATION_270,
            ]
            .contains(&f) =>
            {
                CANTMOVE_LEFT | CANTMOVE_RIGHT
            }
            _ => 0,
        },
        TILE_STOPA => CANTMOVE_LEFT | CANTMOVE_RIGHT | CANTMOVE_UP | CANTMOVE_DOWN,
        _ => 0,
    }
}

/// C++: static `GetMoveRestrictions` (collision.cpp:249): stoppers only block moving
/// *onto* them, except one-way stoppers, which also block while standing on them.
fn move_restrictions(direction: usize, tile: i32, flags: i32) -> i32 {
    let result = move_restrictions_raw(tile, flags);
    if direction == MR_DIR_HERE && tile == TILE_STOP {
        return result;
    }
    const MASKS: [i32; 5] = [0, CANTMOVE_RIGHT, CANTMOVE_DOWN, CANTMOVE_LEFT, CANTMOVE_UP];
    result & MASKS[direction]
}

fn parse<T>(
    data: Option<Vec<u8>>,
    count: usize,
    size: usize,
    f: impl Fn(&[u8]) -> T,
) -> Option<Vec<T>> {
    Some(data?.chunks_exact(size).take(count).map(f).collect())
}

/// C++: `CCollision`.
pub struct Collision {
    width: i32,
    height: i32,
    tiles: Vec<Tile>,
    front: Option<Vec<Tile>>,
    tele: Option<Vec<TeleTile>>,
    speedup: Option<Vec<SpeedupTile>>,
    switch: Option<Vec<SwitchTile>>,
    tune: Option<Vec<TuneTile>>,
    /// Written by doors in the prediction world (C++ mutates the shared grid too).
    door: Option<Vec<Cell<DoorTile>>>,
    pub highest_switch_number: i32,
    /// Tele-out positions by tele number - 1. C++ also lists tele-ins, check-outs and
    /// other tele tiles; only the server reads those.
    tele_outs: BTreeMap<i32, Vec<Vec2>>,
}

/// The physics tile arrays `CCollision::Init` reads from the map (each sized like the
/// game layer); absent layers and layers whose data is invalid are `None`.
pub struct Layers {
    pub width: i32,
    pub height: i32,
    pub tiles: Vec<Tile>,
    pub front: Option<Vec<Tile>>,
    pub tele: Option<Vec<TeleTile>>,
    pub speedup: Option<Vec<SpeedupTile>>,
    pub switch: Option<Vec<SwitchTile>>,
    pub tune: Option<Vec<TuneTile>>,
    /// C++ allocates doors whenever a switch layer exists, even if its data is invalid.
    pub has_switch_layer: bool,
}

impl Layers {
    pub fn from_map(map: &Map) -> Self {
        let (width, height) = (map.game.width, map.game.height);
        let count = (width * height) as usize;
        let tile = |c: &[u8]| Tile {
            index: c[0],
            flags: c[1],
        };
        let data = |layer: Option<crate::map::Layer>| layer.and_then(|l| map.data(l.data_index));
        Self {
            width,
            height,
            tiles: parse(map.data(map.game.data_index), count, 4, tile)
                .expect("game layer data is validated by Map::load"),
            front: parse(data(map.front), count, 4, tile),
            tele: parse(data(map.tele), count, 2, |c| TeleTile {
                number: c[0],
                type_: c[1],
            }),
            speedup: parse(data(map.speedup), count, 6, |c| SpeedupTile {
                force: c[0],
                max_speed: c[1],
                type_: c[2],
                angle: i16::from_le_bytes([c[4], c[5]]),
            }),
            switch: parse(data(map.switch), count, 4, |c| SwitchTile {
                number: c[0],
                type_: c[1],
                flags: c[2],
                delay: c[3],
            }),
            tune: parse(data(map.tune), count, 2, |c| TuneTile {
                number: c[0],
                type_: c[1],
            }),
            has_switch_layer: map.switch.is_some(),
        }
    }
}

impl Collision {
    pub fn new(map: &Map) -> Self {
        Self::from_layers(Layers::from_map(map))
    }

    /// C++: `CCollision::Init` (collision.cpp:51).
    pub fn from_layers(l: Layers) -> Self {
        let (width, height) = (l.width, l.height);
        let count = (width * height) as usize;
        let mut c = Self {
            width,
            height,
            tiles: l.tiles,
            front: l.front,
            tele: l.tele,
            speedup: l.speedup,
            switch: l.switch,
            tune: l.tune,
            door: l
                .has_switch_layer
                .then(|| vec![Cell::new(DoorTile::default()); count]),
            highest_switch_number: 0,
            tele_outs: BTreeMap::new(),
        };

        if let (Some(switch), Some(door)) = (&mut c.switch, &mut c.door) {
            for (s, d) in switch.iter_mut().zip(door.iter()) {
                c.highest_switch_number = c.highest_switch_number.max(s.number as i32);
                d.set(DoorTile {
                    number: s.number,
                    ..DoorTile::default()
                });
                // keep only switchable tile types below TILE_NPH_ENABLE
                let index = s.type_ as i32;
                if index <= TILE_NPH_ENABLE
                    && !((TILE_JUMP..=TILE_SUBTRACT_TIME).contains(&index)
                        || index == TILE_ALLOW_TELE_GUN
                        || index == TILE_ALLOW_BLUE_TELE_GUN)
                {
                    s.type_ = 0;
                }
            }
        }

        if let Some(tele) = &c.tele {
            for (i, t) in tele.iter().enumerate() {
                if t.number == 0 || t.type_ as i32 != TILE_TELEOUT {
                    continue;
                }
                let i = i as i32;
                let pos = vec2(
                    (i % width) as f32 * 32.0 + 16.0,
                    (i / width) as f32 * 32.0 + 16.0,
                );
                c.tele_outs
                    .entry(t.number as i32 - 1)
                    .or_default()
                    .push(pos);
            }
        }
        c
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    /// C++: `CCollision::GetTile`: solid-ish tile at a world position, else 0.
    pub fn get_tile(&self, x: i32, y: i32) -> i32 {
        let nx = (x / 32).clamp(0, self.width - 1);
        let ny = (y / 32).clamp(0, self.height - 1);
        let index = self.tiles[(ny * self.width + nx) as usize].index as i32;
        if (TILE_SOLID..=TILE_NOLASER).contains(&index) {
            index
        } else {
            0
        }
    }

    /// C++: `CCollision::GetFrontTile`.
    pub fn get_front_tile(&self, x: i32, y: i32) -> i32 {
        let Some(front) = &self.front else { return 0 };
        let nx = (x / 32).clamp(0, self.width - 1);
        let ny = (y / 32).clamp(0, self.height - 1);
        let index = front[(ny * self.width + nx) as usize].index as i32;
        if index == TILE_DEATH || index == TILE_NOLASER {
            index
        } else {
            0
        }
    }

    pub fn is_solid(&self, x: i32, y: i32) -> bool {
        let index = self.get_tile(x, y);
        index == TILE_SOLID || index == TILE_NOHOOK
    }

    pub fn check_point(&self, pos: Vec2) -> bool {
        self.is_solid(round_to_int(pos.x), round_to_int(pos.y))
    }

    pub fn get_collision_at(&self, x: f32, y: f32) -> i32 {
        self.get_tile(round_to_int(x), round_to_int(y))
    }

    pub fn get_front_collision_at(&self, x: f32, y: f32) -> i32 {
        self.get_front_tile(round_to_int(x), round_to_int(y))
    }

    /// C++: `CCollision::GetPureMapIndex`.
    pub fn get_pure_map_index(&self, pos: Vec2) -> i32 {
        let nx = (round_to_int(pos.x) / 32).clamp(0, self.width - 1);
        let ny = (round_to_int(pos.y) / 32).clamp(0, self.height - 1);
        ny * self.width + nx
    }

    pub fn get_tile_index(&self, index: i32) -> i32 {
        if index < 0 {
            0
        } else {
            self.tiles[index as usize].index as i32
        }
    }

    pub fn get_tile_flags(&self, index: i32) -> i32 {
        if index < 0 {
            0
        } else {
            self.tiles[index as usize].flags as i32
        }
    }

    pub fn get_front_tile_index(&self, index: i32) -> i32 {
        match &self.front {
            Some(front) if index >= 0 => front[index as usize].index as i32,
            _ => 0,
        }
    }

    pub fn get_front_tile_flags(&self, index: i32) -> i32 {
        match &self.front {
            Some(front) if index >= 0 => front[index as usize].flags as i32,
            _ => 0,
        }
    }

    fn tele_at(&self, index: i32) -> Option<TeleTile> {
        Some(self.tele.as_ref()?[usize::try_from(index).ok()?])
    }

    /// C++: `CCollision::IsTeleCheckpoint`.
    pub fn is_tele_checkpoint(&self, index: i32) -> i32 {
        self.tele_at(index)
            .filter(|t| t.type_ as i32 == TILE_TELECHECK)
            .map_or(0, |t| t.number as i32)
    }

    /// C++: `CCollision::IsTeleportHook`.
    pub fn is_teleport_hook(&self, index: i32) -> i32 {
        self.tele_at(index)
            .filter(|t| t.type_ as i32 == TILE_TELEINHOOK)
            .map_or(0, |t| t.number as i32)
    }

    /// C++: `CCollision::IsSpeedup` (asserts `index >= 0`).
    pub fn is_speedup(&self, index: i32) -> bool {
        self.speedup
            .as_ref()
            .is_some_and(|s| s[index as usize].force > 0)
    }

    /// C++: `CCollision::GetSpeedup`: (direction, force, max speed, type), or `None`
    /// where C++ leaves the outputs untouched.
    pub fn get_speedup(&self, index: i32) -> Option<(Vec2, i32, i32, i32)> {
        let s = self.speedup.as_ref()?[usize::try_from(index).ok()?];
        let angle = s.angle as i32 as f32 * (super::vmath::PI / 180.0);
        Some((
            direction(angle),
            s.force as i32,
            s.max_speed as i32,
            s.type_ as i32,
        ))
    }

    /// C++: `CCollision::IsTune`.
    pub fn is_tune(&self, index: i32) -> i32 {
        let Some(t) = self
            .tune
            .as_ref()
            .zip(usize::try_from(index).ok())
            .map(|(t, i)| t[i])
        else {
            return 0;
        };
        if t.type_ != 0 { t.number as i32 } else { 0 }
    }

    fn switch_at(&self, index: i32) -> Option<SwitchTile> {
        Some(self.switch.as_ref()?[usize::try_from(index).ok()?]).filter(|s| s.type_ > 0)
    }

    pub fn get_switch_type(&self, index: i32) -> i32 {
        self.switch_at(index).map_or(0, |s| s.type_ as i32)
    }

    pub fn get_switch_number(&self, index: i32) -> i32 {
        self.switch_at(index)
            .filter(|s| s.number > 0 && s.number as i32 <= self.highest_switch_number)
            .map_or(0, |s| s.number as i32)
    }

    pub fn get_switch_delay(&self, index: i32) -> i32 {
        self.switch_at(index).map_or(0, |s| s.delay as i32)
    }

    /// C++: `CCollision::GetDoorTile`.
    pub fn get_door_tile(&self, index: i32) -> DoorTile {
        match &self.door {
            Some(door) if index >= 0 && door[index as usize].get().index != 0 => {
                door[index as usize].get()
            }
            _ => DoorTile::default(),
        }
    }

    /// C++: `CCollision::Entity` for `LAYER_GAME`, `LAYER_FRONT` and `LAYER_SWITCH`.
    /// `None` when the map has no such layer (callers check `FrontLayer()`/`SwitchLayer()`
    /// first in C++).
    pub fn entity(&self, x: i32, y: i32, layer: i32) -> Option<i32> {
        let has_layer = match layer {
            LAYER_GAME => true,
            LAYER_FRONT => self.front.is_some(),
            LAYER_SWITCH => self.switch.is_some(),
            _ => false,
        };
        if !has_layer {
            return None;
        }
        if x < 0 || x >= self.width || y < 0 || y >= self.height {
            return Some(0);
        }
        let i = (y * self.width + x) as usize;
        let tile = match layer {
            LAYER_FRONT => self.front.as_ref()?[i].index,
            LAYER_SWITCH => self.switch.as_ref()?[i].type_,
            _ => self.tiles[i].index,
        };
        Some(tile as i32 - ENTITY_OFFSET)
    }

    /// C++: `CCollision::SetDoorCollisionAt`.
    pub fn set_door_collision_at(&self, x: f32, y: f32, type_: u8, flags: u8, number: u8) {
        let Some(door) = &self.door else { return };
        let nx = (round_to_int(x) / 32).clamp(0, self.width - 1);
        let ny = (round_to_int(y) / 32).clamp(0, self.height - 1);
        door[(ny * self.width + nx) as usize].set(DoorTile {
            index: type_,
            flags,
            number,
        });
    }

    /// C++: `CCollision::TeleOuts`.
    pub fn tele_outs(&self, number: i32) -> &[Vec2] {
        self.tele_outs.get(&number).map_or(&[], |v| &v[..])
    }

    /// C++: `CCollision::GetMoveRestrictions` (collision.cpp:262). `switch_active` is the
    /// C++ callback (`None` for `nullptr`).
    pub fn get_move_restrictions(
        &self,
        switch_active: Option<&dyn Fn(u8) -> bool>,
        pos: Vec2,
        distance: f32,
        override_center_tile_index: i32,
    ) -> i32 {
        assert!(
            (0.0..=32.0).contains(&distance),
            "Invalid Distance: {distance}"
        );
        let mut restrictions = 0;
        for (d, dir) in MR_DIRECTIONS.iter().enumerate() {
            let mod_pos = pos + *dir * distance;
            let mut index = self.get_pure_map_index(mod_pos);
            if d == MR_DIR_HERE && override_center_tile_index >= 0 {
                index = override_center_tile_index;
            }
            restrictions |=
                move_restrictions(d, self.get_tile_index(index), self.get_tile_flags(index));
            restrictions |= move_restrictions(
                d,
                self.get_front_tile_index(index),
                self.get_front_tile_flags(index),
            );
            if let Some(switch_active) = switch_active {
                let door = self.get_door_tile(index);
                if door.number as i32 <= self.highest_switch_number && switch_active(door.number) {
                    restrictions |= move_restrictions(d, door.index as i32, door.flags as i32);
                }
            }
        }
        restrictions
    }

    /// C++: `CCollision::TestBox`.
    pub fn test_box(&self, pos: Vec2, size: Vec2) -> bool {
        let size = size * 0.5;
        self.check_point(vec2(pos.x - size.x, pos.y - size.y))
            || self.check_point(vec2(pos.x + size.x, pos.y - size.y))
            || self.check_point(vec2(pos.x - size.x, pos.y + size.y))
            || self.check_point(vec2(pos.x + size.x, pos.y + size.y))
    }

    /// C++: `CCollision::IsOnGround`.
    pub fn is_on_ground(&self, pos: Vec2, size: f32) -> bool {
        self.check_point(vec2(pos.x + size / 2.0, pos.y + size / 2.0 + 5.0))
            || self.check_point(vec2(pos.x - size / 2.0, pos.y + size / 2.0 + 5.0))
    }

    /// C++: `CCollision::MoveBox` (collision.cpp:528). Returns whether the box landed on
    /// the ground (C++ `*pGrounded = true`; it never resets it).
    pub fn move_box(&self, pos: &mut Vec2, vel: &mut Vec2, size: Vec2, elasticity: Vec2) -> bool {
        let mut grounded = false;
        let distance = super::vmath::length(*vel);
        let max = trunc_i32(distance);
        if distance > 0.00001 {
            let fraction = 1.0 / (max + 1) as f32;
            let elasticity_x = elasticity.x.clamp(-1.0, 1.0);
            let elasticity_y = elasticity.y.clamp(-1.0, 1.0);
            for _ in 0..=max {
                // early break once obstacles reduced the speed to exactly 0
                if *vel == vec2(0.0, 0.0) {
                    break;
                }
                let mut new_pos = *pos + *vel * fraction;
                // a very small fraction can have no effect
                if new_pos == *pos {
                    break;
                }
                if self.test_box(new_pos, size) {
                    let mut hits = 0;
                    if self.test_box(vec2(pos.x, new_pos.y), size) {
                        if elasticity_y > 0.0 && vel.y > 0.0 {
                            grounded = true;
                        }
                        new_pos.y = pos.y;
                        vel.y *= -elasticity_y;
                        hits += 1;
                    }
                    if self.test_box(vec2(new_pos.x, pos.y), size) {
                        new_pos.x = pos.x;
                        vel.x *= -elasticity_x;
                        hits += 1;
                    }
                    // neither of the tests got a collision: a real corner case
                    if hits == 0 {
                        if elasticity_y > 0.0 && vel.y > 0.0 {
                            grounded = true;
                        }
                        new_pos.y = pos.y;
                        vel.y *= -elasticity_y;
                        new_pos.x = pos.x;
                        vel.x *= -elasticity_x;
                    }
                }
                *pos = new_pos;
            }
        }
        grounded
    }

    /// C++: `CCollision::IsThrough` (collision.cpp:611).
    fn is_through(
        &self,
        x: i32,
        y: i32,
        offset_x: i32,
        offset_y: i32,
        pos0: Vec2,
        pos1: Vec2,
    ) -> bool {
        let index = self.get_pure_map_index(vec2(x as f32, y as f32)) as usize;
        if let Some(front) = &self.front {
            let f = front[index];
            let (fi, ff) = (f.index as i32, f.flags as i32);
            if fi == TILE_THROUGH_ALL || fi == TILE_THROUGH_CUT {
                return true;
            }
            if fi == TILE_THROUGH_DIR
                && ((ff == ROTATION_0 && pos0.y > pos1.y)
                    || (ff == ROTATION_90 && pos0.x < pos1.x)
                    || (ff == ROTATION_180 && pos0.y < pos1.y)
                    || (ff == ROTATION_270 && pos0.x > pos1.x))
            {
                return true;
            }
        }
        let offset_index = self.get_pure_map_index(vec2(
            x.wrapping_add(offset_x) as f32,
            y.wrapping_add(offset_y) as f32,
        )) as usize;
        self.tiles[offset_index].index as i32 == TILE_THROUGH
            || self
                .front
                .as_ref()
                .is_some_and(|f| f[offset_index].index as i32 == TILE_THROUGH)
    }

    /// C++: `CCollision::IsHookBlocker` (collision.cpp:622).
    fn is_hook_blocker(&self, x: i32, y: i32, pos0: Vec2, pos1: Vec2) -> bool {
        let index = self.get_pure_map_index(vec2(x as f32, y as f32)) as usize;
        let blocks_dir = |t: Tile| {
            let f = t.flags as i32;
            t.index as i32 == TILE_THROUGH_DIR
                && ((f == ROTATION_0 && pos0.y < pos1.y)
                    || (f == ROTATION_90 && pos0.x > pos1.x)
                    || (f == ROTATION_180 && pos0.y > pos1.y)
                    || (f == ROTATION_270 && pos0.x < pos1.x))
        };
        let tile = self.tiles[index];
        let front = self.front.as_ref().map(|f| f[index]);
        tile.index as i32 == TILE_THROUGH_ALL
            || front.is_some_and(|f| f.index as i32 == TILE_THROUGH_ALL)
            || blocks_dir(tile)
            || front.is_some_and(blocks_dir)
    }

    /// C++: `CCollision::IntersectLineTeleHook` (collision.cpp:357) with
    /// `sv_old_teleport_hook 0` (its default). Returns (hit, collision point, point
    /// before it, tele number).
    pub fn intersect_line_tele_hook(&self, pos0: Vec2, pos1: Vec2) -> (i32, Vec2, Vec2, i32) {
        let distance = distance(pos0, pos1);
        let end = trunc_i32(distance + 1.0);
        let mut last = pos0;
        let (dx, dy) = through_offset(pos0, pos1);
        let mut i = 0;
        while i <= end {
            let a = i as f32 / end as f32;
            let pos = mix(pos0, pos1, a);
            let (ix, iy) = (round_to_int(pos.x), round_to_int(pos.y));
            let tele_nr = self.is_teleport_hook(self.get_pure_map_index(pos));
            if tele_nr != 0 {
                return (TILE_TELEINHOOK, pos, last, tele_nr);
            }
            let mut hit = 0;
            if self.check_point(vec2(ix as f32, iy as f32)) {
                if !self.is_through(ix, iy, dx, dy, pos0, pos1) {
                    hit = self.get_collision_at(ix as f32, iy as f32);
                }
            } else if self.is_hook_blocker(ix, iy, pos0, pos1) {
                hit = TILE_NOHOOK;
            }
            if hit != 0 {
                return (hit, pos, last, 0);
            }
            last = pos;
            i += 1;
        }
        (0, pos1, pos1, 0)
    }

    /// C++: `CCollision::TileExists` (collision.cpp:840).
    fn tile_exists(&self, index: i32) -> bool {
        if index < 0 {
            return false;
        }
        let i = index as usize;
        let ddrace = |t: i32| {
            (TILE_FREEZE..=TILE_TELE_LASER_DISABLE).contains(&t)
                || (TILE_LFREEZE..=TILE_LUNFREEZE).contains(&t)
        };
        ddrace(self.tiles[i].index as i32)
            || self
                .front
                .as_ref()
                .is_some_and(|f| ddrace(f[i].index as i32))
            || self.tele.as_ref().is_some_and(|t| {
                [
                    TILE_TELEIN,
                    TILE_TELEINEVIL,
                    TILE_TELECHECKINEVIL,
                    TILE_TELECHECK,
                    TILE_TELECHECKIN,
                ]
                .contains(&(t[i].type_ as i32))
            })
            || self.speedup.as_ref().is_some_and(|s| s[i].force > 0)
            || self.door.as_ref().is_some_and(|d| d[i].get().index != 0)
            || self.switch.as_ref().is_some_and(|s| s[i].type_ != 0)
            || self.tune.as_ref().is_some_and(|t| t[i].type_ != 0)
            || self.tile_exists_next(index)
    }

    /// C++: `CCollision::TileExistsNext` (collision.cpp:862), quirks included: the
    /// "left" and "above" neighbours fall back to the tile itself at index 0 and on the
    /// first row (`> 0`, not `>= 0`), and `Flags | ROTATION_180 | ROTATION_0` is always
    /// true.
    fn tile_exists_next(&self, index: i32) -> bool {
        let n = self.width * self.height;
        let left = (if index - 1 > 0 { index - 1 } else { index }) as usize;
        let right = (if index + 1 < n { index + 1 } else { index }) as usize;
        let below = (if index + self.width < n {
            index + self.width
        } else {
            index
        }) as usize;
        let above = (if index - self.width > 0 {
            index - self.width
        } else {
            index
        }) as usize;
        // the same conditions for the game, front and door layers (C++ orders them
        // differently per layer, which does not change the result)
        let stoppers = |at: &dyn Fn(usize) -> (i32, i32)| -> bool {
            let stop = |i: usize, rotation: i32| at(i) == (TILE_STOP, rotation);
            stop(right, ROTATION_270)
                || stop(left, ROTATION_90)
                || stop(below, ROTATION_0)
                || stop(above, ROTATION_180)
                || [right, left, below, above]
                    .iter()
                    .any(|&i| at(i).0 == TILE_STOPA || at(i).0 == TILE_STOPS)
        };
        stoppers(&|i| (self.tiles[i].index as i32, self.tiles[i].flags as i32))
            || self
                .front
                .as_ref()
                .is_some_and(|f| stoppers(&|i| (f[i].index as i32, f[i].flags as i32)))
            || self
                .door
                .as_ref()
                .is_some_and(|d| stoppers(&|i| (d[i].get().index as i32, d[i].get().flags as i32)))
    }

    /// C++: `CCollision::GetMapIndex` (collision.cpp:911).
    pub fn get_map_index(&self, pos: Vec2) -> i32 {
        let nx = tile_index_from_coord(pos.x, self.width);
        let ny = tile_index_from_coord(pos.y, self.height);
        let index = ny * self.width + nx;
        if self.tile_exists(index) { index } else { -1 }
    }

    /// C++: `CCollision::GetMapIndices` (collision.cpp:923).
    pub fn get_map_indices(&self, prev_pos: Vec2, pos: Vec2, max_indices: usize) -> Vec<i32> {
        let mut indices = Vec::new();
        let d = distance(prev_pos, pos);
        let end = trunc_i32(d + 1.0);
        let index_at = |p: Vec2| {
            let nx = (trunc_i32(p.x) / 32).clamp(0, self.width - 1);
            let ny = (trunc_i32(p.y) / 32).clamp(0, self.height - 1);
            ny * self.width + nx
        };
        if d == 0.0 {
            let index = index_at(pos);
            if self.tile_exists(index) {
                indices.push(index);
            }
            return indices;
        }
        let mut last_index = 0;
        for i in 0..end {
            let a = i as f32 / d;
            let index = index_at(mix(prev_pos, pos, a));
            if self.tile_exists(index) && last_index != index {
                if max_indices != 0 && indices.len() > max_indices {
                    return indices;
                }
                indices.push(index);
                last_index = index;
            }
        }
        indices
    }
}

/// C++: `TileIndexFromCoord` (collision.cpp:904), in double precision like C++.
fn tile_index_from_coord(coord: f32, size: i32) -> i32 {
    if coord <= 0.0 || coord.is_nan() {
        return 0;
    }
    let v = (coord as f64 / 32.0).min((size - 1) as f64);
    if (-2_147_483_648.0..2_147_483_648.0).contains(&v) {
        v as i32
    } else {
        i32::MIN
    }
}
