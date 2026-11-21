//! Character movement core. C++: `CCharacterCore` and `CWorldCore`
//! (`src/game/gamecore.{h,cpp}`).
//!
//! The cores live in `WorldCore::characters` (C++: `m_apCharacters`). A core is taken out
//! of the world while it ticks, so it can read and change the others; C++'s "skip
//! myself" checks then fall out naturally. Only the client case with a world is ported
//! (C++ also allows `m_pWorld == nullptr`).

use std::collections::BTreeSet;

use super::collision::{Collision, clamp_vel};
use super::teams::{MAX_CLIENTS, NUM_DDRACE_TEAMS, TeamsCore};
use super::tuning::{Param, Tuning};
use super::vmath::{
    Vec2, closest_point_on_line, distance, dot, length, mix, normalize, round_to_int, trunc_i32,
    vec2,
};
use crate::protocol::generated::{self as g, ObjCharacterCore, ObjDDNetCharacter, ObjPlayerInput};

pub const SERVER_TICK_SPEED: i32 = 50;
pub const PHYSICAL_SIZE: f32 = 28.0;
pub const NUM_WEAPONS: usize = 6;
/// C++: `INPUT_STATE_MASK` (datasrc/network.py).
const INPUT_STATE_MASK: i32 = 0x3f;

pub const HOOK_RETRACTED: i32 = -1;
pub const HOOK_IDLE: i32 = 0;
pub const HOOK_RETRACT_START: i32 = 1;
pub const HOOK_RETRACT_END: i32 = 3;
pub const HOOK_FLYING: i32 = 4;
pub const HOOK_GRABBED: i32 = 5;

pub const COREEVENT_GROUND_JUMP: i32 = 0x01;
pub const COREEVENT_AIR_JUMP: i32 = 0x02;
pub const COREEVENT_HOOK_LAUNCH: i32 = 0x04;
pub const COREEVENT_HOOK_ATTACH_PLAYER: i32 = 0x08;
pub const COREEVENT_HOOK_ATTACH_GROUND: i32 = 0x10;
pub const COREEVENT_HOOK_HIT_NOHOOK: i32 = 0x20;
pub const COREEVENT_HOOK_RETRACT: i32 = 0x40;

const TILE_NOHOOK: i32 = 3;
const TILE_TELEINHOOK: i32 = 15;

/// C++: `SaturatedAdd` (gamecore.h:87).
pub fn saturated_add(min: f32, max: f32, current: f32, modifier: f32) -> f32 {
    // plain comparisons, not f32::max/min, which treat NaN differently
    if modifier < 0.0 {
        if current < min {
            return current;
        }
        let v = current + modifier;
        if v < min { min } else { v }
    } else {
        if current > max {
            return current;
        }
        let v = current + modifier;
        if v > max { max } else { v }
    }
}

/// C++: `VelocityRamp` (gamecore.cpp:123).
pub fn velocity_ramp(value: f32, start: f32, range: f32, curvature: f32) -> f32 {
    if value < start {
        return 1.0;
    }
    1.0 / curvature.powf((value - start) / range)
}

/// C++: `CCharacterCore::CWeaponStat`.
#[derive(Clone, Copy, Debug, Default)]
pub struct WeaponStat {
    pub ammo_regen_start: i32,
    pub ammo: i32,
    pub ammocost: i32,
    pub got: bool,
}

/// C++: `CCharacterCore::m_Ninja`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ninja {
    pub activation_dir: Vec2,
    pub activation_tick: i32,
    pub current_move_time: i32,
    pub old_vel_amount: i32,
}

