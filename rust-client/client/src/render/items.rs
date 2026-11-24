//! C++: `CItems` (game/client/components/items.cpp): projectiles, pickups, lasers
//! (incl. doors, draggers, freeze lasers and guns) and flags from the snapshot, with the
//! default config (`cl_antiping 0`, so nothing is weapon-predicted).
//!
//! Not drawn: particles (smoke and bullet trails, powerup shine). Tuning is the default
//! for every tune zone (`Sv_TuneParams` is not handled yet).

use std::time::Instant;

use ddnet::items::{
    LaserData, ProjectileData, calc_pos, extract_laser, extract_pickup, extract_projectile, pad,
    snap_entities,
};
use ddnet::net::client::Client;
use ddnet::physics::collision::Collision;
use ddnet::physics::tuning::{Param, Tuning};
use ddnet::physics::vmath::{PI, Vec2, angle, direction, distance, length, mix, vec2};
use ddnet::predict::{Predictor, Rendered};
use ddnet::protocol::generated::{self as g, ObjFlag, ObjGameData};
use ddnet::protocol::snapshot::Item;

use super::content::{
    SPRITE_FLAG_BLUE, SPRITE_FLAG_RED, SPRITE_PART_HECTAGON, SPRITE_PART_PULLEY,
    SPRITE_PART_SNOWFLAKE, SPRITE_PART_SPLAT01, SPRITE_PICKUP_ARMOR, SPRITE_PICKUP_ARMOR_SHOTGUN,
    SPRITE_PICKUP_FREEZE, SPRITE_PICKUP_HEALTH, SPRITE_PICKUP_NINJA, SPRITES, WEAPONS,
};
use super::map::{hsl_to_rgb, unpack_hsl};
use super::tee::Sprites;
use crate::gfx::batch::{Batch, ContainerQuad, FreeformItem};

const TICK_SPEED: i32 = 50;
/// C++ `cl_show_others_alpha` default.
const SHOW_OTHERS_ALPHA: f32 = 40.0 / 100.0;
/// C++ `cl_laser_*_outline_color` / `cl_laser_*_inner_color` defaults (packed HSL) for
/// rifle, shotgun, door, freeze and dragger.
const LASER_RIFLE: (i32, i32) = (11176233, 11206591);
const LASER_SHOTGUN: (i32, i32) = (1866773, 1467241);
const LASER_DOOR: (i32, i32) = (7667473, 7701379);
const LASER_FREEZE: (i32, i32) = (11613223, 12001153);
const LASER_DRAGGER: (i32, i32) = (57618, 42398);

/// C++: `GetSpriteScale`.
fn sprite_scale(sprite: usize) -> (f32, f32) {
    let s = &SPRITES[sprite];
    let f = ((s.w * s.w + s.h * s.h) as f32).sqrt();
    (s.w as f32 / f, s.h as f32 / f)
}

/// C++: `CScreenRect::Inside` for (left, top, right, bottom).
fn inside(rect: &[f32; 4], p: Vec2) -> bool {
    p.x >= rect[0] && p.x <= rect[2] && p.y >= rect[1] && p.y <= rect[3]
}

/// C++: `rotate` (vmath.h): by `angle` degrees.
fn rotate(a: Vec2, angle: f32) -> Vec2 {
    let angle = angle * PI / 180.0;
    let (s, c) = (angle.sin(), angle.cos());
    vec2(c * a.x - s * a.y, s * a.x + c * a.y)
}

/// The quad container of `CItems::OnInit` (items.cpp:646).
struct Quads {
    flag: ContainerQuad,
    health: ContainerQuad,
    armor: ContainerQuad,
    freeze: ContainerQuad,
    weapons: [ContainerQuad; 6],
    ninja: ContainerQuad,
    weapon_armor: [ContainerQuad; 4],
    projectile: ContainerQuad,
    splat: ContainerQuad,
    pulley: ContainerQuad,
    freeze_head: ContainerQuad,
    door_head: ContainerQuad,
}

