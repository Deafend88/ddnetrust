//! C++: `CPickup`, `CDragger` and `CPlasma` of the client prediction
//! (prediction/entities/{pickup,dragger,plasma}.cpp). All three come from snapshots.

use super::core::{NUM_WEAPONS, PHYSICAL_SIZE, SERVER_TICK_SPEED};
use super::projectile::{LAYER_GAME, LAYER_SWITCH};
use super::vmath::{Vec2, distance, normalize, trunc_i32, vec2};
use super::world::GameWorld;
use crate::items::{LaserData, PickupData};
use crate::protocol::generated as g;
use crate::protocol::sounds::*;

/// C++: `PICKUP_PHYSICS_RADIUS` and `CPickup::ms_CollisionExtraSize`.
const PICKUP_PHYSICS_RADIUS: f32 = 14.0;
const PICKUP_COLLISION_EXTRA_SIZE: f32 = 6.0;
/// C++: `g_Config.m_SvDraggerRange` (server setting, never sent: the client default).
const SV_DRAGGER_RANGE: i32 = 700;
/// C++: `PLASMA_ACCEL` (plasma.cpp:10).
const PLASMA_ACCEL: f32 = 1.1;

/// C++: `CPickup`.
#[derive(Clone, Debug, PartialEq)]
pub struct Pickup {
    pub id: i32,
    pub pos: Vec2,
    pub type_: i32,
    pub subtype: i32,
    pub flags: i32,
    pub number: i32,
    pub layer: i32,
    /// C++: `m_Core`, the mover speed.
    pub core: Vec2,
    pub is_core_active: bool,
    pub marked_for_destroy: bool,
}

impl Pickup {
    /// C++: `CPickup(CGameWorld *, int Id, const CPickupData *)` (pickup.cpp:168).
    pub fn from_data(id: i32, d: &PickupData) -> Self {
        Self {
            id,
            pos: d.pos,
            type_: d.type_,
            subtype: d.subtype,
            flags: d.flags,
            number: d.switch_number,
            layer: if d.switch_number > 0 {
                LAYER_SWITCH
            } else {
                LAYER_GAME
            },
            core: vec2(0.0, 0.0),
            is_core_active: false,
            marked_for_destroy: false,
        }
    }
}

/// C++: `CDragger`.
#[derive(Clone, Debug, PartialEq)]
pub struct Dragger {
    pub id: i32,
    pub pos: Vec2,
    pub core: Vec2,
    pub strength: f32,
    pub ignore_walls: bool,
    pub target_id: i32,
    pub number: i32,
    pub layer: i32,
    pub marked_for_destroy: bool,
}

impl Dragger {
    /// C++: `CDragger(CGameWorld *, int Id, const CLaserData *)` (dragger.cpp:131).
    pub fn from_data(id: i32, d: &LaserData) -> Self {
        let (ignore_walls, strength) = if (0..g::NUM_LASERDRAGGERTYPES).contains(&d.subtype) {
            (d.subtype & 1 != 0, ((d.subtype >> 1) + 1) as f32)
        } else {
            (false, 0.0)
        };
        Self {
            id,
            pos: d.from,
            core: vec2(0.0, 0.0),
            strength,
            ignore_walls,
            target_id: d.owner,
            number: d.switch_number,
            layer: if d.switch_number > 0 {
                LAYER_SWITCH
            } else {
                LAYER_GAME
            },
            marked_for_destroy: false,
        }
    }
}

/// C++: `CPlasma`.
#[derive(Clone, Debug, PartialEq)]
pub struct Plasma {
    pub id: i32,
    pub pos: Vec2,
    pub core: Vec2,
    pub freeze: bool,
    pub explosive: bool,
    pub for_client_id: i32,
    pub eval_tick: i32,
    pub life_time: i32,
    pub number: i32,
    pub layer: i32,
    pub marked_for_destroy: bool,
}

impl Plasma {
    /// C++: `CPlasma(CGameWorld *, int Id, const CLaserData *)` (plasma.cpp:12): aimed at
    /// its target, or destroyed without one.
    pub fn from_data(w: &GameWorld, id: i32, d: &LaserData) -> Self {
        let (explosive, freeze) = if (0..g::NUM_LASERGUNTYPES).contains(&d.subtype) {
            (d.subtype & 1 != 0, d.subtype & 2 != 0)
        } else {
            (false, false)
        };
        let mut p = Self {
            id,
            pos: d.from,
            core: vec2(0.0, 0.0),
            freeze,
            explosive,
            for_client_id: d.owner,
            eval_tick: d.start_tick,
            life_time: trunc_i32(SERVER_TICK_SPEED as f32 * 1.5),
            number: d.switch_number,
            layer: if d.switch_number > 0 {
                LAYER_SWITCH
            } else {
                LAYER_GAME
            },
            marked_for_destroy: false,
        };
        match w.char_id(p.for_client_id) {
            Some(t) => p.core = normalize(w.chars[t].as_ref().unwrap().pos - p.pos),
            None => p.marked_for_destroy = true,
        }
        p
    }
}