/// C++: `CCharacterCore`. `Default` is the value-initialized (zeroed) C++ object.
#[derive(Clone, Debug, Default)]
pub struct CharacterCore {
    pub pos: Vec2,
    pub vel: Vec2,
    pub hook_pos: Vec2,
    pub hook_dir: Vec2,
    pub hook_tele_base: Vec2,
    pub hook_tick: i32,
    pub hook_state: i32,
    pub attached_players: BTreeSet<i32>,
    hooked_player: i32,
    pub active_weapon: i32,
    pub weapons: [WeaponStat; NUM_WEAPONS],
    pub ninja: Ninja,
    pub new_hook: bool,
    pub jumped: i32,
    /// jumps performed in the air
    pub jumped_total: i32,
    pub jumps: i32,
    pub direction: i32,
    pub angle: i32,
    pub input: ObjPlayerInput,
    pub triggered_events: i32,
    pub id: i32,
    pub reset: bool,
    pub colliding: i32,
    pub left_wall: bool,
    // DDNet character
    pub solo: bool,
    pub jetpack: bool,
    pub collision_disabled: bool,
    pub endless_hook: bool,
    pub endless_jump: bool,
    pub hammer_hit_disabled: bool,
    pub grenade_hit_disabled: bool,
    pub laser_hit_disabled: bool,
    pub shotgun_hit_disabled: bool,
    pub hook_hit_disabled: bool,
    pub super_: bool,
    pub invincible: bool,
    pub has_telegun_gun: bool,
    pub has_telegun_grenade: bool,
    pub has_telegun_laser: bool,
    pub freeze_start: i32,
    pub freeze_end: i32,
    pub is_in_freeze: bool,
    pub deep_frozen: bool,
    pub live_frozen: bool,
    pub tuning: Tuning,
    move_restrictions: i32,
}

impl CharacterCore {
    pub fn hooked_player(&self) -> i32 {
        self.hooked_player
    }

    /// C++: `CCharacterCore::Reset` without world bookkeeping for the hook (as when
    /// C++ resets a core before it has a world).
    pub fn reset_fields(&mut self) {
        self.pos = vec2(0.0, 0.0);
        self.vel = vec2(0.0, 0.0);
        self.new_hook = false;
        self.hook_pos = vec2(0.0, 0.0);
        self.hook_dir = vec2(0.0, 0.0);
        self.hook_tele_base = vec2(0.0, 0.0);
        self.hook_tick = 0;
        self.hook_state = HOOK_IDLE;
        self.attached_players.clear();
        self.jumped = 0;
        self.jumped_total = 0;
        self.jumps = 2;
        self.triggered_events = 0;
        self.solo = false;
        self.jetpack = false;
        self.collision_disabled = false;
        self.endless_hook = false;
        self.endless_jump = false;
        self.hammer_hit_disabled = false;
        self.grenade_hit_disabled = false;
        self.laser_hit_disabled = false;
        self.shotgun_hit_disabled = false;
        self.hook_hit_disabled = false;
        self.super_ = false;
        self.invincible = false;
        self.has_telegun_gun = false;
        self.has_telegun_grenade = false;
        self.has_telegun_laser = false;
        self.freeze_start = 0;
        self.freeze_end = 0;
        self.is_in_freeze = false;
        self.deep_frozen = false;
        self.live_frozen = false;
        // never initialize both to 0
        self.input.target_x = 0;
        self.input.target_y = -1;
        self.hooked_player = -1;
    }

    /// C++: `CCharacterCore::Init` sets `m_Id = -1`; the world and teams are passed to
    /// each call instead of being stored.
    pub fn new() -> Self {
        Self {
            id: -1,
            ..Self::default()
        }
    }

    /// C++: `CCharacterCore::Write`.
    pub fn write(&self) -> ObjCharacterCore {
        ObjCharacterCore {
            tick: 0,
            x: round_to_int(self.pos.x),
            y: round_to_int(self.pos.y),
            vel_x: round_to_int(self.vel.x * 256.0),
            vel_y: round_to_int(self.vel.y * 256.0),
            hook_state: self.hook_state,
            hook_tick: self.hook_tick,
            hook_x: round_to_int(self.hook_pos.x),
            hook_y: round_to_int(self.hook_pos.y),
            hook_dx: round_to_int(self.hook_dir.x * 256.0),
            hook_dy: round_to_int(self.hook_dir.y * 256.0),
            hooked_player: self.hooked_player,
            jumped: self.jumped,
            direction: self.direction,
            angle: self.angle,
        }
    }
}