const FULL: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

impl Default for Quads {
    fn default() -> Self {
        let scaled = |sprite: usize, size: f32| {
            let (sx, sy) = sprite_scale(sprite);
            ContainerQuad::centered(size * sx, size * sy, FULL)
        };
        // C++: QuadContainerAddSprite(Index, Size) with Size = 20 * ScaleX
        let square = |sprite: usize| {
            let size = 20.0 * sprite_scale(sprite).0;
            ContainerQuad::centered(size, size, FULL)
        };
        Self {
            flag: ContainerQuad::new(-21.0, -42.0, 42.0, 84.0, FULL),
            health: scaled(SPRITE_PICKUP_HEALTH, 64.0),
            armor: scaled(SPRITE_PICKUP_ARMOR, 64.0),
            freeze: scaled(SPRITE_PICKUP_FREEZE, 64.0),
            weapons: std::array::from_fn(|i| {
                scaled(WEAPONS[i].sprite_body, WEAPONS[i].visual_size as f32)
            }),
            ninja: scaled(SPRITE_PICKUP_NINJA, 128.0),
            weapon_armor: std::array::from_fn(|i| scaled(SPRITE_PICKUP_ARMOR_SHOTGUN + i, 64.0)),
            projectile: ContainerQuad::centered(32.0, 32.0, FULL),
            splat: ContainerQuad::centered(24.0, 24.0, FULL),
            pulley: square(SPRITE_PART_PULLEY),
            freeze_head: square(SPRITE_PART_HECTAGON),
            door_head: ContainerQuad::new(0.0, 0.0, 16.0, 16.0, FULL),
        }
    }
}

/// What `CItems` needs from the rest of the client this frame.
pub struct Frame<'a> {
    pub client: &'a Client,
    pub predictor: &'a Predictor,
    pub rendered: &'a [Rendered],
    pub collision: &'a Collision,
    pub sprites: &'a Sprites,
}

pub struct Items {
    quads: Quads,
    /// C++ `LocalTime()` for the pickup bobbing and grenade spin: seconds since start.
    start: Instant,
}

impl Default for Items {
    fn default() -> Self {
        Self {
            quads: Quads::default(),
            start: Instant::now(),
        }
    }
}

