//! C++: the weapon parts of the prediction `CCharacter` (HandleWeapons, HandleNinja,
//! FireWeapon; prediction/entities/character.cpp:108-522) and `CGameWorld::FindEntities`,
//! `IntersectCharacter` and `CreateExplosion` (gameworld.cpp). They reach other characters
//! and create entities, so they work on the whole world instead of on one character taken
//! out of it.

use super::collision::clamp_vel;
use super::core::{PHYSICAL_SIZE, SERVER_TICK_SPEED, count_input};
use super::teams::MAX_CLIENTS;
use super::tuning::Param;
use super::vmath::{
    PI, Vec2, angle, closest_point_on_line, direction, distance, length, normalize, trunc_i32, vec2,
};
use super::world::{
    GameWorld, NINJA_DURATION, PredictedEvent, create_predicted_event, create_predicted_sound,
    fire_delay,
};
use crate::protocol::generated as g;
use crate::protocol::sounds::*;

/// C++: `g_Config.m_SvHit` and `g_Config.m_SvDeepfly`: server settings the client never
/// receives, so the client's own defaults (1) apply.
pub const SV_HIT: bool = true;
const SV_DEEPFLY: bool = true;
/// C++: `g_pData->m_Weapons.m_Ninja` (datasrc/content.py).
const NINJA_MOVETIME: i32 = 200;
const NINJA_VELOCITY: i32 = 50;

impl GameWorld {
    /// C++: `GetCharacterById`: `id` if a character with it exists.
    pub fn char_id(&self, id: i32) -> Option<usize> {
        usize::try_from(id)
            .ok()
            .filter(|&i| i < MAX_CLIENTS && self.chars[i].is_some())
    }

    /// C++: `CCharacter::CanCollide(ClientId)` of character `id`.
    pub(super) fn can_collide(&self, id: usize, other: i32) -> bool {
        self.teams.can_collide(id as i32, other)
    }

    /// C++: `CGameWorld::FindEntities` for characters, in list order.
    pub(super) fn find_characters(&self, pos: Vec2, radius: f32) -> Vec<usize> {
        self.order
            .iter()
            .copied()
            .filter(|&i| {
                self.chars[i]
                    .as_ref()
                    .is_some_and(|c| distance(c.pos, pos) < radius + PHYSICAL_SIZE)
            })
            .collect()
    }

    /// C++: `CGameWorld::IntersectCharacter` (gameworld.cpp:261): the character nearest to
    /// `pos0` whose circle (plus `radius`) the line touches, and the point on the line.
    pub(super) fn intersect_character(
        &self,
        pos0: Vec2,
        pos1: Vec2,
        radius: f32,
        not_this: Option<usize>,
        collide_with: i32,
        this_only: Option<usize>,
    ) -> Option<(usize, Vec2)> {
        let mut closest_len = distance(pos0, pos1) * 100.0;
        let mut closest = None;
        for &i in &self.order {
            let Some(ch) = &self.chars[i] else { continue };
            if Some(i) == not_this
                || this_only.is_some_and(|o| o != i)
                || (collide_with != -1 && !self.can_collide(i, collide_with))
            {
                continue;
            }
            if let Some(at) = closest_point_on_line(pos0, pos1, ch.pos)
                && distance(ch.pos, at) < PHYSICAL_SIZE + radius
            {
                let len = distance(pos0, at);
                if len < closest_len {
                    closest_len = len;
                    closest = Some((i, at));
                }
            }
        }
        closest
    }

    /// C++: `CGameWorld::CreatePredictedSound`.
    pub(super) fn sound(&mut self, pos: Vec2, sound: i32, id: i32) {
        create_predicted_sound(
            &self.config,
            &mut self.predicted_events,
            self.game_tick,
            pos,
            sound,
            id,
        );
    }

    /// C++: `CGameWorld::CreatePredictedDamageIndEvent`.
    pub(super) fn damage_ind_event(&mut self, pos: Vec2, angle: f32, amount: i32, id: i32) {
        let a = 3.0 * PI / 2.0 + angle;
        let (s, e) = (a - PI / 3.0, a + PI / 3.0);
        for i in 0..amount {
            let f = s + (e - s) * ((i + 1) as f32 / (amount + 1) as f32);
            let ev = PredictedEvent::new(
                g::NETEVENTTYPE_DAMAGEIND,
                pos,
                id,
                self.game_tick,
                (f * 256.0) as i32,
            );
            create_predicted_event(&self.config, &mut self.predicted_events, ev);
        }
    }