/// C++: `SSwitchers`.
#[derive(Clone, Debug)]
pub struct Switcher {
    pub status: [bool; NUM_DDRACE_TEAMS],
    pub initial: bool,
    pub end_tick: [i32; NUM_DDRACE_TEAMS],
    pub type_: [i32; NUM_DDRACE_TEAMS],
    pub last_update_tick: [i32; NUM_DDRACE_TEAMS],
}

/// C++: `CWorldCore`. The client has no PRNG (`m_pPrng == nullptr`), so random tele-outs
/// always take the first one (`RandomOr0` is 0).
#[derive(Clone, Debug)]
pub struct WorldCore {
    pub characters: Vec<Option<CharacterCore>>,
    pub switchers: Vec<Switcher>,
    /// Calls of the C++ `m_AntiPingInterfereCallback` during the last ticks, as
    /// (calling core id slot, other client id, disallow reset); C++'s default does nothing.
    pub interferences: Vec<(usize, i32, bool)>,
}

impl Default for WorldCore {
    fn default() -> Self {
        Self {
            characters: vec![None; MAX_CLIENTS],
            switchers: Vec::new(),
            interferences: Vec::new(),
        }
    }
}

/// C++: `CCharacterCore::SetHookedPlayer` (gamecore.cpp:717) for the core taken out of
/// slot `me`.
pub(crate) fn set_hooked_player(
    core: &mut CharacterCore,
    me: usize,
    chars: &mut [Option<CharacterCore>],
    hooked: i32,
) {
    if hooked == core.hooked_player {
        return;
    }
    let id = core.id;
    let mut attach = |slot: i32, insert: bool, core: &mut CharacterCore| {
        if slot == -1 || id == -1 {
            return;
        }
        let set = if slot as usize == me {
            Some(&mut core.attached_players)
        } else {
            chars
                .get_mut(slot as usize)
                .and_then(|c| c.as_mut())
                .map(|c| &mut c.attached_players)
        };
        if let Some(set) = set {
            if insert {
                set.insert(id);
            } else {
                set.remove(&id);
            }
        }
    };
    attach(core.hooked_player, false, core);
    attach(hooked, true, core);
    core.hooked_player = hooked;
}

impl WorldCore {
    /// C++: `CWorldCore::InitSwitchers`.
    pub fn init_switchers(&mut self, highest_switch_number: i32) {
        let count = if highest_switch_number > 0 {
            highest_switch_number as usize + 1
        } else {
            0
        };
        self.switchers = vec![
            Switcher {
                status: [true; NUM_DDRACE_TEAMS],
                initial: true,
                end_tick: [0; NUM_DDRACE_TEAMS],
                type_: [0; NUM_DDRACE_TEAMS],
                last_update_tick: [0; NUM_DDRACE_TEAMS],
            };
            count
        ];
    }

    fn take(&mut self, slot: usize) -> CharacterCore {
        self.characters[slot]
            .take()
            .expect("ticking an empty character slot")
    }

    /// C++: `CCharacterCore::Reset`.
    pub fn reset(&mut self, slot: usize) {
        let mut c = self.take(slot);
        set_hooked_player(&mut c, slot, &mut self.characters, -1);
        c.reset_fields();
        self.characters[slot] = Some(c);
    }

    /// C++: `CCharacterCore::IsSwitchActiveCb` (gamecore.cpp:748).
    fn is_switch_active(&self, core: &CharacterCore, teams: &TeamsCore, number: u8) -> bool {
        if !self.switchers.is_empty() && core.id != -1 && teams.team(core.id) != teams.team_super()
        {
            return self.switchers[number as usize].status[teams.team(core.id) as usize];
        }
        false
    }