impl Items {
    /// C++: `CItems::OnRender` (items.cpp:455), screen set to the world.
    pub fn render(&self, batch: &mut Batch, f: &Frame) {
        let client = f.client;
        let Some(game_tick) = client.game_tick() else {
            return;
        };
        let snap = client.snapshot();
        let prev_snap = client
            .prev_game_tick()
            .and_then(|t| client.snapshot_at(t))
            .unwrap_or(&[]);
        let local = client.local_id();
        // C++: IsLocalCharSuper
        let is_super = local.is_some_and(|id| {
            f.rendered
                .iter()
                .find(|r| r.id == id)
                .and_then(|r| r.ext)
                .is_some_and(|e| e.flags & g::CHARACTERFLAG_SUPER != 0)
        });
        let ticks = game_tick % TICK_SPEED;
        let blinking_pickup = (ticks % 22) < 4;
        let (blinking_gun, blinking_dragger) = (blinking_pickup, blinking_pickup);
        let blinking_proj = (ticks % 20) < 2;
        let blinking_proj_ex = (ticks % 6) < 2;
        let blinking_light = (ticks % 6) < 2;
        let dragger_start_tick = ((game_tick / 7) * 7).max(game_tick - 4);
        let gun_start_tick = (game_tick / 7) * 7;

        let [l, t, r, b] = batch.screen();
        const TILE_SIZE: f32 = 64.0;
        let proj_rect = [l - TILE_SIZE, t - TILE_SIZE, r + TILE_SIZE, b + TILE_SIZE];
        let laser_rect = [
            l - TILE_SIZE / 2.0,
            t - TILE_SIZE / 2.0,
            r + TILE_SIZE / 2.0,
            b + TILE_SIZE / 2.0,
        ];
        let pickup_rect = [
            l - 1.75 * TILE_SIZE,
            t - 0.75 * TILE_SIZE,
            r + 1.75 * TILE_SIZE,
            b + 0.75 * TILE_SIZE,
        ];
        let laser_inside = |d: &LaserData| {
            let [l, t, r, b] = laser_rect;
            !((d.from.x < l && d.to.x < l)
                || (d.from.x > r && d.to.x > r)
                || (d.from.y < t && d.to.y < t)
                || (d.from.y > b && d.to.y > b))
        };
        // C++ m_GameWorld.m_WorldConfig.m_UseTuneZones: what the predictor uses
        let tunes = f.predictor.use_tune_zones().then_some(f.collision);

        for (index, ex) in snap_entities(snap) {
            let item = &snap[index];
            let data = pad(&item.data, 10);
            let inactive = |number: i32| !is_super && f.predictor.switch_inactive(number);
            match item.type_id {
                g::NETOBJTYPE_PROJECTILE
                | g::NETOBJTYPE_DDRACEPROJECTILE
                | g::NETOBJTYPE_DDNETPROJECTILE => {
                    let d = extract_projectile(item.type_id, &data, tunes, ex.as_ref());
                    let blinking = if d.explosive {
                        blinking_proj_ex
                    } else {
                        blinking_proj
                    };
                    if inactive(d.switch_number) && blinking {
                        continue;
                    }
                    self.render_projectile(batch, f, &d, item.id, &proj_rect);
                }
                g::NETOBJTYPE_PICKUP | g::NETOBJTYPE_DDNETPICKUP => {
                    let d = extract_pickup(item.type_id, &data, ex.as_ref());
                    if !inside(&pickup_rect, d.pos)
                        || (inactive(d.switch_number) && blinking_pickup)
                    {
                        continue;
                    }
                    if let Some(prev) = prev_snap
                        .iter()
                        .find(|p| p.type_id == item.type_id && p.id == item.id)
                    {
                        // C++ reads both objects as CNetObj_Pickup (same first ints)
                        let prev = pad(&prev.data, 4);
                        let p = vec2(prev[0] as f32, prev[1] as f32);
                        self.render_pickup(batch, f.sprites, p, &data, client.intra_tick, d.flags);
                    }
                }
                g::NETOBJTYPE_LASER | g::NETOBJTYPE_DDNETLASER => {
                    let mut d = extract_laser(item.type_id, &data, tunes, ex.as_ref());
                    if !laser_inside(&d) {
                        continue;
                    }
                    let inactive = inactive(d.switch_number);
                    let (blink, ent_start_tick) = match d.type_ {
                        g::LASERTYPE_FREEZE => (blinking_light, dragger_start_tick),
                        g::LASERTYPE_GUN => (blinking_gun, gun_start_tick),
                        g::LASERTYPE_DRAGGER => (blinking_dragger, dragger_start_tick),
                        g::LASERTYPE_DOOR => {
                            if d.predict && (inactive || is_super) {
                                d.from = d.to;
                            }
                            (false, game_tick)
                        }
                        _ => (blinking_dragger, game_tick),
                    };
                    if d.predict && inactive && blink {
                        continue;
                    }
                    if d.start_tick <= 0 && ent_start_tick != -1 {
                        d.start_tick = ent_start_tick;
                    }
                    self.render_laser(batch, f, &d);
                }
                _ => {}
            }
        }
        self.render_flags(batch, f, snap, prev_snap);
        batch.set_rotation(0.0);
        batch.set_color([1.0; 4]);
    }

    /// C++: alpha of entities owned by players in other teams.
    fn owner_alpha(f: &Frame, extra_info: bool, owner: i32) -> f32 {
        if extra_info && owner >= 0 && f.predictor.is_other_team(owner) {
            SHOW_OTHERS_ALPHA
        } else {
            1.0
        }
    }

