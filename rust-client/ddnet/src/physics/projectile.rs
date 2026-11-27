//! C++: `CProjectile` of the client prediction (prediction/entities/projectile.cpp):
//! gun, shotgun (vanilla) and grenade bullets, plus the freeze and bouncing projectiles of
//! DDRace turrets that come from snapshots.

use super::core::SERVER_TICK_SPEED;
use super::tuning::{Param, Tuning};
use super::vmath::{Vec2, trunc_i32};
use super::weapons::SV_HIT;
use super::world::GameWorld;
use crate::items::{ProjectileData, calc_pos};
use crate::protocol::generated as g;

/// C++: `LAYER_GAME`, `LAYER_SWITCH` (mapitems.h).
pub const LAYER_GAME: i32 = 0;
pub const LAYER_SWITCH: i32 = 4;

/// C++: `CProjectile`.
#[derive(Clone, Debug, PartialEq)]
pub struct Projectile {
    /// C++: `CEntity::m_Id`: the snapshot id, -1 for a locally fired projectile.
    pub id: i32,
    pub pos: Vec2,
    pub direction: Vec2,
    pub life_span: i32,
    pub owner: i32,
    pub type_: i32,
    pub start_tick: i32,
    pub explosive: bool,
    /// C++ `m_SoundImpact`: played where an explosive projectile explodes (-1 none).
    pub sound_impact: i32,
    pub layer: i32,
    pub number: i32,
    pub bouncing: i32,
    pub freeze: bool,
    pub tune_zone: i32,
    pub marked_for_destroy: bool,
}

impl Projectile {
    /// C++: `CProjectile::GetPos`.
    /// C++: `CProjectile::GetData` (projectile.cpp:214): what the renderer draws.
    pub fn get_data(&self) -> ProjectileData {
        ProjectileData {
            start_pos: self.pos,
            start_vel: self.direction,
            type_: self.type_,
            start_tick: self.start_tick,
            extra_info: true,
            owner: self.owner,
            explosive: self.explosive,
            bouncing: self.bouncing,
            freeze: self.freeze,
            switch_number: self.number,
            tune_zone: self.tune_zone,
        }
    }

    pub fn get_pos(&self, tunings: &[Tuning], time: f32) -> Vec2 {
        let t = &tunings[self.tune_zone as usize];
        let (curvature, speed) = match self.type_ {
            g::WEAPON_GRENADE => (t.get(Param::GrenadeCurvature), t.get(Param::GrenadeSpeed)),
            g::WEAPON_SHOTGUN => (t.get(Param::ShotgunCurvature), t.get(Param::ShotgunSpeed)),
            g::WEAPON_GUN => (t.get(Param::GunCurvature), t.get(Param::GunSpeed)),
            _ => (0.0, 0.0),
        };
        calc_pos(self.pos, self.direction, curvature, speed, time)
    }

    /// C++: `CProjectile(CGameWorld *, int Id, const CProjectileData *)` (projectile.cpp:170),
    /// a projectile from a snapshot. Not inserted into the world.
    pub fn from_data(w: &GameWorld, id: i32, d: &ProjectileData) -> Self {
        let direction = d.start_vel;
        let (owner, explosive, bouncing, freeze) = if d.extra_info {
            (d.owner, d.explosive, d.bouncing, d.freeze)
        } else {
            let unit = (1.0 - super::vmath::length(direction)).abs() < 0.015;
            (-1, d.type_ == g::WEAPON_GRENADE && unit, 0, false)
        };
        let t = &w.tunings[d.tune_zone as usize];
        let lifetime = match d.type_ {
            g::WEAPON_GRENADE => {
                trunc_i32(t.get(Param::GrenadeLifetime) * SERVER_TICK_SPEED as f32)
            }
            g::WEAPON_GUN => trunc_i32(t.get(Param::GunLifetime) * SERVER_TICK_SPEED as f32),
            g::WEAPON_SHOTGUN if !w.config.is_ddrace => {
                trunc_i32(t.get(Param::ShotgunLifetime) * SERVER_TICK_SPEED as f32)
            }
            _ => 20 * SERVER_TICK_SPEED,
        };
        Self {
            id,
            pos: d.start_pos,
            direction,
            life_span: lifetime - (w.game_tick - d.start_tick),
            owner,
            type_: d.type_,
            start_tick: d.start_tick,
            explosive,
            sound_impact: if d.type_ == g::WEAPON_GRENADE {
                crate::protocol::sounds::SOUND_GRENADE_EXPLODE
            } else {
                -1
            },
            layer: if d.switch_number > 0 {
                LAYER_SWITCH
            } else {
                LAYER_GAME
            },
            number: d.switch_number,
            bouncing,
            freeze,
            tune_zone: d.tune_zone,
            marked_for_destroy: false,
        }
    }
}