    /// C++: `CCharacterCore::Tick` (gamecore.cpp:195).
    pub fn tick(
        &mut self,
        slot: usize,
        col: &Collision,
        teams: &TeamsCore,
        use_input: bool,
        do_deferred_tick: bool,
    ) {
        let mut c = self.take(slot);
        c.move_restrictions = if use_input {
            col.get_move_restrictions(
                Some(&|n| self.is_switch_active(&c, teams, n)),
                c.pos,
                18.0,
                -1,
            )
        } else {
            col.get_move_restrictions(None, c.pos, 18.0, -1)
        };
        c.triggered_events = 0;

        // get ground state
        let grounded = col.is_on_ground(c.pos, PHYSICAL_SIZE);
        let target_direction = normalize(vec2(c.input.target_x as f32, c.input.target_y as f32));

        let t = c.tuning;
        c.vel.y += t.get(Param::Gravity);

        let max_speed = if grounded {
            t.get(Param::GroundControlSpeed)
        } else {
            t.get(Param::AirControlSpeed)
        };
        let accel = if grounded {
            t.get(Param::GroundControlAccel)
        } else {
            t.get(Param::AirControlAccel)
        };
        let friction = if grounded {
            t.get(Param::GroundFriction)
        } else {
            t.get(Param::AirFriction)
        };

        if use_input {
            c.direction = c.input.direction;

            // C++ calls std::atan2 with ints, i.e. the double overload
            let tmp_angle = (c.input.target_y as f64).atan2(c.input.target_x as f64) as f32;
            c.angle = if tmp_angle < -(super::vmath::PI / 2.0) {
                trunc_i32((tmp_angle + 2.0 * super::vmath::PI) * 256.0)
            } else {
                trunc_i32(tmp_angle * 256.0)
            };

            // jumped bit 1: a jump was made on this input; bit 2: all air jumps used
            if c.input.jump != 0 {
                if c.jumped & 1 == 0 {
                    if grounded && (c.jumped & 2 == 0 || c.jumps != 0) {
                        c.triggered_events |= COREEVENT_GROUND_JUMP;
                        c.vel.y = -t.get(Param::GroundJumpImpulse);
                        if c.jumps > 1 {
                            c.jumped |= 1;
                        } else {
                            c.jumped |= 3;
                        }
                        c.jumped_total = 0;
                    } else if c.jumped & 2 == 0 {
                        c.triggered_events |= COREEVENT_AIR_JUMP;
                        c.vel.y = -t.get(Param::AirJumpImpulse);
                        c.jumped |= 3;
                        c.jumped_total += 1;
                    }
                }
            } else {
                c.jumped &= !1;
            }

            if c.input.hook != 0 {
                if c.hook_state == HOOK_IDLE {
                    c.hook_state = HOOK_FLYING;
                    c.hook_pos = c.pos + target_direction * PHYSICAL_SIZE * 1.5;
                    c.hook_dir = target_direction;
                    set_hooked_player(&mut c, slot, &mut self.characters, -1);
                    c.hook_tick =
                        trunc_i32(SERVER_TICK_SPEED as f32 * (1.25 - t.get(Param::HookDuration)));
                    c.triggered_events |= COREEVENT_HOOK_LAUNCH;
                }
            } else {
                set_hooked_player(&mut c, slot, &mut self.characters, -1);
                c.hook_state = HOOK_IDLE;
                c.hook_pos = c.pos;
            }
        }

        if grounded {
            c.jumped &= !2;
            c.jumped_total = 0;
        }

        // add the speed modification according to players wanted direction
        if c.direction < 0 {
            c.vel.x = saturated_add(-max_speed, max_speed, c.vel.x, -accel);
        }
        if c.direction > 0 {
            c.vel.x = saturated_add(-max_speed, max_speed, c.vel.x, accel);
        }
        if c.direction == 0 {
            c.vel.x *= friction;
        }

        // do hook
        if c.hook_state == HOOK_IDLE {
            set_hooked_player(&mut c, slot, &mut self.characters, -1);
            c.hook_pos = c.pos;
        } else if c.hook_state >= HOOK_RETRACT_START && c.hook_state < HOOK_RETRACT_END {
            c.hook_state += 1;
        } else if c.hook_state == HOOK_RETRACT_END {
            c.triggered_events |= COREEVENT_HOOK_RETRACT;
            c.hook_state = HOOK_RETRACTED;
        } else if c.hook_state == HOOK_FLYING {
            let hook_base = if c.new_hook { c.hook_tele_base } else { c.pos };
            let mut new_pos = c.hook_pos + c.hook_dir * t.get(Param::HookFireSpeed);
            if distance(hook_base, new_pos) > t.get(Param::HookLength) {
                c.hook_state = HOOK_RETRACT_START;
                new_pos = hook_base + normalize(new_pos - hook_base) * t.get(Param::HookLength);
                c.reset = true;
            }

            // make sure that the hook doesn't go though the ground
            let (hit, coll, _, tele_nr) = col.intersect_line_tele_hook(c.hook_pos, new_pos);
            new_pos = coll;
            let (mut going_to_hit_ground, mut going_to_retract, mut going_through_tele) =
                (false, false, false);
            if hit != 0 {
                if hit == TILE_NOHOOK {
                    going_to_retract = true;
                } else if hit == TILE_TELEINHOOK {
                    going_through_tele = true;
                } else {
                    going_to_hit_ground = true;
                }
                c.reset = true;
            }

            // check against other players first
            if !c.hook_hit_disabled
                && t.get(Param::PlayerHooking) != 0.0
                && (c.hook_state == HOOK_FLYING || !c.new_hook)
            {
                let mut dist = 0.0;
                for i in 0..MAX_CLIENTS {
                    let Some(other) = &self.characters[i] else {
                        continue;
                    };
                    if !(c.super_ || other.super_)
                        && ((c.id != -1 && !teams.can_collide(i as i32, c.id))
                            || other.solo
                            || c.solo)
                    {
                        continue;
                    }
                    let other_pos = other.pos;
                    if let Some(closest) = closest_point_on_line(c.hook_pos, new_pos, other_pos)
                        && distance(other_pos, closest) < PHYSICAL_SIZE + 2.0
                        && (c.hooked_player == -1 || distance(c.hook_pos, other_pos) < dist)
                    {
                        c.triggered_events |= COREEVENT_HOOK_ATTACH_PLAYER;
                        c.hook_state = HOOK_GRABBED;
                        set_hooked_player(&mut c, slot, &mut self.characters, i as i32);
                        dist = distance(c.hook_pos, other_pos);
                        self.interferences.push((slot, i as i32, false));
                    }
                }
            }

            if c.hook_state == HOOK_FLYING {
                // check against ground
                if going_to_hit_ground {
                    c.triggered_events |= COREEVENT_HOOK_ATTACH_GROUND;
                    c.hook_state = HOOK_GRABBED;
                } else if going_to_retract {
                    c.triggered_events |= COREEVENT_HOOK_HIT_NOHOOK;
                    c.hook_state = HOOK_RETRACT_START;
                }

                let outs = if going_through_tele {
                    col.tele_outs(tele_nr - 1)
                } else {
                    &[]
                };
                if !outs.is_empty() {
                    c.triggered_events = 0;
                    set_hooked_player(&mut c, slot, &mut self.characters, -1);
                    c.new_hook = true;
                    // C++: RandomOr0(size) without a PRNG
                    c.hook_pos = outs[0] + target_direction * PHYSICAL_SIZE * 1.5;
                    c.hook_dir = target_direction;
                    c.hook_tele_base = c.hook_pos;
                } else {
                    c.hook_pos = new_pos;
                }
            }
        }

        if c.hook_state == HOOK_GRABBED {
            if c.hooked_player != -1 {
                let hooked = self
                    .characters
                    .get(c.hooked_player as usize)
                    .and_then(|o| o.as_ref());
                match hooked {
                    Some(other) if c.id != -1 && teams.can_keep_hook(c.id, other.id) => {
                        c.hook_pos = other.pos
                    }
                    _ => {
                        // release hook
                        set_hooked_player(&mut c, slot, &mut self.characters, -1);
                        c.hook_state = HOOK_RETRACTED;
                        c.hook_pos = c.pos;
                    }
                }
            }

            // don't do this hook routine when we are already hooked to a player
            if c.hooked_player == -1 && distance(c.hook_pos, c.pos) > 46.0 {
                let mut hook_vel = normalize(c.hook_pos - c.pos) * t.get(Param::HookDragAccel);
                // the hook has more power to drag you up than down
                if hook_vel.y > 0.0 {
                    hook_vel.y *= 0.3;
                }
                // boost if the player wants to move in that direction, dampen otherwise
                if (hook_vel.x < 0.0 && c.direction < 0) || (hook_vel.x > 0.0 && c.direction > 0) {
                    hook_vel.x *= 0.95;
                } else {
                    hook_vel.x *= 0.75;
                }
                let new_vel = c.vel + hook_vel;
                // check if we are under the legal limit for the hook
                let new_vel_length = length(new_vel);
                if new_vel_length < t.get(Param::HookDragSpeed) || new_vel_length < length(c.vel) {
                    c.vel = new_vel;
                }
            }

            // release hook (max default hook time is 1.25 s)
            c.hook_tick += 1;
            if c.hooked_player != -1
                && (c.hook_tick > SERVER_TICK_SPEED + SERVER_TICK_SPEED / 5
                    || self
                        .characters
                        .get(c.hooked_player as usize)
                        .is_none_or(|o| o.is_none()))
            {
                set_hooked_player(&mut c, slot, &mut self.characters, -1);
                c.hook_state = HOOK_RETRACTED;
                c.hook_pos = c.pos;
            }
        }

        self.characters[slot] = Some(c);
        if do_deferred_tick {
            self.tick_deferred(slot, teams);
        }
    }

