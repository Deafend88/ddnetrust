//! C++: `CDoor` (`src/game/client/prediction/entities/door.cpp`): a door laser from the
//! snapshot whose move restrictions go into the collision door grid shared by every
//! world.

use super::collision::{Collision, LAYER_FRONT, LAYER_GAME, LAYER_SWITCH, TILE_STOPA};
use super::vmath::{PI, Vec2, length, round_to_int, trunc_i32, vec2};

const ENTITY_LASER_SHORT: i32 = 19;
const ENTITY_LASER_LONG: i32 = 21;
const ENTITY_DOOR: i32 = 49;
/// C++: `s_aSideOffsetX/Y`: the eight tiles around a door tile in
/// `IGameController::OnEntity` order.
const SIDE_OFFSETS: [(i32, i32); 8] = [
    (0, 1),
    (1, 1),
    (1, 0),
    (1, -1),
    (0, -1),
    (-1, -1),
    (-1, 0),
    (-1, 1),
];

#[derive(Clone, Debug)]
pub struct Door {
    pub id: i32,
    pub pos: Vec2,
    to: Vec2,
    pub number: i32,
    direction: Vec2,
    length: i32,
    active: bool,
    pub marked_for_destroy: bool,
}

impl Door {
    /// C++: `CDoor::CDoor` + `Read`: the laser object is flipped.
    pub fn new(id: i32, from: Vec2, to: Vec2, number: i32) -> Self {
        Self {
            id,
            pos: to,
            to: from,
            number,
            direction: vec2(0.0, 0.0),
            length: 0,
            active: false,
            marked_for_destroy: false,
        }
    }

    /// C++: `CDoor::Match`.
    pub fn matches(&self, other: &Door) -> bool {
        other.pos == self.pos && other.to == self.to && other.number == self.number
    }

    /// C++: `CDoor::MapSide`.
    fn map_side(&self) -> i32 {
        let offset = (
            round_to_int(self.direction.x),
            round_to_int(self.direction.y),
        );
        SIDE_OFFSETS
            .iter()
            .position(|&o| o == offset)
            .map_or(-1, |s| s as i32)
    }

    /// C++: `CDoor::MapLength`: the length the map configures next to the door tile.
    fn map_length(&self, col: &Collision, side: i32) -> i32 {
        let (nx, ny) = (round_to_int(self.pos.x) / 32, round_to_int(self.pos.y) / 32);
        let (ox, oy) = SIDE_OFFSETS[side as usize];
        let mut length = -1;
        for layer in [LAYER_GAME, LAYER_FRONT, LAYER_SWITCH] {
            if col.entity(nx, ny, layer) != Some(ENTITY_DOOR) {
                continue;
            }
            let index = col.entity(nx + ox, ny + oy, layer).unwrap_or(0);
            if !(ENTITY_LASER_SHORT..=ENTITY_LASER_LONG).contains(&index) {
                continue;
            }
            let candidate = 32 * 3 + 32 * (index - ENTITY_LASER_SHORT) * 3;
            if length != -1 && length != candidate {
                return -1;
            }
            length = candidate;
        }
        length
    }

    /// The positions the door covers: `m_Pos + m_Direction * i` up to a solid tile.
    fn span<'c>(&self, col: &'c Collision) -> impl Iterator<Item = Vec2> + 'c {
        let (pos, direction) = (self.pos, self.direction);
        (0..self.length - 1)
            .map(move |i| pos + direction * i as f32)
            .take_while(|&p| !col.check_point(p))
    }

    /// C++: `CDoor::ResetCollision`.
    pub fn reset_collision(&mut self, col: &Collision) {
        // C++ passes the float position to the int parameters of GetTile/GetFrontTile
        let (x, y) = (trunc_i32(self.pos.x), trunc_i32(self.pos.y));
        if col.get_tile(x, y) != 0 || col.get_front_tile(x, y) != 0 {
            return;
        }
        self.active = true;
        let dir = self.to - self.pos;
        let dir_length = length(dir);
        self.direction = if dir_length > 0.0 {
            vec2(dir.x / dir_length, dir.y / dir_length) // C++: normalize_pre_length
        } else {
            vec2(0.0, 0.0)
        };
        let side = if dir_length > 0.0 {
            self.map_side()
        } else {
            -1
        };
        let configured = if side == -1 {
            -1
        } else {
            self.map_length(col, side)
        };
        if configured != -1 {
            self.length = configured;
            let angle = PI / 4.0 * side as f32;
            self.direction = vec2(angle.sin(), angle.cos());
        } else {
            // C++ refuses doors longer than the map, so a hostile server cannot stall it
            let max_length = (col.width() + col.height()) as f32 * 32.0;
            if dir_length <= 0.0 || dir_length > max_length {
                self.active = false;
                return;
            }
            self.length = round_to_int(dir_length);
        }
        for p in self.span(col) {
            col.set_door_collision_at(p.x, p.y, TILE_STOPA as u8, 0, self.number as u8);
        }
    }

    /// C++: `CDoor::Destroy`.
    pub fn destroy(&self, col: &Collision) {
        if self.active {
            for p in self.span(col) {
                col.set_door_collision_at(p.x, p.y, 0, 0, 0);
            }
        }
    }
}

/// C++: `CGameWorld::ResetDoorCollision`: in map order, and a switch door after a
/// game/front-layer door on the same tile (stable, so list order breaks other ties).
pub fn reset_door_collision(doors: &mut [Door], col: &Collision) {
    let mut order: Vec<usize> = (0..doors.len()).collect();
    order.sort_by_key(|&i| (col.get_pure_map_index(doors[i].pos), doors[i].number > 0));
    for i in order {
        doors[i].reset_collision(col);
    }
}