    /// C++: `CGameWorld::CreatePredictedHammerHitEvent`.
    fn hammer_hit_event(&mut self, pos: Vec2, id: i32) {
        let e = PredictedEvent::new(g::NETEVENTTYPE_HAMMERHIT, pos, id, self.game_tick, -1);
        create_predicted_event(&self.config, &mut self.predicted_events, e);
    }

    /// C++: `CCharacter::TakeDamage`: only the force counts.
    pub(super) fn take_damage(&mut self, id: usize, force: Vec2) {
        self.with_char(id, |ch, c, _| ch.take_damage(c, force));
    }

    /// C++: `CCharacter::AddVelocity` (= `SetVelocity(m_Vel + Addition)`).
    pub(super) fn add_velocity(&mut self, id: usize, add: Vec2) {
        let mr = self.chars[id].as_ref().unwrap().move_restrictions;
        let c = self.core.characters[id].as_mut().unwrap();
        c.vel = clamp_vel(mr, c.vel + add);
    }

    /// C++: `CCharacter::ApplyMoveRestrictions`.
    pub(super) fn apply_move_restrictions(&mut self, id: usize) {
        let mr = self.chars[id].as_ref().unwrap().move_restrictions;
        let c = self.core.characters[id].as_mut().unwrap();
        c.vel = clamp_vel(mr, c.vel);
    }

    /// C++: `CCharacter::Freeze()`.
    pub(super) fn freeze(&mut self, id: usize) -> bool {
        self.with_char(id, |ch, c, w| ch.freeze(c, w))
    }

    /// C++: `CCharacter::Unfreeze()`.
    pub(super) fn unfreeze(&mut self, id: usize) {
        self.with_char(id, |ch, c, w| ch.unfreeze(c, w));
    }

    /// C++: `CCharacter::AntiPingInterference` (character.cpp:1345) of character `me`.
    pub(super) fn anti_ping_interference(
        &mut self,
        me: usize,
        other: i32,
        disallow_reset: bool,
        has_to_be_unfrozen: bool,
    ) {
        let Some(me) = &self.chars[me] else { return };
        if has_to_be_unfrozen && me.freeze_time != 0 {
            return;
        }
        let allow_enable = me.is_local || me.interfering;
        let antiping_players = self.config.antiping_players;
        let Some(other) = self.char_id(other) else {
            return;
        };
        let other = self.chars[other].as_mut().unwrap();
        if !allow_enable && !disallow_reset {
            // a non-predicted player interacts with them, but not player bounces here
            if other.freeze_time == 0 || antiping_players != 3 {
                other.interfering = false;
            }
        } else if allow_enable {
            other.interfering = true;
        }
    }

    /// C++: `CGameWorld::CreateExplosion` (gameworld.cpp:345).
    pub(super) fn create_explosion(
        &mut self,
        pos: Vec2,
        owner: i32,
        weapon: i32,
        no_damage: bool,
        activated_team: i32,
        id: i32,
    ) {
        let cfg = self.config;
        if owner < 0 && cfg.is_solo && !(weapon == g::WEAPON_SHOTGUN && cfg.is_ddrace) {
            return;
        }
        if cfg.is_ddrace && cfg.predict_ddrace {
            // vanilla has different projectile physics
            let e = PredictedEvent::new(g::NETEVENTTYPE_EXPLOSION, pos, id, self.game_tick, -1);
            create_predicted_event(&self.config, &mut self.predicted_events, e);
        }
        let (radius, inner_radius) = (135.0f32, 48.0f32);
        for i in self.find_characters(pos, radius) {
            let diff = self.chars[i].as_ref().unwrap().pos - pos;
            let mut force_dir = vec2(0.0, 1.0);
            let mut l = length(diff);
            if l != 0.0 {
                force_dir = normalize(diff);
            }
            l = 1.0 - ((l - inner_radius) / (radius - inner_radius)).clamp(0.0, 1.0);
            let owner_char = self.char_id(owner);
            let strength = match owner_char {
                Some(o) if owner != -1 => {
                    let zone = self.chars[o].as_ref().unwrap().overridden_tune_zone();
                    self.tunings[zone as usize].get(Param::ExplosionStrength)
                }
                _ => self.tunings[0].get(Param::ExplosionStrength),
            };
            let dmg = strength * l;
            if trunc_i32(dmg) == 0 {
                continue;
            }
            let grenade_hit_disabled = |w: &GameWorld, o: usize| {
                w.core.characters[o].as_ref().unwrap().grenade_hit_disabled
            };
            let may_hit = match owner_char {
                Some(o) => !grenade_hit_disabled(self, o),
                None => SV_HIT || no_damage,
            } || owner == i as i32;
            if !may_hit {
                continue;
            }
            if owner != -1 && !self.can_collide(i, owner) {
                continue;
            }
            if owner == -1 && activated_team != -1 && self.teams.team(i as i32) != activated_team {
                continue;
            }
            self.take_damage(i, force_dir * dmg * 2.0);
            if let Some(o) = owner_char {
                self.anti_ping_interference(o, i as i32, false, false);
            }
            let stop = match owner_char {
                Some(o) => grenade_hit_disabled(self, o),
                None => !SV_HIT || no_damage,
            };
            if stop {
                break;
            }
        }
    }

