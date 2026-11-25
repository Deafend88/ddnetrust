//! C++: `CLaser` of the client prediction (prediction/entities/laser.cpp): laser rifle and
//! DDRace shotgun beams with their bounces.

use super::core::SERVER_TICK_SPEED;
use super::tuning::Param;
use super::vmath::{Vec2, distance, normalize, trunc_i32, vec2};
use super::weapons::SV_HIT;
use super::world::GameWorld;
use crate::protocol::generated as g;

/// C++: `CLaser`.
#[derive(Clone, Debug, PartialEq)]
pub struct Laser {
    /// C++: `CEntity::m_Id`: the snapshot id, -1 for a locally fired laser.
    pub id: i32,
    pub pos: Vec2,
    pub from: Vec2,
    pub dir: Vec2,
    pub prev_pos: Vec2,
    pub energy: f32,
    pub bounces: i32,
    pub eval_tick: i32,
    pub owner: i32,
    /// `WEAPON_LASER` or `WEAPON_SHOTGUN`.
    pub type_: i32,
    pub zero_energy_bounce_in_last_tick: bool,
    pub tune_zone: i32,
    pub marked_for_destroy: bool,
}

/// C++: `StackedLaserShotgunBugSpeed` (laser.cpp:35).
const STACKED_LASER_SHOTGUN_BUG_SPEED: Vec2 = Vec2 {
    x: -2147483648.0,
    y: -2147483648.0,
};

impl GameWorld {
    /// C++: `new CLaser(World, Pos, Direction, StartEnergy, Owner, Type)` (laser.cpp:15):
    /// inserted at the front, then the first bounce.
    pub(super) fn spawn_laser(
        &mut self,
        pos: Vec2,
        dir: Vec2,
        energy: f32,
        owner: i32,
        type_: i32,
    ) {
        let tune_zone = if self.config.use_tune_zones {
            self.col.is_tune(self.col.get_map_index(pos))
        } else {
            0
        };
        let mut l = Laser {
            id: -1,
            pos,
            from: vec2(0.0, 0.0),
            dir,
            prev_pos: vec2(0.0, 0.0),
            energy: if self.config.is_fng && energy < 10.0 {
                800.0
            } else {
                energy
            },
            bounces: 0,
            eval_tick: 0,
            owner,
            type_,
            zero_energy_bounce_in_last_tick: false,
            tune_zone,
            marked_for_destroy: false,
        };
        self.lasers.insert(0, l.clone());
        self.do_bounce(&mut l);
        self.lasers[0] = l;
    }