    /// C++: `CItems::RenderProjectile` (items.cpp:22).
    fn render_projectile(
        &self,
        batch: &mut Batch,
        f: &Frame,
        d: &ProjectileData,
        item_id: i32,
        rect: &[f32; 4],
    ) {
        let weapon = d.type_.clamp(0, 5);
        // ponytail: default tuning for every zone until Sv_TuneParams is handled
        let tuning = Tuning::default();
        let (curvature, speed) = match weapon {
            g::WEAPON_GRENADE => (
                tuning.get(Param::GrenadeCurvature),
                tuning.get(Param::GrenadeSpeed),
            ),
            g::WEAPON_SHOTGUN => (
                tuning.get(Param::ShotgunCurvature),
                tuning.get(Param::ShotgunSpeed),
            ),
            g::WEAPON_GUN => (tuning.get(Param::GunCurvature), tuning.get(Param::GunSpeed)),
            _ => (0.0, 0.0),
        };
        let client = f.client;
        // C++ s_LastGameTickTime: GameTickTime while the world runs
        let last_tick_time = client.game_tick_time;
        let prev_tick = client.prev_game_tick().unwrap_or(0);
        let mut ct = (prev_tick - d.start_tick) as f32 / TICK_SPEED as f32 + last_tick_time;
        if ct < 0.0 {
            if ct > -last_tick_time / 2.0 {
                ct = 0.0;
            } else {
                return; // projectile haven't been shot yet
            }
        }
        let pos = calc_pos(d.start_pos, d.start_vel, curvature, speed, ct);
        if !inside(rect, pos) {
            return;
        }
        let prev = calc_pos(d.start_pos, d.start_vel, curvature, speed, ct - 0.001);
        let alpha = Self::owner_alpha(f, d.extra_info, d.owner);
        let vel = pos - prev;
        if weapon == g::WEAPON_GRENADE {
            // C++ s_Time: local time while grenades are drawn
            let time = self.start.elapsed().as_secs_f32();
            batch.set_rotation(time * PI * 2.0 * 2.0 + item_id as f32);
        } else if length(vel) > 0.00001 {
            batch.set_rotation(angle(vel));
        } else {
            batch.set_rotation(0.0);
        }
        if let Some(tex) = f.sprites.get(WEAPONS[weapon as usize].sprite_proj) {
            batch.texture_set(Some(tex));
            batch.set_color([1.0, 1.0, 1.0, alpha]);
            batch.container_quad(&self.quads.projectile, pos.x, pos.y, 1.0, 1.0);
        }
    }

