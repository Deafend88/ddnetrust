//! Client prediction world: C++ `CGameWorld` (`src/game/client/prediction/gameworld.cpp`)
//! with its characters, `CCharacter` (`prediction/entities/character.cpp`), which add the
//! DDNet tiles (freeze, switches, speedups, tune zones, ...) to the character core.
//!
//! Weapons are not predicted (`m_PredictWeapons` false), so `FireWeapon` does nothing and
//! no projectiles, lasers or pickups exist. Doors come from snapshot lasers.

use super::collision::{
    CANTMOVE_DOWN, Collision, TILE_DEATH, TILE_FREEZE, TILE_JUMP, TILE_LFREEZE, TILE_LUNFREEZE,
    TILE_NPH_ENABLE, TILE_TELE_LASER_DISABLE, clamp_vel,
};
use super::core::{
    COREEVENT_HOOK_ATTACH_GROUND, CharacterCore, HOOK_IDLE, NUM_WEAPONS, Ninja, PHYSICAL_SIZE,
    SERVER_TICK_SPEED, WorldCore, count_input, read_core, read_ddnet, set_hooked_player,
};
use super::door::{Door, reset_door_collision};
use super::teams::{MAX_CLIENTS, NUM_DDRACE_TEAMS, TeamsCore};
use super::tuning::{Param, Tuning};
use super::vmath::{Vec2, distance, dot, normalize, trunc_i32, vec2};
use crate::protocol::generated::{
    self as g, ObjCharacter, ObjCharacterCore, ObjDDNetCharacter, ObjEntityEx, ObjPlayerInput,
};

const TILE_UNFREEZE: i32 = 11;
const TILE_DFREEZE: i32 = 12;
const TILE_DUNFREEZE: i32 = 13;
const TILE_WALLJUMP: i32 = 16;
const TILE_EHOOK_ENABLE: i32 = 17;
const TILE_EHOOK_DISABLE: i32 = 18;
const TILE_HIT_ENABLE: i32 = 19;
const TILE_HIT_DISABLE: i32 = 20;
const TILE_SOLO_ENABLE: i32 = 21;
const TILE_SOLO_DISABLE: i32 = 22;
const TILE_SWITCHTIMEDOPEN: i32 = 22;
const TILE_SWITCHTIMEDCLOSE: i32 = 23;
const TILE_SWITCHOPEN: i32 = 24;
const TILE_SWITCHCLOSE: i32 = 25;
const TILE_SPEED_BOOST_OLD: i32 = 28;
const TILE_SPEED_BOOST: i32 = 29;
const TILE_REFILL_JUMPS: i32 = 32;
const TILE_NPC_DISABLE: i32 = 88;
const TILE_UNLIMITED_JUMPS_DISABLE: i32 = 89;
const TILE_JETPACK_DISABLE: i32 = 90;
const TILE_NPH_DISABLE: i32 = 91;
const TILE_TELE_GUN_ENABLE: i32 = 96;
const TILE_TELE_GUN_DISABLE: i32 = 97;
const TILE_NPC_ENABLE: i32 = 104;
const TILE_UNLIMITED_JUMPS_ENABLE: i32 = 105;
const TILE_JETPACK_ENABLE: i32 = 106;
const TILE_TELE_GRENADE_ENABLE: i32 = 112;
const TILE_TELE_GRENADE_DISABLE: i32 = 113;
const TILE_TELE_LASER_ENABLE: i32 = 128;

/// C++: `TuneZone::NUM`.
pub const NUM_TUNE_ZONES: usize = 256;
/// C++: `TuneZone::OVERRIDE_NONE`.
pub const TUNE_ZONE_OVERRIDE_NONE: i32 = -1;
/// C++: `g_pData->m_Weapons.m_Ninja.m_Duration` (ms).
const NINJA_DURATION: i32 = 15000;

/// C++: `CGameWorld::m_WorldConfig` plus the `g_Config` values the prediction reads.
#[derive(Clone, Copy, Debug, Default)]
pub struct WorldConfig {
    pub infinite_ammo: bool,
    pub predict_tiles: bool,
    pub predict_freeze: bool,
    pub predict_ddrace: bool,
    pub use_tune_zones: bool,
    pub bug_ddrace_input: bool,
    pub no_weak_hook_and_bounce: bool,
    /// C++: `g_Config.m_SvFreezeDelay` (default 3).
    pub freeze_delay: i32,
    /// C++: `g_Config.m_ClAntiPingPlayers` (default 1).
    pub antiping_players: i32,
}

/// The non-core part of C++ `CCharacter`; its core lives in `GameWorld::core`.
#[derive(Clone, Debug, Default)]
pub struct Character {
    pub id: i32,
    pub pos: Vec2,
    pub prev_pos: Vec2,
    pub prev_prev_pos: Vec2,
    pub is_local: bool,
    pub can_move_in_freeze: bool,
    pub freeze_time: i32,
    pub frozen_last_tick: bool,
    pub tele_checkpoint: i32,
    pub tile_index: i32,
    pub tile_f_index: i32,
    pub last_refill_jumps: bool,
    pub last_weapon: i32,
    pub queued_weapon: i32,
    pub reload_timer: i32,
    pub attack_tick: i32,
    pub move_restrictions: i32,
    pub latest_prev_input: ObjPlayerInput,
    pub latest_input: ObjPlayerInput,
    pub prev_input: ObjPlayerInput,
    pub input: ObjPlayerInput,
    pub saved_input: ObjPlayerInput,
    pub num_inputs: i32,
    pub tune_zone: i32,
    pub tune_zone_override: i32,
    pub last_snap_weapon: i32,
    pub last_weapon_switch_tick: i32,
    pub last_tune_zone_tick: i32,
    pub interfering: bool,
    /// C++: `CEntity::m_SnapTicks`, world ticks since the last snapshot had it.
    pub snap_ticks: i32,
    pub strong_weak_id: i32,
    marked_for_destroy: bool,
}

/// What a character reaches through C++ `GameWorld()` while its core is taken out of the
/// world. `me` is the core's slot in `cores` (`usize::MAX` before it is inserted).
struct Ctx<'w> {
    col: &'w Collision,
    tunings: &'w [Tuning],
    config: &'w WorldConfig,
    game_tick: i32,
    teams: &'w mut TeamsCore,
    switchers: &'w mut [super::core::Switcher],
    cores: &'w mut [Option<CharacterCore>],
    me: usize,
}