    /// C++: `CCharacterCore::TickDeferred` (gamecore.cpp:465): player collision and
    /// hook drag on the hooked player.
    pub fn tick_deferred(&mut self, slot: usize, teams: &TeamsCore) {
        let mut c = self.take(slot);
        let t = c.tuning;
        for i in 0..MAX_CLIENTS {
            let Some(other) = &mut self.characters[i] else {
                continue;
            };
            if c.id != -1 && !teams.can_collide(c.id, i as i32) {
                continue; // make sure that we don't nudge our self
            }
            if !(c.super_ || other.super_) && (c.solo || other.solo) {
                continue;
            }

            // handle player <-> player collision
            let dist = distance(c.pos, other.pos);
            if dist > 0.0 {
                let dir = normalize(c.pos - other.pos);
                let can_collide = (c.super_ || other.super_)
                    || (!c.collision_disabled
                        && !other.collision_disabled
                        && t.get(Param::PlayerCollision) != 0.0);
                if can_collide && dist < PHYSICAL_SIZE * 1.25 {
                    let a = PHYSICAL_SIZE * 1.45 - dist;
                    let mut velocity = 0.5;
                    // don't add excess force when moving away already
                    if length(c.vel) > 0.0001 {
                        velocity = 1.0 - (dot(normalize(c.vel), dir) + 1.0) / 2.0;
                    }
                    c.vel += dir * a * (velocity * 0.75);
                    c.vel *= 0.85;
                    self.interferences.push((slot, i as i32, true));
                }

                // handle hook influence
                if !c.hook_hit_disabled
                    && c.hooked_player == i as i32
                    && t.get(Param::PlayerHooking) != 0.0
                    && dist > PHYSICAL_SIZE * 1.50
                {
                    let hook_accel =
                        t.get(Param::HookDragAccel) * (dist / t.get(Param::HookLength));
                    let drag_speed = t.get(Param::HookDragSpeed);
                    // add force to the hooked player
                    let temp = vec2(
                        saturated_add(
                            -drag_speed,
                            drag_speed,
                            other.vel.x,
                            hook_accel * dir.x * 1.5,
                        ),
                        saturated_add(
                            -drag_speed,
                            drag_speed,
                            other.vel.y,
                            hook_accel * dir.y * 1.5,
                        ),
                    );
                    other.vel = clamp_vel(other.move_restrictions, temp);
                    // add a little bit force to the guy who has the grip
                    let temp = vec2(
                        saturated_add(-drag_speed, drag_speed, c.vel.x, -hook_accel * dir.x * 0.25),
                        saturated_add(-drag_speed, drag_speed, c.vel.y, -hook_accel * dir.y * 0.25),
                    );
                    c.vel = clamp_vel(c.move_restrictions, temp);
                }
            }
        }
        if c.hook_state != HOOK_FLYING {
            c.new_hook = false;
        }

        // clamp the velocity to something sane
        if length(c.vel) > 6000.0 {
            c.vel = normalize(c.vel) * 6000.0;
        }
        self.characters[slot] = Some(c);
    }