    /// C++: `CItems::RenderPickup` (items.cpp:129). `cur` holds the current object's ints.
    fn render_pickup(
        &self,
        batch: &mut Batch,
        sprites: &Sprites,
        prev: Vec2,
        cur: &[i32],
        intra: f32,
        flags: i32,
    ) {
        let (ty, subtype) = (cur[2], cur[3]);
        let weapon = subtype.clamp(0, 5) as usize;
        let mut pos = mix(prev, vec2(cur[0] as f32, cur[1] as f32), intra);
        let q = &self.quads;
        let (quad, sprite) = match ty {
            g::POWERUP_HEALTH => (&q.health, SPRITE_PICKUP_HEALTH),
            g::POWERUP_FREEZE => (&q.freeze, SPRITE_PICKUP_FREEZE),
            g::POWERUP_ARMOR => (&q.armor, SPRITE_PICKUP_ARMOR),
            g::POWERUP_WEAPON => (&q.weapons[weapon], WEAPON_PICKUPS[weapon]),
            g::POWERUP_NINJA => (&q.ninja, SPRITE_PICKUP_NINJA),
            g::POWERUP_ARMOR_SHOTGUN..=g::POWERUP_ARMOR_LASER => {
                let i = (ty - g::POWERUP_ARMOR_SHOTGUN) as usize;
                (&q.weapon_armor[i], SPRITE_PICKUP_ARMOR_SHOTGUN + i)
            }
            _ => return,
        };
        batch.texture_set(sprites.get(sprite));
        batch.set_rotation(0.0);
        batch.set_color([1.0; 4]);
        let mut scale = vec2(1.0, 1.0);
        if flags & g::PICKUPFLAG_XFLIP != 0 {
            scale.x = -scale.x;
        }
        if flags & g::PICKUPFLAG_YFLIP != 0 {
            scale.y = -scale.y;
        }
        let ninja_shift = if flags & g::PICKUPFLAG_XFLIP != 0 {
            10.0
        } else {
            -10.0
        };
        if flags & g::PICKUPFLAG_ROTATE != 0 {
            batch.set_rotation(90.0 * (PI / 180.0));
            scale = vec2(scale.y, scale.x);
            if ty == g::POWERUP_NINJA {
                pos.y += ninja_shift;
            }
        } else if ty == g::POWERUP_NINJA {
            pos.x += ninja_shift;
        }
        // C++ s_Time: local time while pickups are drawn
        let time = self.start.elapsed().as_secs_f32();
        let offset = pos.y / 32.0 + pos.x / 32.0;
        pos += direction(time * 2.0 + offset) * 2.5;
        batch.container_quad(quad, pos.x, pos.y, scale.x, scale.y);
        batch.set_rotation(0.0);
    }

    /// C++: `CItems::RenderFlags`/`RenderFlag` (items.cpp:219): flags with a previous
    /// object, at their carrier if it is active.
    fn render_flags(&self, batch: &mut Batch, f: &Frame, snap: &[Item], prev_snap: &[Item]) {
        let game_data = |items: &[Item]| {
            items
                .iter()
                .find(|it| it.type_id == g::NETOBJTYPE_GAMEDATA)
                .map(|it| ObjGameData::from_ints(&pad(&it.data, ObjGameData::NUM_INTS)))
        };
        let (cur_data, prev_data) = (game_data(snap), game_data(prev_snap));
        for item in snap.iter().filter(|it| it.type_id == g::NETOBJTYPE_FLAG) {
            let Some(prev) = prev_snap
                .iter()
                .find(|p| p.type_id == item.type_id && p.id == item.id)
            else {
                continue;
            };
            let cur = ObjFlag::from_ints(&pad(&item.data, ObjFlag::NUM_INTS));
            let prev = ObjFlag::from_ints(&pad(&prev.data, ObjFlag::NUM_INTS));
            let mut pos = mix(
                vec2(prev.x as f32, prev.y as f32),
                vec2(cur.x as f32, cur.y as f32),
                f.client.intra_tick,
            );
            if let Some(d) = cur_data {
                let red = cur.team == g::TEAM_RED;
                let carrier = if red {
                    d.flag_carrier_red
                } else {
                    d.flag_carrier_blue
                };
                // use the flag carrier's position if available
                if let Some(r) = f.rendered.iter().find(|r| r.id == carrier) {
                    pos = r.pos;
                }
                // make sure that the flag isn't interpolated between capture and return
                if let Some(p) = prev_data
                    && ((red && p.flag_carrier_red != d.flag_carrier_red)
                        || (cur.team == g::TEAM_BLUE && p.flag_carrier_blue != d.flag_carrier_blue))
                {
                    pos = vec2(cur.x as f32, cur.y as f32);
                }
            }
            const SIZE: f32 = 42.0;
            let sprite = if cur.team == g::TEAM_RED {
                SPRITE_FLAG_RED
            } else {
                SPRITE_FLAG_BLUE
            };
            batch.texture_set(f.sprites.get(sprite));
            batch.set_rotation(0.0);
            batch.set_color([1.0; 4]);
            batch.container_quad(&self.quads.flag, pos.x, pos.y - SIZE * 0.75, 1.0, 1.0);
        }
    }