    /// C++: `CLaser::HitCharacter` (laser.cpp:33).
    fn laser_hit_character(&mut self, l: &mut Laser, from: Vec2, to: Vec2) -> bool {
        let owner = self.char_id(l.owner);
        let cfg = self.config;
        let dont_hit_self = (cfg.old_laser || !cfg.is_ddrace) || l.bounces == 0;
        let not_this = if dont_hit_self { owner } else { None };
        let core = |w: &GameWorld, i: usize| w.core.characters[i].as_ref().unwrap().clone();
        let can_hit = match owner {
            Some(o) => {
                let c = core(self, o);
                (!c.laser_hit_disabled && l.type_ == g::WEAPON_LASER)
                    || (!c.shotgun_hit_disabled && l.type_ == g::WEAPON_SHOTGUN)
            }
            None => SV_HIT,
        };
        let this_only = if can_hit { None } else { owner };
        let hit = self.intersect_character(l.pos, to, 0.0, not_this, l.owner, this_only);
        let Some((hit, at)) = hit else {
            return false;
        };
        let refuse = match owner {
            Some(o) if Some(hit) != owner => {
                let c = core(self, o);
                (c.laser_hit_disabled && l.type_ == g::WEAPON_LASER)
                    || (c.shotgun_hit_disabled && l.type_ == g::WEAPON_SHOTGUN)
            }
            _ => !SV_HIT,
        };
        if (Some(hit) == owner && cfg.old_laser) || refuse {
            return false;
        }
        l.from = from;
        l.pos = at;
        l.energy = -1.0;
        if l.type_ == g::WEAPON_SHOTGUN {
            let strength = self.tunings[l.tune_zone as usize].get(Param::ShotgunStrength);
            let hit_pos = self.core.characters[hit].as_ref().unwrap().pos;
            if !cfg.old_laser {
                if l.prev_pos != hit_pos {
                    self.add_velocity(hit, normalize(l.prev_pos - hit_pos) * strength);
                } else {
                    self.core.characters[hit].as_mut().unwrap().vel =
                        STACKED_LASER_SHOTGUN_BUG_SPEED;
                }
            } else if let Some(o) = owner {
                let owner_pos = self.core.characters[o].as_ref().unwrap().pos;
                if owner_pos != hit_pos {
                    self.add_velocity(hit, normalize(owner_pos - hit_pos) * strength);
                } else {
                    self.core.characters[hit].as_mut().unwrap().vel =
                        STACKED_LASER_SHOTGUN_BUG_SPEED;
                }
            } else {
                // re-apply move restrictions as a part of 'shotgun bug' reproduction
                self.apply_move_restrictions(hit);
            }
            if let Some(o) = owner {
                self.anti_ping_interference(o, hit as i32, false, false);
            }
        } else if l.type_ == g::WEAPON_LASER {
            self.unfreeze(hit);
        }
        true
    }

    /// C++: `CLaser::DoBounce` (laser.cpp:96). The C++ `Res == -1` branches (a temporary
    /// `SetCollisionAt`) are unreachable and left out (DECISIONS.md, 8.2).
    pub(super) fn do_bounce(&mut self, l: &mut Laser) {
        l.eval_tick = self.game_tick;
        if l.energy < 0.0 {
            l.marked_for_destroy = true;
            return;
        }
        l.prev_pos = l.pos;
        let to = l.pos + l.dir * l.energy;
        let (res, _coltile, to) = self.col.intersect_line_tele_weapon(l.pos, to);
        if res != 0 {
            if !self.laser_hit_character(l, l.pos, to) {
                // intersected
                l.from = l.pos;
                l.pos = to;
                let mut temp_pos = l.pos;
                let mut temp_dir = l.dir * 4.0;
                self.col.move_point(&mut temp_pos, &mut temp_dir, 1.0);
                l.pos = temp_pos;
                l.dir = normalize(temp_dir);
                let distance = distance(l.from, l.pos);
                // prevent infinite bounces
                if distance == 0.0 && l.zero_energy_bounce_in_last_tick {
                    l.energy = -1.0;
                } else {
                    l.energy -=
                        distance + self.tunings[l.tune_zone as usize].get(Param::LaserBounceCost);
                }
                l.zero_energy_bounce_in_last_tick = distance == 0.0;
                l.bounces += 1;
                let bounce_num =
                    trunc_i32(self.tunings[l.tune_zone as usize].get(Param::LaserBounceNum));
                if l.bounces > bounce_num {
                    l.energy = -1.0;
                }
            }
        } else if !self.laser_hit_character(l, l.pos, to) {
            l.from = l.pos;
            l.pos = to;
            l.energy = -1.0;
        }
    }

    /// C++: `CLaser::Tick` (laser.cpp:171).
    pub(super) fn tick_laser(&mut self, i: usize) {
        let mut l = self.lasers[i].clone();
        let delay = self.tunings[l.tune_zone as usize].get(Param::LaserBounceDelay);
        let ticks = SERVER_TICK_SPEED as f32 * delay / 1000.0;
        let due = if self.config.is_vanilla {
            self.game_tick as f32 > l.eval_tick as f32 + ticks
        } else {
            ((self.game_tick - l.eval_tick) as f32) > ticks
        };
        if due {
            self.do_bounce(&mut l);
        }
        self.lasers[i] = l;
    }
}