impl GameWorld {
    /// C++: `new CProjectile(...)` (projectile.cpp:15), which inserts it at the front.
    #[allow(clippy::too_many_arguments)] // the C++ constructor
    pub(super) fn spawn_projectile(
        &mut self,
        type_: i32,
        owner: i32,
        pos: Vec2,
        direction: Vec2,
        span: i32,
        freeze: bool,
        explosive: bool,
        sound_impact: i32,
        layer: i32,
    ) {
        let tune_zone = if self.config.use_tune_zones {
            self.col.is_tune(self.col.get_map_index(pos))
        } else {
            0
        };
        self.projectiles.insert(
            0,
            Projectile {
                id: -1,
                pos,
                direction,
                life_span: span,
                owner,
                type_,
                start_tick: self.game_tick,
                explosive,
                sound_impact,
                layer,
                number: 0,
                bouncing: 0,
                freeze,
                tune_zone,
                marked_for_destroy: false,
            },
        );
    }

    /// C++: `CProjectile::Tick` (projectile.cpp:75).
    pub(super) fn tick_projectile(&mut self, i: usize) {
        let mut p = self.projectiles[i].clone();
        let tick_speed = SERVER_TICK_SPEED as f32;
        let pt = (self.game_tick - p.start_tick - 1) as f32 / tick_speed;
        let ct = (self.game_tick - p.start_tick) as f32 / tick_speed;
        let prev_pos = p.get_pos(&self.tunings, pt);
        let cur_pos = p.get_pos(&self.tunings, ct);
        let (collide, mut col_pos, new_pos) = self.col.intersect_line(prev_pos, cur_pos);
        let owner_char = self.char_id(p.owner);

        let mut target = None;
        if !self.config.is_solo || (p.type_ == g::WEAPON_SHOTGUN && self.config.is_ddrace) {
            let radius = if p.freeze { 1.0 } else { 6.0 };
            if let Some((t, at)) =
                self.intersect_character(prev_pos, col_pos, radius, owner_char, p.owner, None)
            {
                target = Some(t);
                col_pos = at;
            }
        }

        if p.life_span > -1 {
            p.life_span -= 1;
        }

        let weapon_collide =
            owner_char.is_some() && target.is_some_and(|t| !self.can_collide(t, p.owner));
        let hits_target = target.is_some_and(|t| match owner_char {
            Some(o) => {
                !self.core.characters[o]
                    .as_ref()
                    .unwrap()
                    .grenade_hit_disabled
            }
            None => SV_HIT || p.owner == -1 || Some(t) == owner_char,
        });
        if (hits_target || collide != 0 || self.game_layer_clipped(cur_pos)) && !weapon_collide {
            if p.explosive
                && (target.is_none() || !p.freeze || (p.type_ == g::WEAPON_SHOTGUN && collide != 0))
            {
                let team = target.map_or(-1, |t| self.teams.team(t as i32));
                self.create_explosion(col_pos, p.owner, p.type_, p.owner == -1, team, p.start_tick);
                if self.config.is_ddrace && self.config.predict_ddrace {
                    // vanilla has different projectile physics
                    self.sound(col_pos, p.sound_impact, p.start_tick);
                }
            } else if p.freeze {
                for c in self.find_characters(cur_pos, 1.0) {
                    let team = self.teams.team(c as i32) as usize;
                    let switchers = &self.core.switchers;
                    if p.layer != LAYER_SWITCH
                        || (p.number > 0
                            && (p.number as usize) < switchers.len()
                            && switchers[p.number as usize].status[team])
                    {
                        self.freeze(c);
                    }
                }
            }
            if collide != 0 && p.bouncing != 0 {
                p.start_tick = self.game_tick;
                p.pos = new_pos + (-(p.direction * 4.0));
                if p.bouncing == 1 {
                    p.direction.x = -p.direction.x;
                } else if p.bouncing == 2 {
                    p.direction.y = -p.direction.y;
                }
                if p.direction.x.abs() < 1e-6 {
                    p.direction.x = 0.0;
                }
                if p.direction.y.abs() < 1e-6 {
                    p.direction.y = 0.0;
                }
                p.pos += p.direction;
            } else if p.type_ == g::WEAPON_GUN {
                if self.config.is_ddrace && self.config.predict_ddrace {
                    let angle = -p.direction.x.atan2(p.direction.y);
                    self.damage_ind_event(cur_pos, angle, 10, p.start_tick);
                }
                p.marked_for_destroy = true;
            } else if !p.freeze {
                p.marked_for_destroy = true;
            }
        }
        if p.life_span == -1 {
            if p.explosive {
                let owner_char = self.char_id(p.owner);
                let team = owner_char.map_or(-1, |o| self.teams.team(o as i32));
                self.create_explosion(col_pos, p.owner, p.type_, p.owner == -1, team, p.start_tick);
                self.sound(col_pos, p.sound_impact, p.start_tick);
            }
            p.marked_for_destroy = true;
        }
        self.projectiles[i] = p;
    }

    /// C++: `CEntity::GameLayerClipped` (entity.cpp:38).
    pub(super) fn game_layer_clipped(&self, pos: Vec2) -> bool {
        use super::vmath::round_to_int;
        let (x, y) = (round_to_int(pos.x) / 32, round_to_int(pos.y) / 32);
        x < -200 || x > self.col.width() + 200 || y < -200 || y > self.col.height() + 200
    }
}