    /// C++: `CItems::RenderLaser(const CLaserData *, bool)` (items.cpp:262), not predicted.
    fn render_laser(&self, batch: &mut Batch, f: &Frame, d: &LaserData) {
        let ty = d.type_.clamp(-1, g::NUM_LASERTYPES - 1);
        let (out, inn) = match ty {
            g::LASERTYPE_RIFLE => LASER_RIFLE,
            g::LASERTYPE_SHOTGUN => LASER_SHOTGUN,
            g::LASERTYPE_DOOR => LASER_DOOR,
            g::LASERTYPE_FREEZE => LASER_FREEZE,
            g::LASERTYPE_DRAGGER => LASER_DRAGGER,
            g::LASERTYPE_GUN | g::LASERTYPE_PLASMA
                if d.subtype == g::LASERGUNTYPE_FREEZE
                    || d.subtype == g::LASERGUNTYPE_EXPFREEZE =>
            {
                LASER_FREEZE
            }
            _ => LASER_RIFLE,
        };
        let alpha = Self::owner_alpha(f, d.extra_info, d.owner);
        // C++: color_cast<ColorRGBA>(ColorHSLA(Packed).WithAlpha(Alpha))
        let color = |packed: i32| {
            let [r, g, b] = hsl_to_rgb(unpack_hsl(packed as u32));
            [r, g, b, alpha]
        };
        let game_tick = f.client.game_tick().unwrap_or(0);
        let intra = f.client.intra_tick;
        let ticks = if ty == g::LASERTYPE_DOOR {
            1.0
        } else {
            (game_tick - d.start_tick) as f32 + intra
        };
        let mut ticks_head = game_tick as f32 + intra;
        if ty == g::LASERTYPE_DRAGGER {
            ticks_head *= (((d.subtype >> 1) % 3) as f32 * 4.0) + 1.0;
            ticks_head *= if d.subtype & 1 != 0 { -1.0 } else { 1.0 };
        }
        self.draw_laser(
            batch,
            f,
            d.from,
            d.to,
            color(out),
            color(inn),
            ticks,
            ticks_head,
            ty,
        );
    }