impl GameWorld {
    /// C++: `(int)(GameTickSpeed() * 0.15f)`, the mover and dragger period.
    fn every_015s(&self) -> bool {
        self.game_tick % trunc_i32(SERVER_TICK_SPEED as f32 * 0.15) == 0
    }

    /// C++: `CPickup::Tick` (pickup.cpp:15) with `CPickup::Move`.
    pub(super) fn tick_pickup(&mut self, i: usize) {
        let mut p = self.pickups[i].clone();
        if self.every_015s() {
            if let Some((_, speed)) = self.col.mover_speed(trunc_i32(p.pos.x), trunc_i32(p.pos.y)) {
                p.core = speed;
                p.is_core_active = true;
            }
            p.pos += p.core;
        }
        let cfg = self.config;
        let ddrace = cfg.is_ddrace && cfg.predict_ddrace;
        let radius = PICKUP_PHYSICS_RADIUS + PICKUP_COLLISION_EXTRA_SIZE;
        for c in self.find_characters(p.pos, radius) {
            let pos = self.chars[c].as_ref().unwrap().pos;
            // pickup distance is shorter on vanilla due to using ClosestEntity
            if cfg.is_vanilla && distance(p.pos, pos) >= radius * 2.0 {
                continue;
            }
            let team = self.teams.team(c as i32);
            let switchers = &self.core.switchers;
            if p.layer == LAYER_SWITCH
                && p.number > 0
                && (p.number as usize) < switchers.len()
                && !switchers[p.number as usize].status[team as usize]
            {
                continue;
            }
            let team_super = self.teams.team_super();
            match p.type_ {
                g::POWERUP_FREEZE => {
                    if cfg.predict_ddrace && self.freeze(c) {
                        self.sound(p.pos, SOUND_PICKUP_HEALTH, c as i32);
                    }
                }
                g::POWERUP_ARMOR => {
                    if !ddrace || self.core.characters[c].as_ref().unwrap().super_ {
                        continue;
                    }
                    self.with_char(c, |ch, core, w| {
                        let mut sound = false;
                        for j in g::WEAPON_SHOTGUN as usize..NUM_WEAPONS {
                            if core.weapons[j].got {
                                core.weapons[j].got = false;
                                core.weapons[j].ammo = 0;
                                sound = true;
                            }
                        }
                        core.ninja.activation_dir = vec2(0.0, 0.0);
                        core.ninja.activation_tick = -500;
                        core.ninja.current_move_time = 0;
                        if sound {
                            ch.last_weapon = g::WEAPON_GUN;
                            w.sound(p.pos, SOUND_PICKUP_ARMOR, ch.id);
                        }
                        if core.active_weapon >= g::WEAPON_SHOTGUN {
                            ch.set_active_weapon(core, w, g::WEAPON_HAMMER);
                        }
                    });
                }
                g::POWERUP_ARMOR_SHOTGUN | g::POWERUP_ARMOR_GRENADE | g::POWERUP_ARMOR_LASER => {
                    if !ddrace || team == team_super {
                        continue;
                    }
                    let weapon = match p.type_ {
                        g::POWERUP_ARMOR_SHOTGUN => g::WEAPON_SHOTGUN,
                        g::POWERUP_ARMOR_GRENADE => g::WEAPON_GRENADE,
                        _ => g::WEAPON_LASER,
                    };
                    self.with_char(c, |ch, core, w| {
                        let slot = &mut core.weapons[weapon as usize];
                        if slot.got {
                            slot.got = false;
                            slot.ammo = 0;
                            ch.last_weapon = g::WEAPON_GUN;
                            w.sound(p.pos, SOUND_PICKUP_ARMOR, ch.id);
                        }
                        if core.active_weapon == weapon {
                            ch.set_active_weapon(core, w, g::WEAPON_HAMMER);
                        }
                    });
                }
                g::POWERUP_ARMOR_NINJA => {
                    if !ddrace || team == team_super {
                        continue;
                    }
                    let ninja = &mut self.core.characters[c].as_mut().unwrap().ninja;
                    ninja.activation_dir = vec2(0.0, 0.0);
                    ninja.activation_tick = -500;
                    ninja.current_move_time = 0;
                }
                g::POWERUP_WEAPON => {
                    let sub = p.subtype;
                    if (0..NUM_WEAPONS as i32).contains(&sub) {
                        let w = self.core.characters[c].as_ref().unwrap().weapons[sub as usize];
                        if !w.got || w.ammo != -1 {
                            self.with_char(c, |ch, core, w| ch.give_weapon(core, w, sub, false));
                            if ddrace {
                                let sound = match sub {
                                    g::WEAPON_GRENADE => Some(SOUND_PICKUP_GRENADE),
                                    g::WEAPON_SHOTGUN | g::WEAPON_LASER => {
                                        Some(SOUND_PICKUP_SHOTGUN)
                                    }
                                    _ => None,
                                };
                                if let Some(sound) = sound {
                                    self.sound(p.pos, sound, c as i32);
                                }
                            }
                        }
                    }
                }
                g::POWERUP_NINJA => self.with_char(c, |ch, core, w| ch.give_ninja(core, w)),
                _ => {}
            }
        }
        self.pickups[i] = p;
    }