    /// C++: `CCharacterCore::Move` (gamecore.cpp:538).
    pub fn move_(&mut self, slot: usize, col: &Collision, teams: &TeamsCore) {
        let mut c = self.take(slot);
        let t = c.tuning;
        let ramp_value = velocity_ramp(
            length(c.vel) * 50.0,
            t.get(Param::VelrampStart),
            t.get(Param::VelrampRange),
            t.get(Param::VelrampCurvature),
        );
        c.vel.x *= ramp_value;

        let mut new_pos = c.pos;
        let old_vel = c.vel;
        let elasticity = vec2(
            t.get(Param::GroundElasticityX),
            t.get(Param::GroundElasticityY),
        );
        let grounded = col.move_box(
            &mut new_pos,
            &mut c.vel,
            vec2(PHYSICAL_SIZE, PHYSICAL_SIZE),
            elasticity,
        );
        if grounded {
            c.jumped &= !2;
            c.jumped_total = 0;
        }

        c.colliding = 0;
        if c.vel.x < 0.001 && c.vel.x > -0.001 {
            if old_vel.x > 0.0 {
                c.colliding = 1;
            } else if old_vel.x < 0.0 {
                c.colliding = 2;
            }
        } else {
            c.left_wall = true;
        }

        c.vel.x *= 1.0 / ramp_value;

        if c.super_ || (t.get(Param::PlayerCollision) != 0.0 && !c.collision_disabled && !c.solo) {
            // check player collision
            let dist = distance(c.pos, new_pos);
            if dist > 0.0 {
                let colliders: Vec<Vec2> = (0..MAX_CLIENTS)
                    .filter_map(|id| {
                        let other = self.characters[id].as_ref()?;
                        let skip = !(other.super_ || c.super_)
                            && (c.solo
                                || other.solo
                                || other.collision_disabled
                                || (c.id != -1 && !teams.can_collide(c.id, id as i32)));
                        (!skip).then_some(other.pos)
                    })
                    .collect();
                let end = trunc_i32(dist + 1.0);
                let mut last_pos = c.pos;
                for i in 0..end.max(0) {
                    let a = i as f32 / dist;
                    let pos = mix(c.pos, new_pos, a);
                    for &other_pos in &colliders {
                        let d = distance(pos, other_pos);
                        if d < PHYSICAL_SIZE {
                            if a > 0.0 {
                                c.pos = last_pos;
                            } else if distance(new_pos, other_pos) > d {
                                c.pos = new_pos;
                            }
                            self.characters[slot] = Some(c);
                            return;
                        }
                    }
                    last_pos = pos;
                }
            }
        }
        c.pos = new_pos;
        self.characters[slot] = Some(c);
    }