    /// C++: `CCharacter::HandleNinja` (character.cpp:108).
    fn handle_ninja(&mut self, id: usize) {
        let dashing = self.with_char(id, |ch, c, w| {
            if c.active_weapon != g::WEAPON_NINJA {
                return false;
            }
            if w.game_tick - c.ninja.activation_tick > NINJA_DURATION * SERVER_TICK_SPEED / 1000 {
                // time's up, return
                ch.remove_ninja(c, w);
                return false;
            }
            // force ninja weapon
            ch.set_weapon(c, w, g::WEAPON_NINJA);
            c.ninja.current_move_time -= 1;
            if c.ninja.current_move_time == 0 {
                c.vel = c.ninja.activation_dir * c.ninja.old_vel_amount as f32;
            }
            c.ninja.current_move_time > 0
        });
        if !dashing {
            return;
        }
        let old_pos = self.with_char(id, |ch, c, w| {
            c.vel = c.ninja.activation_dir * NINJA_VELOCITY as f32;
            let old_pos = ch.pos;
            let t = ch.tuning(w);
            let elasticity = vec2(
                t.get(Param::GroundElasticityX),
                t.get(Param::GroundElasticityY),
            );
            let size = vec2(PHYSICAL_SIZE, PHYSICAL_SIZE);
            w.col.move_box(&mut c.pos, &mut c.vel, size, elasticity);
            // reset velocity so the client doesn't predict stuff
            c.vel = vec2(0.0, 0.0);
            old_pos
        });
        // check if we hit anything along the way
        let radius = PHYSICAL_SIZE * 2.0;
        let found = self.find_characters(old_pos, radius);
        if self.teams.is_solo(id as i32) {
            return;
        }
        let my_team = self.teams.team(id as i32);
        let my_pos = self.chars[id].as_ref().unwrap().pos;
        for i in found {
            if i == id
                || self.teams.team(i as i32) != my_team
                || self.teams.is_solo(i as i32)
                || self.chars[id]
                    .as_ref()
                    .unwrap()
                    .hit_objects
                    .contains(&(i as i32))
                || distance(self.chars[i].as_ref().unwrap().pos, my_pos) > radius
            {
                continue;
            }
            assert!(
                self.chars[id].as_ref().unwrap().hit_objects.len() < MAX_CLIENTS,
                "m_aHitObjects overflow"
            );
            let target = self.chars[i].as_ref().unwrap().pos;
            self.sound(target, SOUND_NINJA_HIT, id as i32);
            self.chars[id].as_mut().unwrap().hit_objects.push(i as i32);
            self.take_damage(i, vec2(0.0, -10.0));
        }
    }

    /// C++: `CCharacter::HandleWeapons` (character.cpp:504).
    pub(super) fn handle_weapons(&mut self, id: usize) {
        self.handle_ninja(id);
        let fire = self.with_char(id, |ch, c, w| {
            ch.handle_jetpack(c, w);
            if ch.pain_sound_timer > 0 {
                ch.pain_sound_timer -= 1;
            }
            // check reload timer
            if ch.reload_timer != 0 {
                ch.reload_timer -= 1;
                return false;
            }
            true
        });
        if fire {
            self.fire_weapon(id);
        }
    }