    /// C++: `CItems::RenderLaser(From, Pos, ...)` (items.cpp:339).
    #[allow(clippy::too_many_arguments)] // the C++ signature
    fn draw_laser(
        &self,
        batch: &mut Batch,
        f: &Frame,
        from: Vec2,
        pos: Vec2,
        outer: [f32; 4],
        inner: [f32; 4],
        mut ticks_body: f32,
        ticks_head: f32,
        ty: i32,
    ) {
        let len = distance(pos, from);
        if len > 0.0 {
            if ty == g::LASERTYPE_DRAGGER {
                // rubber band effect
                let thickness = len.sqrt() / 5.0;
                ticks_body = thickness.clamp(1.0, 5.0);
            }
            // C++: normalize_pre_length
            let v = pos - from;
            let dir = vec2(v.x / len, v.y / len);
            let ms = ticks_body * 1000.0 / TICK_SPEED as f32;
            // ponytail: default tuning in every tune zone (C++ looks up the laser's zone)
            let a = (ms / Tuning::default().get(Param::LaserBounceDelay)).clamp(0.0, 1.0);
            let ia = 1.0 - a;
            batch.texture_set(None);
            batch.quads_begin();
            // do outline
            batch.set_color(outer);
            let out = vec2(dir.y, -dir.x) * (7.0 * ia);
            let p = |a: Vec2| [a.x, a.y];
            batch.quads_draw_freeform(&[FreeformItem([
                p(from - out),
                p(from + out),
                p(pos - out),
                p(pos + out),
            ])]);
            // do inner
            let out = vec2(dir.y, -dir.x) * (5.0 * ia);
            let extra_pos = dir;
            let extra_from = if ty == g::LASERTYPE_DOOR {
                vec2(0.0, 0.0)
            } else {
                dir
            };
            batch.set_color(inner);
            batch.quads_draw_freeform(&[FreeformItem([
                p(from - out + extra_from),
                p(from + out + extra_from),
                p(pos - out - extra_pos),
                p(pos + out - extra_pos),
            ])]);
        }
        // render head
        let sprites = f.sprites;
        let q = &self.quads;
        if ty == g::LASERTYPE_DOOR {
            batch.texture_set(None);
            batch.set_rotation(0.0);
            batch.set_color(outer);
            batch.container_quad(&q.door_head, pos.x - 8.0, pos.y - 8.0, 1.0, 1.0);
            batch.set_color(inner);
            batch.container_quad(&q.door_head, pos.x - 6.0, pos.y - 6.0, 6.0 / 8.0, 6.0 / 8.0);
        } else if ty == g::LASERTYPE_DRAGGER {
            batch.texture_set(sprites.get(SPRITE_PART_PULLEY));
            for inner_pass in [false, true] {
                batch.set_color(if inner_pass { inner } else { outer });
                let size = if inner_pass { 4.0 / 5.0 } else { 1.0 };
                // circle at laser end
                if len > 0.0 {
                    batch.set_rotation(0.0);
                    batch.container_quad(&q.pulley, from.x, from.y, size, size);
                }
                // rotating orbs
                let size = if inner_pass { 0.75 - 1.0 / 5.0 } else { 0.75 };
                for orb in 0..3 {
                    let offset = rotate(vec2(10.0, 0.0), (orb * 120) as f32 + ticks_head);
                    batch.set_rotation(ticks_head + orb as f32 * PI * 2.0 / 3.0);
                    batch.container_quad(
                        &q.pulley,
                        from.x + offset.x,
                        from.y + offset.y,
                        size,
                        size,
                    );
                }
            }
        } else if ty == g::LASERTYPE_FREEZE {
            let pulsation = 6.0 / 5.0 + 1.0 / 10.0 * (ticks_head / 2.0).sin();
            batch.texture_set(sprites.get(SPRITE_PART_HECTAGON));
            batch.set_rotation(angle(pos - from));
            batch.set_color(outer);
            let s = 6.0 / 5.0 * pulsation;
            batch.container_quad(&q.freeze_head, pos.x, pos.y, s, s);
            batch.texture_set(sprites.get(SPRITE_PART_SNOWFLAKE));
            // snowflakes are white
            batch.set_color([1.0; 4]);
            batch.container_quad(&q.freeze_head, pos.x, pos.y, pulsation, pulsation);
        } else {
            // (C++ `(int)TicksHead % 3`; never negative here, rem_euclid keeps it in range)
            let particle = ddnet::physics::vmath::trunc_i32(ticks_head).rem_euclid(3) as usize;
            batch.texture_set(sprites.get(SPRITE_PART_SPLAT01 + particle));
            batch.set_rotation(ddnet::physics::vmath::trunc_i32(ticks_head) as f32);
            batch.set_color(outer);
            batch.container_quad(&q.splat, pos.x, pos.y, 1.0, 1.0);
            batch.set_color(inner);
            batch.container_quad(&q.splat, pos.x, pos.y, 20.0 / 24.0, 20.0 / 24.0);
        }
    }
}

/// C++: `m_GameSkin.m_aSpritePickupWeapons`: hammer, gun, shotgun, grenade, laser, ninja.
const WEAPON_PICKUPS: [usize; 6] = [
    super::content::SPRITE_PICKUP_HAMMER,
    super::content::SPRITE_PICKUP_GUN,
    super::content::SPRITE_PICKUP_SHOTGUN,
    super::content::SPRITE_PICKUP_GRENADE,
    super::content::SPRITE_PICKUP_LASER,
    super::content::SPRITE_PICKUP_NINJA,
];