    /// C++: `CCharacterCore::Read`.
    pub fn read(&mut self, slot: usize, core: &ObjCharacterCore) {
        let mut c = self.take(slot);
        read_core(&mut c, slot, &mut self.characters, core);
        self.characters[slot] = Some(c);
    }

    /// C++: `CCharacterCore::Quantize`: round through the network representation.
    pub fn quantize(&mut self, slot: usize) {
        let net = self.characters[slot]
            .as_ref()
            .expect("quantizing an empty slot")
            .write();
        self.read(slot, &net);
    }
}

/// C++: `CCharacterCore::Read` for the core that belongs in slot `me` (taken out of
/// `cores`, or not inserted yet).
pub fn read_core(
    c: &mut CharacterCore,
    me: usize,
    cores: &mut [Option<CharacterCore>],
    core: &ObjCharacterCore,
) {
    c.pos = vec2(core.x as f32, core.y as f32);
    c.vel = vec2(core.vel_x as f32 / 256.0, core.vel_y as f32 / 256.0);
    c.hook_state = core.hook_state;
    c.hook_tick = core.hook_tick;
    c.hook_pos = vec2(core.hook_x as f32, core.hook_y as f32);
    c.hook_dir = vec2(core.hook_dx as f32 / 256.0, core.hook_dy as f32 / 256.0);
    set_hooked_player(c, me, cores, core.hooked_player);
    c.jumped = core.jumped;
    c.direction = core.direction;
    c.angle = core.angle;
}