    /// C++: `CDragger::Tick` (dragger.cpp:14).
    pub(super) fn tick_dragger(&mut self, i: usize) {
        let mut d = self.draggers[i].clone();
        if self.every_015s() {
            if let Some((_, speed)) = self.col.mover_speed(trunc_i32(d.pos.x), trunc_i32(d.pos.y)) {
                d.core = speed;
            }
            d.pos += d.core;
            self.look_for_players_to_drag(&mut d);
        }
        self.dragger_beam_tick(&mut d);
        self.draggers[i] = d;
    }

    fn dragger_reaches(&self, d: &Dragger, to: Vec2) -> bool {
        let hit = if d.ignore_walls {
            self.col.intersect_no_laser_no_walls(d.pos, to).0
        } else {
            self.col.intersect_no_laser(d.pos, to).0
        };
        hit == 0
    }

    fn dragger_switch_off(&self, d: &Dragger, team: i32) -> bool {
        let switchers = &self.core.switchers;
        d.layer == LAYER_SWITCH
            && d.number > 0
            && (d.number as usize) < switchers.len()
            && !switchers[d.number as usize].status[team as usize]
    }

    /// C++: `CDragger::LookForPlayersToDrag` (dragger.cpp:27).
    fn look_for_players_to_drag(&self, d: &mut Dragger) {
        let range = SV_DRAGGER_RANGE as f32 - PHYSICAL_SIZE;
        let (mut closest, mut still_target, mut min_dist) = (-1, false, 0);
        for t in self.find_characters(d.pos, range) {
            let team = self.teams.team(t as i32);
            if team == self.teams.team_super() || self.dragger_switch_off(d, team) {
                continue;
            }
            let pos = self.chars[t].as_ref().unwrap().pos;
            if self.dragger_reaches(d, pos) {
                let dist = trunc_i32(distance(pos, d.pos));
                if min_dist == 0 || min_dist > dist {
                    min_dist = dist;
                    closest = t as i32;
                }
                if t as i32 == d.target_id {
                    still_target = true;
                }
            }
        }
        if (d.target_id != -1 && !still_target) || d.target_id == -1 {
            d.target_id = closest;
        }
    }

    /// C++: `CDragger::DraggerBeamTick` (dragger.cpp:94).
    fn dragger_beam_tick(&mut self, d: &mut Dragger) {
        let Some(t) = self.char_id(d.target_id) else {
            d.target_id = -1;
            return;
        };
        let team = self.teams.team(t as i32);
        if self.every_015s() && self.dragger_switch_off(d, team) {
            d.target_id = -1;
            return;
        }
        let pos = self.chars[t].as_ref().unwrap().pos;
        if distance(pos, d.pos) >= SV_DRAGGER_RANGE as f32 || !self.dragger_reaches(d, pos) {
            d.target_id = -1;
        } else if distance(pos, d.pos) > 28.0 {
            // in the center of the dragger a tee does not experience speed-up
            self.add_velocity(t, normalize(d.pos - pos) * d.strength);
        }
    }

    /// C++: `CPlasma::Tick` (plasma.cpp:66).
    pub(super) fn tick_plasma(&mut self, i: usize) {
        let mut p = self.plasmas[i].clone();
        let target = self.char_id(p.for_client_id);
        match target {
            Some(t) if p.life_time != 0 => {
                p.life_time -= 1;
                // Move
                p.pos += p.core;
                p.core *= PLASMA_ACCEL;
                self.plasma_hit_character(&mut p);
                // plasma may explode twice if it hits a player and an obstacle in one step
                let (hit, _, _) = self.col.intersect_no_laser(p.pos, p.pos + p.core);
                if hit != 0 {
                    if p.explosive {
                        let team = self.teams.team(t as i32);
                        self.create_explosion(
                            p.pos,
                            p.for_client_id,
                            g::WEAPON_GRENADE,
                            true,
                            team,
                            p.for_client_id,
                        );
                    }
                    p.marked_for_destroy = true;
                }
            }
            _ => p.marked_for_destroy = true,
        }
        self.plasmas[i] = p;
    }

    /// C++: `CPlasma::HitCharacter` (plasma.cpp:89).
    fn plasma_hit_character(&mut self, p: &mut Plasma) {
        let hit = self.intersect_character(p.pos, p.pos + p.core, 0.0, None, p.for_client_id, None);
        let Some((hit, _)) = hit else {
            return;
        };
        // super players do not stop plasma
        if self.teams.team(hit as i32) == self.teams.team_super() {
            return;
        }
        if p.freeze {
            self.freeze(hit);
        } else {
            self.unfreeze(hit);
        }
        if p.explosive {
            // only the target's team feels it
            let team = self.teams.team(p.for_client_id);
            let id = p.for_client_id;
            self.create_explosion(p.pos, id, g::WEAPON_GRENADE, true, team, id);
        }
        p.marked_for_destroy = true;
    }
}