/// C++: `CNetObj_Character` starts with the `CNetObj_CharacterCore` fields.
pub fn core_obj(c: &ObjCharacter) -> ObjCharacterCore {
    ObjCharacterCore {
        tick: c.tick,
        x: c.x,
        y: c.y,
        vel_x: c.vel_x,
        vel_y: c.vel_y,
        angle: c.angle,
        direction: c.direction,
        jumped: c.jumped,
        hooked_player: c.hooked_player,
        hook_state: c.hook_state,
        hook_tick: c.hook_tick,
        hook_x: c.hook_x,
        hook_y: c.hook_y,
        hook_dx: c.hook_dx,
        hook_dy: c.hook_dy,
    }
}

/// C++: `CTuningParams::GetWeaponFireDelay`.
fn fire_delay(t: &Tuning, weapon: i32) -> f32 {
    let p = [
        Param::HammerFireDelay,
        Param::GunFireDelay,
        Param::ShotgunFireDelay,
        Param::GrenadeFireDelay,
        Param::LaserFireDelay,
        Param::NinjaFireDelay,
    ][weapon as usize];
    t.get(p) / 1000.0
}

/// C++: the angle computation in `HandleSkippableTiles`: `atan` of the slope, mapped to
/// [0, 2 pi). A zero x takes the `x < 0.0000001f` branch like in C++.
fn speeder_angle(v: Vec2) -> f32 {
    let half_pi = 1.0f32.asin();
    let mut angle = if v.x > 0.0000001 {
        -(v.y / v.x).atan()
    } else if v.x < 0.0000001 {
        (v.y / v.x).atan() + 2.0 * half_pi
    } else if v.y > 0.0000001 {
        half_pi
    } else {
        (-1.0f32).asin()
    };
    if angle < 0.0 {
        angle += 4.0 * half_pi;
    }
    angle
}