/// C++: `CCharacterCore::ReadDDNet` (gamecore.cpp:654).
pub fn read_ddnet(c: &mut CharacterCore, ext: &ObjDDNetCharacter) {
    let has = |flag: i32| ext.flags & flag != 0;
    c.solo = has(g::CHARACTERFLAG_SOLO);
    c.jetpack = has(g::CHARACTERFLAG_JETPACK);
    c.collision_disabled = has(g::CHARACTERFLAG_COLLISION_DISABLED);
    c.hammer_hit_disabled = has(g::CHARACTERFLAG_HAMMER_HIT_DISABLED);
    c.shotgun_hit_disabled = has(g::CHARACTERFLAG_SHOTGUN_HIT_DISABLED);
    c.grenade_hit_disabled = has(g::CHARACTERFLAG_GRENADE_HIT_DISABLED);
    c.laser_hit_disabled = has(g::CHARACTERFLAG_LASER_HIT_DISABLED);
    c.hook_hit_disabled = has(g::CHARACTERFLAG_HOOK_HIT_DISABLED);
    c.super_ = has(g::CHARACTERFLAG_SUPER);
    c.invincible = has(g::CHARACTERFLAG_INVINCIBLE);
    c.endless_hook = has(g::CHARACTERFLAG_ENDLESS_HOOK);
    c.endless_jump = has(g::CHARACTERFLAG_ENDLESS_JUMP);
    c.freeze_end = ext.freeze_end;
    c.deep_frozen = ext.freeze_end == -1;
    c.live_frozen = has(g::CHARACTERFLAG_MOVEMENTS_DISABLED);
    c.has_telegun_grenade = has(g::CHARACTERFLAG_TELEGUN_GRENADE);
    c.has_telegun_gun = has(g::CHARACTERFLAG_TELEGUN_GUN);
    c.has_telegun_laser = has(g::CHARACTERFLAG_TELEGUN_LASER);
    let weapons = [
        g::CHARACTERFLAG_WEAPON_HAMMER,
        g::CHARACTERFLAG_WEAPON_GUN,
        g::CHARACTERFLAG_WEAPON_SHOTGUN,
        g::CHARACTERFLAG_WEAPON_GRENADE,
        g::CHARACTERFLAG_WEAPON_LASER,
        g::CHARACTERFLAG_WEAPON_NINJA,
    ];
    for (w, flag) in c.weapons.iter_mut().zip(weapons) {
        w.got = ext.flags & flag != 0;
    }
    c.jumps = ext.jumps;
    // display information is only accepted when it was sent (not -1)
    if ext.jumped_total != -1 {
        c.jumped_total = ext.jumped_total;
    }
    if ext.ninja_activation_tick != -1 {
        c.ninja.activation_tick = ext.ninja_activation_tick;
    }
    if ext.freeze_start != -1 {
        c.freeze_start = ext.freeze_start;
        c.is_in_freeze = has(g::CHARACTERFLAG_IN_FREEZE);
    }
}

/// C++: `CountInput` (gamecore.h:296): presses and releases between two input states.
pub fn count_input(prev: i32, cur: i32) -> (i32, i32) {
    let (mut presses, mut releases) = (0, 0);
    let (mut i, cur) = (prev & INPUT_STATE_MASK, cur & INPUT_STATE_MASK);
    while i != cur {
        i = (i + 1) & INPUT_STATE_MASK;
        if i & 1 != 0 {
            presses += 1;
        } else {
            releases += 1;
        }
    }
    (presses, releases)
}