    /// C++: `CCharacter::FireWeapon` (character.cpp:257); of the events only the sounds and
    /// hammer hits (the others only feed particles).
    pub(super) fn fire_weapon(&mut self, id: usize) {
        {
            let ch = self.chars[id].as_ref().unwrap();
            if ch.num_inputs < 2 || !self.config.predict_weapons || ch.reload_timer != 0 {
                return;
            }
        }
        self.with_char(id, |ch, c, w| ch.do_weapon_switch(c, w));
        let ch = self.chars[id].as_ref().unwrap().clone();
        let c = self.core.characters[id].as_ref().unwrap().clone();
        let dir = normalize(vec2(
            ch.latest_input.target_x as f32,
            ch.latest_input.target_y as f32,
        ));
        let weapon = c.active_weapon;
        let full_auto = [g::WEAPON_GRENADE, g::WEAPON_SHOTGUN, g::WEAPON_LASER].contains(&weapon)
            || (c.jetpack && weapon == g::WEAPON_GUN)
            || ch.frozen_last_tick;
        // don't fire hammer when player is deep and sv_deepfly is disabled
        if !SV_DEEPFLY && weapon == g::WEAPON_HAMMER && c.deep_frozen {
            return;
        }
        let ammo = |w: i32| c.weapons[w as usize].ammo;
        let will_fire = count_input(ch.latest_prev_input.fire, ch.latest_input.fire).0 != 0
            || (full_auto && ch.latest_input.fire & 1 != 0 && weapon >= 0 && ammo(weapon) != 0);
        if !will_fire {
            return;
        }
        if ch.freeze_time != 0 {
            // timer stuff to avoid shrieking orchestra caused by unfreeze-plasma
            let me = self.chars[id].as_mut().unwrap();
            if me.pain_sound_timer <= 0 && me.latest_prev_input.fire & 1 == 0 {
                me.pain_sound_timer = SERVER_TICK_SPEED;
                self.sound(ch.pos, SOUND_PLAYER_PAIN_LONG, id as i32);
            }
            return;
        }
        if weapon < 0 || ammo(weapon) == 0 {
            return;
        }
        let pos = ch.pos;
        let proj_start = pos + dir * PHYSICAL_SIZE * 0.75;
        let tuning = self.tunings[ch.overridden_tune_zone() as usize];
        let ticks = SERVER_TICK_SPEED as f32;
        match weapon {
            g::WEAPON_HAMMER if !c.hammer_hit_disabled => {
                self.sound(pos, SOUND_HAMMER_FIRE, id as i32);
                let mut hits = 0;
                for t in self.find_characters(proj_start, PHYSICAL_SIZE * 0.5) {
                    if t == id || !self.can_collide(id, t as i32) {
                        continue;
                    }
                    let tp = self.chars[t].as_ref().unwrap().pos;
                    let hit_pos = if length(tp - proj_start) > 0.0 {
                        tp - normalize(tp - proj_start) * PHYSICAL_SIZE * 0.5
                    } else {
                        proj_start
                    };
                    self.hammer_hit_event(hit_pos, id as i32);
                    let target = self.chars[t].as_ref().unwrap();
                    let (tpos, mr, snap_weapon, game_team) = (
                        target.pos,
                        target.move_restrictions,
                        target.last_snap_weapon,
                        target.game_team,
                    );
                    let tvel = self.core.characters[t].as_ref().unwrap().vel;
                    let d = if length(tpos - pos) > 0.0 {
                        normalize(tpos - pos)
                    } else {
                        vec2(0.0, -1.0)
                    };
                    let strength = tuning.get(Param::HammerStrength);
                    let mut temp = tvel + normalize(d + vec2(0.0, -1.1)) * 10.0;
                    temp = clamp_vel(mr, temp);
                    temp -= tvel;
                    let mut force = vec2(0.0, -1.0) + temp;
                    if self.config.is_fng {
                        if ch.game_team == game_team && snap_weapon == g::WEAPON_NINJA {
                            // melt hammer
                            force.x *= 50.0 * 0.01;
                            force.y *= 50.0 * 0.01;
                        } else {
                            force.x *= 320.0 * 0.01;
                            force.y *= 120.0 * 0.01;
                        }
                    } else {
                        force *= strength;
                    }
                    self.take_damage(t, force);
                    self.unfreeze(t);
                    hits += 1;
                    self.anti_ping_interference(id, t as i32, false, false);
                }
                // if we hit anything, we have to wait for the reload
                if hits != 0 {
                    let fire_delay = tuning.get(Param::HammerHitFireDelay);
                    self.chars[id].as_mut().unwrap().reload_timer =
                        trunc_i32(fire_delay * ticks / 1000.0);
                }
            }
            g::WEAPON_GUN if !c.jetpack => {
                let lifetime = trunc_i32(ticks * tuning.get(Param::GunLifetime));
                // C++ passes 0 as SoundImpact and -1 as Layer (its comments say otherwise)
                self.spawn_projectile(
                    g::WEAPON_GUN,
                    id as i32,
                    proj_start,
                    dir,
                    lifetime,
                    false,
                    false,
                    0,
                    -1,
                );
                self.sound(pos, SOUND_GUN_FIRE, id as i32);
            }
            g::WEAPON_SHOTGUN => {
                if self.config.is_vanilla {
                    let global = self.tunings[0];
                    const SPREADING: [f32; 5] = [-0.185, -0.070, 0.0, 0.070, 0.185];
                    for i in -2i32..=2 {
                        let a = angle(dir) + SPREADING[(i + 2) as usize];
                        let v = 1.0 - (i.abs() as f32 / 2.0);
                        let diff = global.get(Param::ShotgunSpeeddiff);
                        let speed = diff + (1.0 - diff) * v; // C++ mix
                        let lifetime = trunc_i32(ticks * global.get(Param::ShotgunLifetime));
                        let vel = direction(a) * speed;
                        self.spawn_projectile(
                            g::WEAPON_SHOTGUN,
                            id as i32,
                            proj_start,
                            vel,
                            lifetime,
                            false,
                            false,
                            -1,
                            0,
                        );
                    }
                } else if self.config.is_ddrace {
                    let reach = tuning.get(Param::LaserReach);
                    self.spawn_laser(pos, dir, reach, id as i32, g::WEAPON_SHOTGUN);
                }
                self.sound(pos, SOUND_SHOTGUN_FIRE, id as i32);
            }
            g::WEAPON_GRENADE => {
                let lifetime = trunc_i32(ticks * tuning.get(Param::GrenadeLifetime));
                self.spawn_projectile(
                    g::WEAPON_GRENADE,
                    id as i32,
                    proj_start,
                    dir,
                    lifetime,
                    false,
                    true,
                    SOUND_GRENADE_EXPLODE,
                    0,
                );
                self.sound(pos, SOUND_GRENADE_FIRE, id as i32);
            }
            g::WEAPON_LASER => {
                let reach = tuning.get(Param::LaserReach);
                self.spawn_laser(pos, dir, reach, id as i32, g::WEAPON_LASER);
                self.sound(pos, SOUND_LASER_FIRE, id as i32);
            }
            g::WEAPON_NINJA => {
                let me = self.chars[id].as_mut().unwrap();
                me.hit_objects.clear();
                let c = self.core.characters[id].as_mut().unwrap();
                c.ninja.activation_dir = dir;
                c.ninja.current_move_time = NINJA_MOVETIME * SERVER_TICK_SPEED / 1000;
                // clamp to prevent massive MoveBox calculation lag with SG bug
                c.ninja.old_vel_amount = trunc_i32(length(c.vel).clamp(0.0, 6000.0));
                self.sound(pos, SOUND_NINJA_FIRE, id as i32);
            }
            _ => {}
        }
        let tick = self.game_tick;
        let active = self.core.characters[id].as_ref().unwrap().active_weapon;
        let me = self.chars[id].as_mut().unwrap();
        me.attack_tick = tick;
        // -1 is no weapon, handled here so pain sound still plays when firing in freeze
        if me.reload_timer == 0 && active != -1 {
            me.reload_timer = trunc_i32(fire_delay(&tuning, active) * ticks);
        }
    }
}