impl Character {
    fn tuning<'w>(&self, w: &Ctx<'w>) -> &'w Tuning {
        &w.tunings[self.overridden_tune_zone() as usize]
    }

    /// C++: `CCharacter::GetOverriddenTuneZone`.
    pub fn overridden_tune_zone(&self) -> i32 {
        if self.tune_zone_override == TUNE_ZONE_OVERRIDE_NONE {
            self.tune_zone
        } else {
            self.tune_zone_override
        }
    }

    fn team(&self, w: &Ctx) -> i32 {
        w.teams.team(self.id)
    }

    fn set_weapon(&mut self, c: &mut CharacterCore, w: &Ctx, weapon: i32) {
        if weapon == c.active_weapon {
            return;
        }
        self.last_weapon = c.active_weapon;
        self.queued_weapon = -1;
        self.set_active_weapon(c, w, weapon);
    }

    fn set_active_weapon(&mut self, c: &mut CharacterCore, w: &Ctx, weapon: i32) {
        c.active_weapon = if (g::WEAPON_HAMMER..NUM_WEAPONS as i32).contains(&weapon) {
            weapon
        } else {
            -1
        };
        self.last_weapon_switch_tick = w.game_tick;
    }

    fn set_solo(&mut self, c: &mut CharacterCore, w: &mut Ctx, solo: bool) {
        c.solo = solo;
        w.teams.set_solo(self.id, solo);
    }

    fn set_super(&mut self, c: &mut CharacterCore, w: &mut Ctx, super_: bool) {
        c.super_ = super_;
        if super_ {
            let team_super = w.teams.team_super();
            w.teams.set_team(self.id, team_super);
        }
    }

    fn set_tune_zone(&mut self, w: &Ctx, zone: i32) {
        if zone == self.tune_zone {
            return;
        }
        self.tune_zone = zone;
        self.last_tune_zone_tick = w.game_tick;
    }

    fn take_damage(&mut self, c: &mut CharacterCore, force: Vec2) {
        c.vel = clamp_vel(self.move_restrictions, c.vel + force);
    }

    fn handle_jetpack(&mut self, c: &mut CharacterCore, w: &Ctx) {
        if self.num_inputs < 2 || c.active_weapon < 0 {
            return;
        }
        let direction = normalize(vec2(
            self.latest_input.target_x as f32,
            self.latest_input.target_y as f32,
        ));
        let weapon = c.active_weapon;
        let full_auto = [g::WEAPON_GRENADE, g::WEAPON_SHOTGUN, g::WEAPON_LASER].contains(&weapon)
            || (c.jetpack && weapon == g::WEAPON_GUN);
        let ammo = c.weapons[weapon as usize].ammo;
        let will_fire = count_input(self.latest_prev_input.fire, self.latest_input.fire).0 != 0
            || (full_auto && self.latest_input.fire & 1 != 0 && ammo != 0);
        if !will_fire || ammo == 0 || self.freeze_time != 0 {
            return;
        }
        if weapon == g::WEAPON_GUN && c.jetpack {
            let strength = self.tuning(w).get(Param::JetpackStrength);
            self.take_damage(c, direction * -1.0 * (strength / 100.0 / 6.11));
        }
    }

    fn remove_ninja(&mut self, c: &mut CharacterCore, w: &Ctx) {
        c.ninja.current_move_time = 0;
        c.weapons[g::WEAPON_NINJA as usize].got = false;
        c.active_weapon = self.last_weapon;
        self.set_weapon(c, w, c.active_weapon);
    }

    fn handle_ninja(&mut self, c: &mut CharacterCore, w: &Ctx) {
        if c.active_weapon != g::WEAPON_NINJA {
            return;
        }
        if w.game_tick - c.ninja.activation_tick > NINJA_DURATION * SERVER_TICK_SPEED / 1000 {
            self.remove_ninja(c, w);
            return;
        }
        self.set_weapon(c, w, g::WEAPON_NINJA);
        c.ninja.current_move_time -= 1;
        if c.ninja.current_move_time == 0 {
            c.vel = c.ninja.activation_dir * c.ninja.old_vel_amount as f32;
        }
        // C++ moves the ninja dash while `m_CurrentMoveTime > 0`; only FireWeapon starts one.
    }

    fn do_weapon_switch(&mut self, c: &mut CharacterCore, w: &Ctx) {
        if self.reload_timer != 0 || self.queued_weapon == -1 {
            return;
        }
        if c.weapons[g::WEAPON_NINJA as usize].got || !c.weapons[self.queued_weapon as usize].got {
            return;
        }
        self.set_weapon(c, w, self.queued_weapon);
    }

    fn handle_weapon_switch(&mut self, c: &mut CharacterCore, w: &Ctx) {
        if self.num_inputs < 2 {
            return;
        }
        let mut wanted = if self.queued_weapon != -1 {
            self.queued_weapon
        } else {
            c.active_weapon
        };
        if !c.weapons[..NUM_WEAPONS - 1].iter().any(|w| w.got) {
            return;
        }
        let got = |w: i32| c.weapons[w as usize].got;
        let n = NUM_WEAPONS as i32;
        let mut next = count_input(
            self.latest_prev_input.next_weapon,
            self.latest_input.next_weapon,
        )
        .0;
        let mut prev = count_input(
            self.latest_prev_input.prev_weapon,
            self.latest_input.prev_weapon,
        )
        .0;
        // (C++ only steps for fewer than 128 presses, which the 6-bit counters always are)
        while next != 0 {
            wanted = (wanted + 1) % n;
            if got(wanted) {
                next -= 1;
            }
        }
        while prev != 0 {
            wanted = if wanted - 1 < 0 { n - 1 } else { wanted - 1 };
            if got(wanted) {
                prev -= 1;
            }
        }
        if self.latest_input.wanted_weapon != 0 {
            wanted = self.input.wanted_weapon - 1; // sic: m_Input, not m_LatestInput
        }
        if (0..n).contains(&wanted) && wanted != c.active_weapon && got(wanted) {
            self.queued_weapon = wanted;
        }
        self.do_weapon_switch(c, w);
    }

    fn handle_weapons(&mut self, c: &mut CharacterCore, w: &Ctx) {
        self.handle_ninja(c, w);
        self.handle_jetpack(c, w);
        if self.reload_timer != 0 {
            self.reload_timer -= 1;
        }
        // FireWeapon: weapons are not predicted
    }

    fn give_ninja(&mut self, c: &mut CharacterCore, w: &Ctx) {
        c.ninja.activation_tick = w.game_tick;
        let ninja = &mut c.weapons[g::WEAPON_NINJA as usize];
        ninja.got = true;
        if self.freeze_time == 0 {
            ninja.ammo = -1;
        }
        if c.active_weapon != g::WEAPON_NINJA {
            self.last_weapon = c.active_weapon;
        }
        self.set_active_weapon(c, w, g::WEAPON_NINJA);
    }

    fn on_predicted_input(&mut self, config: &WorldConfig, new: &ObjPlayerInput) {
        if !config.bug_ddrace_input && new.player_flags & g::PLAYERFLAG_CHATTING != 0 {
            self.saved_input = self.input;
            return;
        }
        let (prev_x, prev_y) = (self.input.target_x, self.input.target_y);
        self.input = *new;
        if new.player_flags & g::PLAYERFLAG_SPEC_CAM != 0 {
            self.input.target_x = prev_x;
            self.input.target_y = prev_y;
        }
        if self.input.target_x == 0 && self.input.target_y == 0 {
            self.input.target_y = -1;
        }
        self.saved_input = self.input;
    }

    fn on_direct_input(&mut self, c: &mut CharacterCore, w: &Ctx, new: &ObjPlayerInput) {
        if !w.config.bug_ddrace_input && new.player_flags & g::PLAYERFLAG_CHATTING != 0 {
            self.reset_input();
            self.input.hook = 0;
            return;
        }
        self.num_inputs += 1;
        self.latest_prev_input = self.latest_input;
        self.latest_input = *new;
        if new.player_flags & g::PLAYERFLAG_SPEC_CAM != 0 {
            self.latest_input.target_x = self.latest_prev_input.target_x;
            self.latest_input.target_y = self.latest_prev_input.target_y;
        }
        if self.latest_input.target_x == 0 && self.latest_input.target_y == 0 {
            self.latest_input.target_y = -1;
        }
        if self.num_inputs > 1 && self.team(w) != g::TEAM_SPECTATORS {
            self.handle_weapon_switch(c, w);
            // FireWeapon: weapons are not predicted
        }
        self.latest_prev_input = self.latest_input;
    }

    fn reset_input(&mut self) {
        self.input.direction = 0;
        if self.input.fire & 1 != 0 {
            self.input.fire += 1;
        }
        self.input.fire &= 0x3f; // INPUT_STATE_MASK
        self.input.jump = 0;
        self.latest_input = self.input;
        self.latest_prev_input = self.input;
    }

    /// C++: `CCharacter::Tick` after the core tick.
    fn tick(&mut self, c: &mut CharacterCore, w: &mut Ctx) {
        self.handle_weapons(c, w);
        self.ddrace_post_core_tick(c, w);
        if self.interfering
            && (self.freeze_time == 0 || w.config.antiping_players != 3)
            && (self.input.direction != self.prev_input.direction
                || self.input.jump != self.prev_input.jump
                || c.triggered_events & COREEVENT_HOOK_ATTACH_GROUND != 0)
        {
            // disable dynamic interaction based antiping when the player moved or hooked a wall
            self.interfering = false;
        }
        self.prev_input = self.input;
        self.prev_prev_pos = self.prev_pos;
        self.prev_pos = c.pos;
    }

    fn handle_skippable_tiles(&mut self, c: &mut CharacterCore, w: &Ctx, index: i32) {
        if index < 0 || !w.col.is_speedup(index) {
            return;
        }
        let Some((direction, force, mut max_speed, type_)) = w.col.get_speedup(index) else {
            return;
        };
        let mut temp_vel = c.vel;
        if type_ == TILE_SPEED_BOOST_OLD {
            if force == 255 && max_speed != 0 {
                c.vel = direction * (max_speed / 5) as f32;
                return;
            }
            if max_speed > 0 && max_speed < 5 {
                max_speed = 5;
            }
            if max_speed > 0 {
                let speeder = speeder_angle(direction);
                let tee = speeder_angle(temp_vel);
                // C++ `std::pow(float, 2)` squares in double
                let (vx, vy) = (temp_vel.x as f64, temp_vel.y as f64);
                let tee_speed = (vx * vx + vy * vy).sqrt() as f32;
                let speed_left = max_speed as f32 / 5.0 - (speeder - tee).cos() * tee_speed;
                // a tee at rest gets a NaN angle and SpeedLeft converts to INT_MIN, which
                // does not exceed
                let speed_left_int = trunc_i32(speed_left);
                let exceeds = speed_left_int != i32::MIN && speed_left_int.abs() > force;
                if exceeds && speed_left > 0.0000001 {
                    temp_vel += direction * force as f32;
                } else if exceeds {
                    temp_vel += direction * -force as f32;
                } else {
                    temp_vel += direction * speed_left;
                }
            } else {
                temp_vel += direction * force as f32;
            }
            c.vel = clamp_vel(self.move_restrictions, temp_vel);
        } else if type_ == TILE_SPEED_BOOST {
            const MAX_SPEED_SCALE: f32 = 5.0;
            if max_speed == 0 {
                let t = self.tuning(w);
                let max_ramp_speed = t.get(Param::VelrampRange)
                    / (50.0 * t.get(Param::VelrampCurvature).max(1.01).ln());
                max_speed = trunc_i32(
                    max_ramp_speed.max(t.get(Param::VelrampStart) / 50.0) * MAX_SPEED_SCALE,
                );
            }
            // (signed) length of projection
            let current_directional_speed = dot(direction, c.vel);
            let temp_max_speed = max_speed as f32 / MAX_SPEED_SCALE;
            if current_directional_speed + force as f32 > temp_max_speed {
                temp_vel += direction * (temp_max_speed - current_directional_speed);
            } else {
                temp_vel += direction * force as f32;
            }
            c.vel = clamp_vel(self.move_restrictions, temp_vel);
        }
    }

    fn handle_tiles(&mut self, c: &mut CharacterCore, w: &mut Ctx, map_index: i32) {
        let col = w.col;
        let id = self.id;
        let team = self.team(w);
        let team_super = w.teams.team_super();
        self.tile_index = col.get_tile_index(map_index);
        self.tile_f_index = col.get_front_tile_index(map_index);
        {
            let switchers = &*w.switchers;
            let switch_active = |n: u8| {
                !switchers.is_empty()
                    && team != team_super
                    && switchers[n as usize].status[team as usize]
            };
            self.move_restrictions =
                col.get_move_restrictions(Some(&switch_active), self.pos, 18.0, map_index);
        }
        if !w.config.predict_tiles {
            return;
        }
        if map_index < 0 {
            self.last_refill_jumps = false;
            return;
        }

        let tele_checkpoint = col.is_tele_checkpoint(map_index);
        if tele_checkpoint != 0 {
            self.tele_checkpoint = tele_checkpoint;
        }

        let (ti, tf) = (self.tile_index, self.tile_f_index);
        let on = |tile: i32| ti == tile || tf == tile;
        let vulnerable = !c.super_ && !c.invincible;

        // freeze
        if on(TILE_FREEZE) && vulnerable && !c.deep_frozen {
            self.freeze(c, w);
        } else if on(TILE_UNFREEZE) && !c.deep_frozen {
            self.unfreeze(c, w);
        }
        // deep freeze
        if on(TILE_DFREEZE) && vulnerable && !c.deep_frozen {
            c.deep_frozen = true;
        } else if on(TILE_DUNFREEZE) && vulnerable && c.deep_frozen {
            c.deep_frozen = false;
        }
        // live freeze
        if on(TILE_LFREEZE) && vulnerable {
            c.live_frozen = true;
        } else if on(TILE_LUNFREEZE) && vulnerable {
            c.live_frozen = false;
        }
        // endless hook
        if on(TILE_EHOOK_ENABLE) && !c.endless_hook {
            c.endless_hook = true;
        } else if on(TILE_EHOOK_DISABLE) && c.endless_hook {
            c.endless_hook = false;
        }
        // hit others
        let any_hit_enabled = !c.hammer_hit_disabled
            || !c.shotgun_hit_disabled
            || !c.grenade_hit_disabled
            || !c.laser_hit_disabled;
        let any_hit_disabled = c.hammer_hit_disabled
            || c.shotgun_hit_disabled
            || c.grenade_hit_disabled
            || c.laser_hit_disabled;
        if on(TILE_HIT_DISABLE) && any_hit_enabled {
            set_hit_disabled(c, true);
        } else if on(TILE_HIT_ENABLE) && any_hit_disabled {
            set_hit_disabled(c, false);
        }
        // collide with others
        if on(TILE_NPC_DISABLE) && !c.collision_disabled {
            c.collision_disabled = true;
        } else if on(TILE_NPC_ENABLE) && c.collision_disabled {
            c.collision_disabled = false;
        }
        // hook others
        if on(TILE_NPH_DISABLE) && !c.hook_hit_disabled {
            c.hook_hit_disabled = true;
        } else if on(TILE_NPH_ENABLE) && c.hook_hit_disabled {
            c.hook_hit_disabled = false;
        }
        // unlimited air jumps
        if on(TILE_UNLIMITED_JUMPS_ENABLE) && !c.endless_jump {
            c.endless_jump = true;
        } else if on(TILE_UNLIMITED_JUMPS_DISABLE) && c.endless_jump {
            c.endless_jump = false;
        }
        // walljump
        if on(TILE_WALLJUMP) && c.vel.y > 0.0 && c.colliding != 0 && c.left_wall {
            c.left_wall = false;
            c.jumped_total = if c.jumps >= 2 { c.jumps - 2 } else { 0 };
            c.jumped = 1;
        }
        // jetpack gun
        if on(TILE_JETPACK_ENABLE) && !c.jetpack {
            c.jetpack = true;
        } else if on(TILE_JETPACK_DISABLE) && c.jetpack {
            c.jetpack = false;
        }
        // solo part
        let solo = w.teams.is_solo(id);
        if on(TILE_SOLO_ENABLE) && !solo {
            self.set_solo(c, w, true);
        } else if on(TILE_SOLO_DISABLE) && solo {
            self.set_solo(c, w, false);
        }
        // refill jumps
        if on(TILE_REFILL_JUMPS) && !self.last_refill_jumps {
            c.jumped_total = 0;
            c.jumped = 0;
            self.last_refill_jumps = true;
        }
        if !on(TILE_REFILL_JUMPS) {
            self.last_refill_jumps = false;
        }
        // teleport gun
        if on(TILE_TELE_GUN_ENABLE) && !c.has_telegun_gun {
            c.has_telegun_gun = true;
        } else if on(TILE_TELE_GUN_DISABLE) && c.has_telegun_gun {
            c.has_telegun_gun = false;
        }
        if on(TILE_TELE_GRENADE_ENABLE) && !c.has_telegun_grenade {
            c.has_telegun_grenade = true;
        } else if on(TILE_TELE_GRENADE_DISABLE) && c.has_telegun_grenade {
            c.has_telegun_grenade = false;
        }
        if on(TILE_TELE_LASER_ENABLE) && !c.has_telegun_laser {
            c.has_telegun_laser = true;
        } else if on(TILE_TELE_LASER_DISABLE) && c.has_telegun_laser {
            c.has_telegun_laser = false;
        }

        // stopper
        if c.vel.y > 0.0 && self.move_restrictions & CANTMOVE_DOWN != 0 {
            c.jumped = 0;
            c.jumped_total = 0;
        }
        c.vel = clamp_vel(self.move_restrictions, c.vel);

        // switch tiles
        let switch_type = col.get_switch_type(map_index);
        let number = col.get_switch_number(map_index);
        let delay = col.get_switch_delay(map_index);
        let in_team = team != team_super;
        let tick = w.game_tick;
        let mut set_switch = |status: bool, end_tick: i32, type_: i32| {
            let s = &mut w.switchers[number as usize];
            let t = team as usize;
            s.status[t] = status;
            s.end_tick[t] = end_tick;
            s.type_[t] = type_;
            s.last_update_tick[t] = tick;
        };
        let timed_end = tick + 1 + delay * SERVER_TICK_SPEED;
        if switch_type == TILE_SWITCHOPEN && in_team && number > 0 {
            set_switch(true, 0, TILE_SWITCHOPEN);
        } else if switch_type == TILE_SWITCHTIMEDOPEN && in_team && number > 0 {
            set_switch(true, timed_end, TILE_SWITCHTIMEDOPEN);
        } else if switch_type == TILE_SWITCHTIMEDCLOSE && in_team && number > 0 {
            set_switch(false, timed_end, TILE_SWITCHTIMEDCLOSE);
        } else if switch_type == TILE_SWITCHCLOSE && in_team && number > 0 {
            set_switch(false, 0, TILE_SWITCHCLOSE);
        } else if [
            TILE_FREEZE,
            TILE_DFREEZE,
            TILE_DUNFREEZE,
            TILE_LFREEZE,
            TILE_LUNFREEZE,
        ]
        .contains(&switch_type)
            && in_team
            && !c.invincible
        {
            let active = number == 0 || w.switchers[number as usize].status[team as usize];
            if active {
                match switch_type {
                    TILE_FREEZE => self.freeze_for(c, w, delay),
                    TILE_DFREEZE => c.deep_frozen = true,
                    TILE_DUNFREEZE => c.deep_frozen = false,
                    TILE_LFREEZE => c.live_frozen = true,
                    _ => c.live_frozen = false,
                }
            }
        } else if switch_type == TILE_HIT_ENABLE || switch_type == TILE_HIT_DISABLE {
            let disable = switch_type == TILE_HIT_DISABLE;
            let flag = match delay {
                g::WEAPON_HAMMER => Some(&mut c.hammer_hit_disabled),
                g::WEAPON_SHOTGUN => Some(&mut c.shotgun_hit_disabled),
                g::WEAPON_GRENADE => Some(&mut c.grenade_hit_disabled),
                g::WEAPON_LASER => Some(&mut c.laser_hit_disabled),
                _ => None,
            };
            if let Some(flag) = flag.filter(|f| **f != disable) {
                *flag = disable;
            }
        } else if switch_type == TILE_JUMP {
            let new_jumps = if delay == 255 { -1 } else { delay };
            if new_jumps != c.jumps {
                c.jumps = new_jumps;
            }
        }
    }

    fn handle_tune_layer(&mut self, c: &mut CharacterCore, w: &Ctx) {
        let index = w.col.get_map_index(self.pos);
        let zone = if w.config.use_tune_zones {
            w.col.is_tune(index)
        } else {
            0
        };
        self.set_tune_zone(w, zone);
        c.tuning = *self.tuning(w);
    }

    fn ddrace_tick(&mut self, c: &mut CharacterCore, w: &Ctx) {
        self.input = self.saved_input;
        if c.live_frozen && !self.can_move_in_freeze && !c.super_ && !c.invincible {
            self.input.direction = 0;
            self.input.jump = 0;
            // hook and weapons are possible in live freeze
        }
        if self.freeze_time > 0 {
            self.freeze_time -= 1;
            if !self.can_move_in_freeze {
                self.input.direction = 0;
                self.input.jump = 0;
                self.input.hook = 0;
            }
            if self.freeze_time == 1 {
                self.unfreeze(c, w);
            }
        }

        self.handle_tune_layer(c, w);

        // check if the tee is in any type of freeze
        let col = w.col;
        let index = col.get_pure_map_index(self.pos);
        let tiles = [
            col.get_tile_index(index),
            col.get_front_tile_index(index),
            col.get_switch_type(index),
        ];
        c.is_in_freeze = tiles
            .iter()
            .any(|t| [TILE_FREEZE, TILE_DFREEZE, TILE_LFREEZE, TILE_DEATH].contains(t));
        let r = PHYSICAL_SIZE / 3.0;
        let (x, y) = (self.pos.x, self.pos.y);
        let corners = [
            (x + r, y - r),
            (x + r, y + r),
            (x - r, y - r),
            (x - r, y + r),
        ];
        c.is_in_freeze |= corners
            .iter()
            .any(|&(x, y)| col.get_collision_at(x, y) == TILE_DEATH)
            || corners
                .iter()
                .any(|&(x, y)| col.get_front_collision_at(x, y) == TILE_DEATH);
    }

    fn ddrace_post_core_tick(&mut self, c: &mut CharacterCore, w: &mut Ctx) {
        if !w.config.predict_ddrace {
            return;
        }
        if c.endless_hook {
            c.hook_tick = 0;
        }
        self.frozen_last_tick = false;
        if c.deep_frozen && !c.super_ && !c.invincible {
            self.freeze(c, w);
        }

        // following jump rules can be overridden by tiles, like Refill Jumps, Stopper and
        // Wall Jump
        if c.jumps == -1 || c.jumps == 0 || (c.jumps == 1 && c.jumped > 0) {
            // only one ground jump, no jumps at all, or each jump is the last one: dark feet
            c.jumped |= 2;
        } else if c.jumped_total < c.jumps - 1 && c.jumped > 1 {
            // not yet used up all jumps: light feet
            c.jumped = 1;
        }
        if (c.super_ || c.endless_jump) && c.jumped > 1 {
            c.jumped = 1;
        }

        let current = w.col.get_map_index(self.pos);
        self.handle_skippable_tiles(c, w, current);

        // anti-skip tiles
        let indices = w.col.get_map_indices(self.prev_pos, self.pos, 0);
        if indices.is_empty() {
            self.handle_tiles(c, w, current);
        } else {
            for index in indices {
                self.handle_tiles(c, w, index);
            }
        }
    }

    /// C++: `CCharacter::Freeze()`.
    fn freeze(&mut self, c: &mut CharacterCore, w: &Ctx) {
        self.freeze_for(c, w, w.config.freeze_delay);
    }

    /// C++: `CCharacter::Freeze(int Seconds)`.
    fn freeze_for(&mut self, c: &mut CharacterCore, w: &Ctx, seconds: i32) {
        if !w.config.predict_freeze
            || seconds <= 0
            || c.super_
            || c.invincible
            || self.freeze_time > seconds * SERVER_TICK_SPEED
        {
            return;
        }
        if c.freeze_start < w.game_tick - SERVER_TICK_SPEED {
            self.freeze_time = seconds * SERVER_TICK_SPEED;
            c.freeze_start = w.game_tick;
            c.freeze_end = if c.deep_frozen {
                -1
            } else if self.freeze_time == 0 {
                0
            } else {
                w.game_tick + self.freeze_time
            };
        }
    }

    fn unfreeze(&mut self, c: &mut CharacterCore, w: &Ctx) {
        if self.freeze_time <= 0 {
            return;
        }
        if c.active_weapon >= 0 && !c.weapons[c.active_weapon as usize].got {
            c.active_weapon = g::WEAPON_GUN;
        }
        self.freeze_time = 0;
        c.freeze_start = 0;
        c.freeze_end = if c.deep_frozen { -1 } else { 0 };
        if w.config.predict_ddrace {
            self.frozen_last_tick = true;
        }
    }

    fn reset_prediction(&mut self, c: &mut CharacterCore, w: &mut Ctx) {
        self.set_solo(c, w, false);
        self.set_super(c, w, false);
        c.endless_hook = false;
        set_hit_disabled(c, false);
        c.endless_jump = false;
        c.jetpack = false;
        c.jumps = 2;
        c.hook_hit_disabled = false;
        c.collision_disabled = false;
        self.num_inputs = 0;
        self.freeze_time = 0;
        c.freeze_start = 0;
        c.is_in_freeze = false;
        c.deep_frozen = false;
        c.live_frozen = false;
        self.frozen_last_tick = false;
        for weapon in &mut c.weapons {
            weapon.got = false;
            weapon.ammo = -1;
        }
        if c.hooked_player() >= 0 {
            set_hooked_player(c, w.me, w.cores, -1);
            c.hook_state = HOOK_IDLE;
        }
        self.last_weapon_switch_tick = 0;
        self.last_tune_zone_tick = 0;
        self.interfering = false;
    }

    /// C++: `CCharacter::Read` with the DDNet extended object (DDNet servers always send
    /// it). ponytail: the vanilla path without `CNetObj_DDNetCharacter` is not ported; add
    /// it with vanilla server support.
    fn read(
        &mut self,
        c: &mut CharacterCore,
        w: &mut Ctx,
        obj: &ObjCharacter,
        ext: &ObjDDNetCharacter,
        is_local: bool,
    ) {
        read_core(c, w.me, w.cores, &core_obj(obj));
        self.is_local = is_local;

        let has = |flag: i32| ext.flags & flag != 0;
        self.set_solo(c, w, has(g::CHARACTERFLAG_SOLO));
        self.set_super(c, w, has(g::CHARACTERFLAG_SUPER));
        self.tele_checkpoint = ext.tele_checkpoint;
        self.strong_weak_id = ext.strong_weak_id;
        self.tune_zone_override = ext.tune_zone_override;

        let ninja = has(g::CHARACTERFLAG_WEAPON_NINJA);
        if ninja && c.active_weapon != g::WEAPON_NINJA {
            self.give_ninja(c, w);
        } else if !ninja && c.active_weapon == g::WEAPON_NINJA {
            self.remove_ninja(c, w);
        }

        if w.config.predict_freeze && ext.freeze_end != 0 {
            if ext.freeze_end > 0 {
                if self.freeze_time == 0 {
                    self.freeze(c, w);
                }
                self.freeze_time = 1.max(ext.freeze_end - w.game_tick);
            } else if ext.freeze_end == -1 {
                c.deep_frozen = true;
            }
            // wait for the server to say who is frozen instead of the predicted Freeze()
            if w.config.antiping_players == 3 {
                self.interfering = true;
            }
        } else {
            self.unfreeze(c, w);
        }
        read_ddnet(c, ext);
        if !w.config.predict_freeze {
            self.unfreeze(c, w);
        }

        let pos_before = self.pos;
        self.pos = c.pos;
        if distance(pos_before, self.pos) > 2.0 {
            self.prev_pos = self.pos; // misprediction, don't use prevpos
        }
        if distance(self.prev_pos, self.pos) > 10.0 * 32.0 {
            self.prev_pos = self.pos;
        }
        if obj.jumped & 2 != 0 {
            c.jumped_total = c.jumps;
        }
        self.attack_tick = obj.attack_tick;
        self.last_snap_weapon = obj.weapon;

        let zone = if w.config.use_tune_zones {
            w.col.is_tune(w.col.get_map_index(self.pos))
        } else {
            0
        };
        self.set_tune_zone(w, zone);

        // set the current weapon
        if obj.weapon != g::WEAPON_NINJA {
            if let Some(weapon) = usize::try_from(obj.weapon)
                .ok()
                .and_then(|i| c.weapons.get_mut(i))
            {
                weapon.ammo = if w.config.infinite_ammo || obj.weapon == g::WEAPON_HAMMER {
                    -1
                } else {
                    obj.ammo_count
                };
            }
            if obj.weapon != c.active_weapon {
                self.set_active_weapon(c, w, obj.weapon);
            }
        } else if c.active_weapon < 0 {
            self.set_active_weapon(c, w, obj.weapon);
        }

        // reset all input except direction and hook for non-local players
        if !is_local {
            let mut input = ObjPlayerInput {
                direction: c.direction,
                hook: (c.hook_state != HOOK_IDLE) as i32,
                ..Default::default()
            };
            if ext.target_x != 0 && ext.target_y != 0 {
                input.target_x = ext.target_x;
                input.target_y = ext.target_y;
            } else {
                let angle = obj.angle as f32 / 256.0;
                input.target_x = trunc_i32(angle.cos() * 256.0);
                input.target_y = trunc_i32(angle.sin() * 256.0);
            }
            self.input = input;
            self.saved_input = input;
        }

        // the reload timer of autofire weapons follows from the last attack tick
        if is_local
            && c.active_weapon != -1
            && c.active_weapon != g::WEAPON_HAMMER
            && !c.weapons[g::WEAPON_NINJA as usize].got
            && self.last_tune_zone_tick.max(self.last_weapon_switch_tick) + SERVER_TICK_SPEED
                < w.game_tick
        {
            let ticks =
                trunc_i32(fire_delay(self.tuning(w), c.active_weapon) * SERVER_TICK_SPEED as f32);
            self.reload_timer = 0.max(self.attack_tick + ticks - w.game_tick);
        }
    }
}

fn set_hit_disabled(c: &mut CharacterCore, disabled: bool) {
    c.hammer_hit_disabled = disabled;
    c.shotgun_hit_disabled = disabled;
    c.grenade_hit_disabled = disabled;
    c.laser_hit_disabled = disabled;
}

/// C++: `CGameWorld` holding characters and doors.
#[derive(Clone)]
pub struct GameWorld<'a> {
    col: &'a Collision,
    pub core: WorldCore,
    pub teams: TeamsCore,
    /// C++: the tuning list, one per tune zone.
    pub tunings: Vec<Tuning>,
    pub game_tick: i32,
    pub config: WorldConfig,
    chars: Vec<Option<Character>>,
    /// Character ids in entity list order: C++ inserts at the front.
    order: Vec<usize>,
    /// Doors in entity list order. C++ copies none into the predicted world; a clone
    /// keeps them, but only `net_obj_end` reads them.
    doors: Vec<Door>,
    /// C++: `m_LocalClientId`, set by `net_obj_begin`.
    local_id: i32,
}

impl<'a> GameWorld<'a> {
    /// C++: `CGameWorld::Init` plus `InitSwitchers` from `CGameClient::OnConnected`.
    pub fn new(col: &'a Collision, config: WorldConfig) -> Self {
        let mut core = WorldCore::default();
        core.init_switchers(col.highest_switch_number);
        Self {
            col,
            core,
            teams: TeamsCore::default(),
            tunings: vec![Tuning::default(); NUM_TUNE_ZONES],
            game_tick: 0,
            config,
            chars: vec![None; MAX_CLIENTS],
            order: Vec::new(),
            doors: Vec::new(),
            local_id: -1,
        }
    }

    pub fn character(&self, id: usize) -> Option<(&Character, &CharacterCore)> {
        Some((
            self.chars.get(id)?.as_ref()?,
            self.core.characters[id].as_ref()?,
        ))
    }

    fn ctx(&mut self, me: usize) -> Ctx<'_> {
        Ctx {
            col: self.col,
            tunings: &self.tunings,
            config: &self.config,
            game_tick: self.game_tick,
            teams: &mut self.teams,
            switchers: &mut self.core.switchers,
            cores: &mut self.core.characters,
            me,
        }
    }

    /// Runs `f` on character `id` with it and its core taken out of the world.
    fn with_char<R>(
        &mut self,
        id: usize,
        f: impl FnOnce(&mut Character, &mut CharacterCore, &mut Ctx) -> R,
    ) -> R {
        let mut c = self.core.characters[id]
            .take()
            .expect("character without core");
        let mut ch = self.chars[id].take().expect("character missing");
        let r = f(&mut ch, &mut c, &mut self.ctx(id));
        self.chars[id] = Some(ch);
        self.core.characters[id] = Some(c);
        r
    }

    /// C++: `new CCharacter(World, Id, pChar, pExtended)` then `InsertEntity`. Ids
    /// outside 0..MAX_CLIENTS are ignored like C++ does for its id lookup.
    pub fn add_character(&mut self, id: usize, obj: &ObjCharacter, ext: &ObjDDNetCharacter) {
        if id >= MAX_CLIENTS {
            return;
        }
        let pos = vec2(obj.x as f32, obj.y as f32);
        let input = ObjPlayerInput {
            target_y: -1, // never initialize both to zero
            ..Default::default()
        };
        let mut ch = Character {
            id: id as i32,
            pos,
            prev_pos: pos,
            prev_prev_pos: pos,
            last_weapon: g::WEAPON_HAMMER,
            last_snap_weapon: -1,
            queued_weapon: -1,
            tune_zone_override: TUNE_ZONE_OVERRIDE_NONE,
            input,
            latest_prev_input: input,
            latest_input: input,
            prev_input: input,
            saved_input: input,
            ..Default::default()
        };
        let mut c = CharacterCore::new();
        c.reset_fields();
        c.id = id as i32;
        c.active_weapon = -1; // set by the first Read below
        c.ninja = Ninja::default();
        c.left_wall = true;
        let mut w = self.ctx(usize::MAX); // not inserted yet
        ch.reset_prediction(&mut c, &mut w);
        ch.read(&mut c, &mut w, obj, ext, false);
        self.chars[id] = Some(ch);
        self.core.characters[id] = Some(c);
        self.order.insert(0, id);
    }

    /// C++: `CCharacter::Read` for a character in the world.
    pub fn read_character(
        &mut self,
        id: usize,
        obj: &ObjCharacter,
        ext: &ObjDDNetCharacter,
        is_local: bool,
    ) {
        self.with_char(id, |ch, c, w| ch.read(c, w, obj, ext, is_local));
    }

    /// C++: `CGameWorld::NetObjBegin`: every character is dropped by `net_obj_end` unless
    /// the snapshot has it.
    pub fn net_obj_begin(&mut self, teams: TeamsCore, local_id: i32) {
        self.teams = teams;
        self.local_id = local_id;
        for ch in self.chars.iter_mut().flatten() {
            ch.marked_for_destroy = true;
        }
        for door in &mut self.doors {
            door.marked_for_destroy = true;
        }
    }

    /// C++: `CGameWorld::NetObjAdd` for `NETOBJTYPE_LASER`/`NETOBJTYPE_DDNETLASER`, with
    /// `ExtractLaserInfo`. Only doors are predicted. `data` holds at least the object's
    /// ints.
    pub fn net_obj_add_laser(
        &mut self,
        id: i32,
        type_id: i32,
        data: &[i32],
        ex: Option<&ObjEntityEx>,
    ) {
        let l = crate::items::extract_laser(type_id, data, None, ex);
        let (from, to, owner, type_, number, predict) =
            (l.from, l.to, l.owner, l.type_, l.switch_number, l.predict);
        // C++: IsLocalTeam (no local player: as in net_char_add)
        let local_team =
            owner < 0 || self.local_id < 0 || self.teams.can_collide(self.local_id, owner);
        // doors follow the tile config, everything else needs weapon prediction
        if !local_team || !predict || type_ != g::LASERTYPE_DOOR || !self.config.predict_tiles {
            return;
        }
        let net = Door::new(id, from, to, number);
        if let Some(door) = self.doors.iter_mut().find(|d| d.id == id)
            && net.matches(door)
        {
            door.marked_for_destroy = false; // C++: Keep (Read sets the same positions)
            return;
        }
        self.doors.insert(0, net);
    }

    /// C++: `CGameWorld::NetCharAdd` (with the extended object, see `Character::read`).
    pub fn net_char_add(
        &mut self,
        id: usize,
        obj: &ObjCharacter,
        ext: &ObjDDNetCharacter,
        is_local: bool,
    ) {
        // C++: IsLocalTeam
        if id >= MAX_CLIENTS
            || (self.local_id >= 0 && !self.teams.can_collide(self.local_id, id as i32))
        {
            return;
        }
        if self.chars[id].is_some() {
            self.read_character(id, obj, ext, is_local);
            let ch = self.chars[id].as_mut().unwrap();
            ch.snap_ticks = 0; // C++: CEntity::Keep
            ch.marked_for_destroy = false;
        } else {
            self.add_character(id, obj, ext);
        }
    }

    /// C++: `CGameWorld::NetObjEnd`.
    pub fn net_obj_end(&mut self) {
        // keep predicting hooked characters, based on hook position
        for i in 0..MAX_CLIENTS {
            let Some(ch) = &self.chars[i] else { continue };
            if ch.marked_for_destroy {
                continue;
            }
            let hooked = self.core.characters[i].as_ref().unwrap().hooked_player();
            let hook_pos = self.core.characters[i].as_ref().unwrap().hook_pos;
            let Some(h) = usize::try_from(hooked).ok().filter(|&h| h < MAX_CLIENTS) else {
                continue;
            };
            if let (Some(hc), Some(c)) = (&mut self.chars[h], &mut self.core.characters[h])
                && hc.marked_for_destroy
            {
                hc.pos = hook_pos;
                c.pos = hook_pos;
                c.vel = vec2(0.0, 0.0);
                hc.saved_input = ObjPlayerInput {
                    target_y: -1,
                    ..Default::default()
                };
                hc.marked_for_destroy = false;
            }
        }
        // RemoveEntities
        for i in 0..MAX_CLIENTS {
            if self.chars[i].as_ref().is_some_and(|c| c.marked_for_destroy) {
                self.remove_character(i);
            }
        }
        let col = self.col;
        self.doors.retain(|d| {
            if d.marked_for_destroy {
                d.destroy(col);
            }
            !d.marked_for_destroy
        });
        reset_door_collision(&mut self.doors, col);
    }

    /// C++: `CEntity::Destroy` of a character.
    pub fn remove_character(&mut self, id: usize) {
        self.chars[id] = None;
        self.core.characters[id] = None;
        self.order.retain(|&i| i != id);
    }

    /// C++: `RemoveEntity` + `InsertEntity` for each id that has a character, so the last
    /// one ticks first (strong hook order).
    pub fn reinsert(&mut self, ids: impl IntoIterator<Item = usize>) {
        for id in ids {
            if self.chars.get(id).is_some_and(|c| c.is_some()) {
                self.order.retain(|&i| i != id);
                self.order.insert(0, id);
            }
        }
    }

    /// C++: `CCharacter::SetInput`.
    pub fn set_input(&mut self, id: usize, input: &ObjPlayerInput) {
        if let Some(ch) = self.chars.get_mut(id).and_then(|c| c.as_mut()) {
            ch.latest_input = *input;
            ch.input = *input;
            if ch.input.target_x == 0 && ch.input.target_y == 0 {
                ch.input.target_y = -1;
                ch.latest_input.target_y = -1;
            }
        }
    }

    /// C++: `CCharacter::OnDirectInput`.
    pub fn on_direct_input(&mut self, id: usize, input: &ObjPlayerInput) {
        self.with_char(id, |ch, c, w| ch.on_direct_input(c, w, input));
    }

    /// C++: `CCharacter::OnPredictedInput`.
    pub fn on_predicted_input(&mut self, id: usize, input: &ObjPlayerInput) {
        let config = self.config;
        if let Some(ch) = self.chars.get_mut(id).and_then(|c| c.as_mut()) {
            ch.on_predicted_input(&config, input);
        }
    }

    /// C++: `CCharacter::AntiPingInterference(ClientId, DisallowReset, true)` for the
    /// callbacks the core ticks queued.
    fn apply_interferences(&mut self) {
        for (slot, other, disallow_reset) in std::mem::take(&mut self.core.interferences) {
            let Some(me) = &self.chars[slot] else {
                continue;
            };
            if me.freeze_time != 0 {
                continue;
            }
            let allow_enable = me.is_local || me.interfering;
            let Some(other) = self.chars.get_mut(other as usize).and_then(|c| c.as_mut()) else {
                continue;
            };
            if !allow_enable && !disallow_reset {
                // a non-predicted player interacts with them, but not player bounces here
                if other.freeze_time == 0 || self.config.antiping_players != 3 {
                    other.interfering = false;
                }
            } else if allow_enable {
                other.interfering = true;
            }
        }
    }

    /// C++: `CCharacter::PreTick`.
    fn pre_tick(&mut self, id: usize) {
        self.with_char(id, |ch, c, w| {
            ch.ddrace_tick(c, w);
            c.input = ch.input;
        });
        let deferred = !self.config.no_weak_hook_and_bounce;
        self.core.tick(id, self.col, &self.teams, true, deferred);
        self.apply_interferences();
    }

    /// C++: `CGameWorld::Tick` for the characters.
    pub fn tick(&mut self) {
        let order = self.order.clone();
        let no_weak = self.config.no_weak_hook_and_bounce;
        if no_weak {
            for &id in &order {
                self.pre_tick(id);
            }
        }
        for &id in &order {
            if no_weak {
                self.core.tick_deferred(id, &self.teams);
                self.apply_interferences();
            } else {
                self.pre_tick(id);
            }
            self.with_char(id, |ch, c, w| ch.tick(c, w));
        }
        for &id in &order {
            self.core.move_(id, self.col, &self.teams);
            self.core.quantize(id);
            let pos = self.core.characters[id].as_ref().map(|c| c.pos);
            if let (Some(ch), Some(pos)) = (self.chars[id].as_mut(), pos) {
                ch.pos = pos;
                ch.snap_ticks += 1;
            }
        }

        // update switch state
        let tick = self.game_tick;
        for s in &mut self.core.switchers {
            for j in 0..NUM_DDRACE_TEAMS {
                if s.end_tick[j] <= tick && s.type_[j] == TILE_SWITCHTIMEDOPEN {
                    s.status[j] = false;
                    s.end_tick[j] = 0;
                    s.type_[j] = TILE_SWITCHCLOSE;
                } else if s.end_tick[j] <= tick && s.type_[j] == TILE_SWITCHTIMEDCLOSE {
                    s.status[j] = true;
                    s.end_tick[j] = 0;
                    s.type_[j] = TILE_SWITCHOPEN;
                }
            }
        }
    }
}
