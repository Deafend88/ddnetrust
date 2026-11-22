//! C++: `CPlayers` (game/client/components/players.cpp): `OnRender`, `RenderPlayer`
//! and `RenderHook`, plus the per-client skin data `CGameClient` keeps from the
//! snapshot (`ClientInfo`, `DDNetCharacter`, `DDNetPlayer`, game info flags).
//!
//! Drawn from the current snapshot only (previous = current, no prediction): 6.5 adds
//! interpolation. Not drawn yet: muzzle flashes, the ninja hadoken, emoticon messages
//! (`Sv_Emoticon` is not handled), the hook collision line (only with the aim flag), particles (freeze
//! flakes, sparkles, skid trails, powerup shine), the unpredicted shadow (default off).

use std::collections::HashMap;

use ddnet::map::ints_to_str;
use ddnet::net::client::Client;
use ddnet::physics::collision::Collision;
use ddnet::physics::vmath::{PI, Vec2, angle, direction, distance, mix, normalize, vec2};
use ddnet::protocol::generated::{self as g, ObjCharacter};
use ddnet::protocol::snapshot::Item;

use super::anim::AnimState;
use super::content::{
    ANIM_BASE, ANIM_HAMMER_SWING, ANIM_IDLE, ANIM_INAIR, ANIM_NINJA_SWING, ANIM_RUN_LEFT,
    ANIM_RUN_RIGHT, ANIM_SIT_LEFT, ANIM_SIT_RIGHT, ANIM_WALK, ANIMATIONS, SPRITE_DOTDOT,
    SPRITE_OOP, SPRITE_ZZZ, WEAPONS,
};
use super::tee::{
    GameSkin, QUAD_EMOTICONS, QUAD_HOOK_CHAIN, QUAD_HOOK_HEAD, Quads, Skin, TEE_NO_WEAPON,
    TeeRenderInfo, render_hand, render_tee,
};
use crate::assets;
use crate::gfx::Gfx;
use crate::gfx::batch::Batch;
use crate::skin::load_skin_data;
use crate::storage::Storage;

/// C++ `MAX_CLIENTS` (engine/shared/protocol.h).
const MAX_CLIENTS: i32 = 128;
/// C++ `SERVER_TICK_SPEED`.
const TICK_SPEED: f32 = 50.0;

/// Skins by name, loaded on first use (C++ loads them in the background; we have no
/// skin download, so missing skins are `None` and fall back to `default`).
#[derive(Default)]
pub struct Skins(HashMap<String, Option<Skin>>);

impl Skins {
    fn get(&mut self, gfx: &mut Gfx, storage: &Storage, name: &str) -> Option<Skin> {
        *self.0.entry(name.to_string()).or_insert_with(|| {
            let data = load_skin_data(assets::load_skin(storage, name)?)?;
            Some(Skin::upload(gfx, &data))
        })
    }

    /// C++: `CSkins::Find`: the skin, else `default` (else nothing is drawn).
    pub fn find(&mut self, gfx: &mut Gfx, storage: &Storage, name: &str) -> Option<Skin> {
        self.get(gfx, storage, name)
            .or_else(|| self.get(gfx, storage, "default"))
    }
}

/// C++: the walk and run animation times of `RenderPlayer` (players.cpp:641).
pub fn walk_run_times(x: f32) -> (f32, f32) {
    let mut walk_time = (x % 100.0) / 100.0;
    let mut run_time = (x % 200.0) / 200.0;
    // Don't do a moon walk outside the left border
    if walk_time < 0.0 {
        walk_time += 1.0;
    }
    if run_time < 0.0 {
        run_time += 1.0;
    }
    (walk_time, run_time)
}

pub struct Players {
    quads: Quads,
    game_skin: GameSkin,
    skins: Skins,
}

/// A snapshot item's ints as an object (`SecureUnpackObj` already sized them).
fn find(items: &[Item], type_id: i32, id: i32) -> Option<&[i32]> {
    items
        .iter()
        .find(|it| it.type_id == type_id && it.id == id)
        .map(|it| &it.data[..])
}

/// One client as `CPlayers::OnRender` sees it.
struct Rendered {
    id: i32,
    prev: ObjCharacter,
    cur: ObjCharacter,
    /// C++ `m_RenderPos`.
    pos: Vec2,
    info: TeeRenderInfo,
    /// C++ `m_HasExtendedDisplayInfo`: the DDNet target.
    target: Option<Vec2>,
    inactive: bool,
    afk: bool,
}

impl Players {
    pub fn new(gfx: &mut Gfx, storage: &Storage) -> Self {
        // C++ cl_asset_game and cl_asset_emoticons "default"
        let game_skin = GameSkin::upload(
            gfx,
            assets::load_asset(storage, assets::GAME, "default"),
            assets::load_asset(storage, assets::EMOTICONS, "default"),
        );
        Self {
            quads: Quads::default(),
            game_skin,
            skins: Skins::default(),
        }
    }

    /// C++: `CPlayers::OnRender`, with the screen set to the world (game group).
    pub fn render(
        &mut self,
        gfx: &mut Gfx,
        storage: &Storage,
        batch: &mut Batch,
        client: &Client,
        collision: &Collision,
    ) {
        let snap = client.snapshot();
        let prev_snap = client
            .prev_game_tick()
            .and_then(|t| client.snapshot_at(t))
            .unwrap_or(&[]);
        let local = client.local_id();
        let info_ex = snap
            .iter()
            .find(|it| it.type_id == g::NETOBJTYPE_GAMEINFOEX)
            .map(|it| g::ObjGameInfoEx::from_ints(&it.data));
        let flags2 = |version: i32, flag: i32| {
            info_ex.is_some_and(|i| i.version >= version && i.flags2 & flag != 0)
        };
        let allow_x_skins = flags2(5, g::GAMEINFOFLAG2_ALLOW_X_SKINS);
        let no_skin_change_for_frozen = flags2(9, g::GAMEINFOFLAG2_NO_SKIN_CHANGE_FOR_FROZEN);
        // C++: IsTeamPlay
        let team_play = find(snap, g::NETOBJTYPE_GAMEINFO, 0)
            .is_some_and(|d| g::ObjGameInfo::from_ints(d).game_flags & g::GAMEFLAG_TEAMS != 0);

        let mut players = Vec::new();
        for item in snap
            .iter()
            .filter(|it| it.type_id == g::NETOBJTYPE_CHARACTER)
        {
            let id = item.id;
            // C++: IsPlayerInfoAvailable (the character also needs a previous snapshot)
            if !(0..MAX_CLIENTS).contains(&id)
                || find(prev_snap, g::NETOBJTYPE_CHARACTER, id).is_none()
                || find(snap, g::NETOBJTYPE_PLAYERINFO, id).is_none()
                || find(prev_snap, g::NETOBJTYPE_PLAYERINFO, id).is_none()
            {
                continue;
            }
            let cur = ObjCharacter::from_ints(&item.data);
            let player =
                g::ObjPlayerInfo::from_ints(find(snap, g::NETOBJTYPE_PLAYERINFO, id).unwrap());
            let ddnet =
                find(snap, g::NETOBJTYPE_DDNETCHARACTER, id).map(g::ObjDDNetCharacter::from_ints);
            let ex_player =
                find(snap, g::NETOBJTYPE_DDNETPLAYER, id).map(g::ObjDDNetPlayer::from_ints);

            // C++: OnNewSnapshot CLIENTINFO, UpdateSkinInfo, UpdateRenderInfo
            let client_info =
                find(snap, g::NETOBJTYPE_CLIENTINFO, id).map(g::ObjClientInfo::from_ints);
            let mut skin_name = client_info
                .and_then(|c| ints_to_str(&c.skin))
                .unwrap_or_default();
            if !assets::valid_skin_name(&skin_name)
                || (!allow_x_skins && skin_name.to_ascii_lowercase().starts_with("x_"))
            {
                skin_name = "default".into();
            }
            let Some(skin) = self.skins.find(gfx, storage, &skin_name) else {
                continue;
            };
            let mut info = TeeRenderInfo::new(skin);
            if let Some(c) = client_info {
                info.apply_colors(c.use_custom_color != 0, c.color_body, c.color_feet);
            }
            info.size = 64.0;
            if team_play {
                info.custom_colored = true;
                if (g::TEAM_RED..=g::TEAM_BLUE).contains(&player.team) {
                    const TEAM_COLORS: [u32; 2] = [65461, 10223541];
                    let [r, gr, b] =
                        super::map::hsl_packed_to_rgb(TEAM_COLORS[player.team as usize]);
                    info.color_body = [r, gr, b, 1.0];
                    info.color_feet = info.color_body;
                } else {
                    info.color_body = [1.0; 4];
                    info.color_feet = [1.0; 4];
                }
            }

            // C++: CPlayers::OnRender, freeze and ninja
            // ponytail: snapshot freeze state also for the local tee (C++: predicted), 6.5
            let frozen = ddnet.is_some_and(|d| d.freeze_end != 0);
            // (the TEE_EFFECT_* flags only feed particles, which are not drawn)
            if frozen {
                info.flags |= TEE_NO_WEAPON;
            }
            // C++ cl_show_ninja 1
            if (cur.weapon == g::WEAPON_NINJA || (frozen && !no_skin_change_for_frozen))
                && let Some(ninja) = self.skins.find(gfx, storage, "x_ninja")
            {
                info.skin = ninja;
                info.custom_colored = team_play;
                if !team_play {
                    info.color_body = [1.0; 4];
                    info.color_feet = [1.0; 4];
                }
            }
            let target = ddnet
                .filter(|d| d.jumped_total != -1)
                .map(|d| vec2(d.target_x as f32, d.target_y as f32));
            let ex_flags = ex_player.map_or(0, |p| p.flags);
            let afk = ex_flags & g::EXPLAYERFLAG_AFK != 0;
            let inactive = afk || ex_flags & g::EXPLAYERFLAG_PAUSED != 0;
            players.push(Rendered {
                id,
                prev: cur,
                cur,
                pos: vec2(cur.x as f32, cur.y as f32),
                info,
                target,
                inactive,
                afk,
            });
        }

        // C++: GetScreen, expanded by 100 for tees, weapons and emotes
        let [l, t, r, b] = batch.screen();
        let screen = [l - 100.0, t - 100.0, r + 100.0, b + 100.0];
        let by_id = |id: i32| players.iter().find(|p| p.id == id);
        let ordered = players
            .iter()
            .filter(|p| Some(p.id) != local)
            .chain(local.and_then(by_id));
        // render everyone else's hook, then our own
        for p in ordered.clone() {
            self.render_hook(batch, client, &screen, p, &by_id);
        }
        // render everyone else's tee, then our own
        for p in ordered {
            self.render_player(batch, client, collision, &screen, p, local);
        }
    }

    /// C++: `CPlayers::RenderHook` (players.cpp:477).
    fn render_hook<'a>(
        &self,
        batch: &mut Batch,
        client: &Client,
        screen: &[f32; 4],
        p: &Rendered,
        by_id: &dyn Fn(i32) -> Option<&'a Rendered>,
    ) {
        let (prev, cur) = (&p.prev, &p.cur);
        if cur.hook_state <= 0 {
            return;
        }
        // don't render hooks to not active character cores
        let hooked = by_id(cur.hooked_player);
        if cur.hooked_player != -1 && hooked.is_none() {
            return;
        }
        let intra = client.intra_tick;
        let alpha = 1.0; // same team; C++ cl_show_others_alpha for other teams
        let mut info = p.info;
        info.size = 64.0;
        let pos = p.pos;
        let hook_pos = match hooked {
            Some(h) => h.pos,
            _ => mix(
                vec2(prev.hook_x as f32, prev.hook_y as f32),
                vec2(cur.hook_x as f32, cur.hook_y as f32),
                intra,
            ),
        };
        let [l, t, r, b] = *screen;
        if (pos.x < l && hook_pos.x < l)
            || (pos.x > r && hook_pos.x > r)
            || (pos.y < t && hook_pos.y < t)
            || (pos.y > b && hook_pos.y > b)
        {
            return;
        }
        let d = distance(pos, hook_pos);
        let dir = normalize(pos - hook_pos);
        batch.texture_set(self.game_skin.hook_head);
        batch.set_rotation(angle(dir) + PI);
        batch.set_color([1.0, 1.0, 1.0, alpha]);
        let head = &self.quads.weapon[QUAD_HOOK_HEAD];
        batch.container_quad(head, hook_pos.x, hook_pos.y, 1.0, 1.0);
        // C++: RenderQuadContainerAsSpriteMultiple, at most 1024 links
        batch.texture_set(self.game_skin.hook_chain);
        let chain = &self.quads.weapon[QUAD_HOOK_CHAIN];
        let mut f = 24.0;
        let mut count = 0;
        while f < d && count < 1024 {
            let c = hook_pos + dir * f;
            batch.set_rotation(angle(dir) + PI);
            batch.container_quad(chain, c.x, c.y, 1.0, 1.0);
            f += 24.0;
            count += 1;
        }
        batch.set_rotation(0.0);
        batch.set_color([1.0; 4]);
        let quads = &self.quads;
        render_hand(
            batch,
            quads,
            &info,
            pos,
            normalize(hook_pos - pos),
            -PI / 2.0,
            vec2(20.0, 0.0),
            alpha,
        );
    }

    /// C++: `CPlayers::GetPlayerTargetAngle` (players.cpp:97), without the dummy.
    fn target_angle(client: &Client, p: &Rendered, local: Option<i32>) -> f32 {
        if Some(p.id) == local {
            // what would be sent to the server from our current input
            let i = &client.input;
            let mut dir = normalize(vec2(i.target_x as f32, i.target_y as f32));
            if dir == vec2(0.0, 0.0) {
                dir = vec2(1.0, 0.0);
            }
            return angle(dir);
        }
        if let Some(t) = p.target {
            // ponytail: no previous DDNet target until 6.5 interpolates
            return angle(t);
        }
        let intra = client.intra_tick;
        let (pa, ca) = (p.prev.angle as f32, p.cur.angle);
        let m = |a: f32, b: f32| a + (b - a) * intra;
        // the short way round when the weapon passes the top
        if ca as f32 > 256.0 * PI && pa < 0.0 {
            m(pa, ca as f32 - 256.0 * 2.0 * PI) / 256.0
        } else if ca < 0 && pa > 256.0 * PI {
            m(pa, ca as f32 + 256.0 * 2.0 * PI) / 256.0
        } else {
            m(pa, ca as f32) / 256.0
        }
    }

    /// C++: `CPlayers::RenderPlayer` (players.cpp:565).
    fn render_player(
        &self,
        batch: &mut Batch,
        client: &Client,
        collision: &Collision,
        screen: &[f32; 4],
        p: &Rendered,
        local: Option<i32>,
    ) {
        let (prev, player) = (&p.prev, &p.cur);
        let position = p.pos;
        let [l, t, r, b] = *screen;
        // C++: CScreenRect::Inside
        if !(position.x >= l && position.x <= r && position.y >= t && position.y <= b) {
            return;
        }
        let mut info = p.info;
        let alpha = 1.0;
        info.size = 64.0;
        let intra = client.intra_tick;
        let prev_tick = client.prev_game_tick().unwrap_or(0);
        let attack_time =
            (prev_tick - player.attack_tick) as f32 / TICK_SPEED + client.game_tick_time;
        let last_attack_time = attack_time; // not paused
        let attack_ticks_passed = attack_time * TICK_SPEED;

        let angle_ = Self::target_angle(client, p, local);
        let dir = direction(angle_);
        let vel = mix(
            vec2(prev.vel_x as f32 / 256.0, prev.vel_y as f32 / 256.0),
            vec2(player.vel_x as f32 / 256.0, player.vel_y as f32 / 256.0),
            intra,
        );
        info.got_air_jump = player.jumped & 2 == 0;
        info.feet_flipped = false;

        let stationary = player.vel_x <= 1 && player.vel_x >= -1;
        let in_air = !collision.check_point(vec2(player.x as f32, player.y as f32 + 16.0));
        let running = player.vel_x >= 5000 || player.vel_x <= -5000;
        let want_other_dir =
            (player.direction == -1 && vel.x > 0.0) || (player.direction == 1 && vel.x < 0.0);
        let inactive = p.inactive;

        // evaluate animation
        let (walk_time, run_time) = walk_run_times(position.x);
        let mut state = AnimState::default();
        state.set(&ANIMATIONS[ANIM_BASE], 0.0);
        if in_air {
            state.add(&ANIMATIONS[ANIM_INAIR], 0.0, 1.0);
        } else if stationary {
            if inactive {
                let sit = if dir.x < 0.0 {
                    ANIM_SIT_LEFT
                } else {
                    ANIM_SIT_RIGHT
                };
                state.add(&ANIMATIONS[sit], 0.0, 1.0);
                info.feet_flipped = true;
            } else {
                state.add(&ANIMATIONS[ANIM_IDLE], 0.0, 1.0);
            }
        } else if !want_other_dir {
            if running {
                let run = if player.vel_x < 0 {
                    ANIM_RUN_LEFT
                } else {
                    ANIM_RUN_RIGHT
                };
                state.add(&ANIMATIONS[run], run_time, 1.0);
            } else {
                state.add(&ANIMATIONS[ANIM_WALK], walk_time, 1.0);
            }
        }
        const HAMMER_TIME_SCALE: f32 = 5.0;
        if player.weapon == g::WEAPON_HAMMER {
            let t = (last_attack_time * HAMMER_TIME_SCALE).clamp(0.0, 1.0);
            state.add(&ANIMATIONS[ANIM_HAMMER_SWING], t, 1.0);
        }
        if player.weapon == g::WEAPON_NINJA {
            let t = (last_attack_time * 2.0).clamp(0.0, 1.0);
            state.add(&ANIMATIONS[ANIM_NINJA_SWING], t, 1.0);
        }

        // draw gun
        if player.weapon >= 0 && info.flags & TEE_NO_WEAPON == 0 {
            batch.set_color([1.0, 1.0, 1.0, alpha]);
            let weapon = player.weapon.clamp(0, 5) as usize;
            let spec = &WEAPONS[weapon];
            batch.texture_set(self.game_skin.weapons[weapon]);
            let quad = &self.quads.weapon[weapon * 2 + usize::from(dir.x < 0.0)];
            let is_sit = inactive && !in_air && stationary;
            let attach = &state.attach;
            let mut weapon_pos;
            if player.weapon == g::WEAPON_HAMMER {
                // static position for hammer
                weapon_pos = position + vec2(attach.x, attach.y);
                weapon_pos.y += spec.offsety;
                if dir.x < 0.0 {
                    weapon_pos.x -= spec.offsetx;
                }
                if is_sit {
                    weapon_pos.y += 3.0;
                }
                // if active and attack is under way, bash stuffs
                batch.set_rotation(if !inactive || last_attack_time * HAMMER_TIME_SCALE < 1.0 {
                    if dir.x < 0.0 {
                        -PI / 2.0 - attach.angle * PI * 2.0
                    } else {
                        -PI / 2.0 + attach.angle * PI * 2.0
                    }
                } else if dir.x < 0.0 {
                    100.0
                } else {
                    500.0
                });
                batch.container_quad(quad, weapon_pos.x, weapon_pos.y, 1.0, 1.0);
            } else if player.weapon == g::WEAPON_NINJA {
                weapon_pos = position;
                weapon_pos.y += spec.offsety;
                if is_sit {
                    weapon_pos.y += 3.0;
                }
                if dir.x < 0.0 {
                    batch.set_rotation(-PI / 2.0 - attach.angle * PI * 2.0);
                    weapon_pos.x -= spec.offsetx;
                } else {
                    batch.set_rotation(-PI / 2.0 + attach.angle * PI * 2.0);
                }
                batch.container_quad(quad, weapon_pos.x, weapon_pos.y, 1.0, 1.0);
            } else {
                // TODO in C++: should be an animation
                let mut recoil = 0.0;
                let a = attack_ticks_passed / 5.0;
                if a < 1.0 {
                    recoil = (a * PI).sin();
                }
                weapon_pos = position + dir * spec.offsetx - dir * recoil * 10.0;
                weapon_pos.y += spec.offsety;
                if is_sit {
                    weapon_pos.y += 3.0;
                }
                // C++ cl_old_gun_position 0
                batch.set_rotation(attach.angle * PI * 2.0 + angle_);
                batch.container_quad(quad, weapon_pos.x, weapon_pos.y, 1.0, 1.0);
            }
            batch.set_color([1.0; 4]);
            batch.set_rotation(0.0);
            let hand = |batch: &mut Batch, offset: f32, post: Vec2| {
                render_hand(
                    batch,
                    &self.quads,
                    &info,
                    weapon_pos,
                    dir,
                    offset,
                    post,
                    alpha,
                )
            };
            match player.weapon {
                g::WEAPON_GUN => hand(batch, -3.0 * PI / 4.0, vec2(-15.0, 4.0)),
                g::WEAPON_SHOTGUN => hand(batch, -PI / 2.0, vec2(-5.0, 4.0)),
                g::WEAPON_GRENADE => hand(batch, -PI / 2.0, vec2(-4.0, 7.0)),
                _ => {}
            }
        }

        render_tee(
            batch,
            &self.quads,
            &state,
            &info,
            player.emote,
            dir,
            position,
            alpha,
        );

        // C++: the chat bubble, then the afk emote (cl_afk_emote 1, no dummy)
        let chatting = player.player_flags & g::PLAYERFLAG_CHATTING != 0 && !p.afk;
        for (show, emoticon) in [
            (chatting, SPRITE_DOTDOT - SPRITE_OOP),
            (p.afk, SPRITE_ZZZ - SPRITE_OOP),
        ] {
            if show {
                batch.texture_set(self.game_skin.emoticons[emoticon]);
                batch.set_color([1.0, 1.0, 1.0, alpha]);
                let q = &self.quads.weapon[QUAD_EMOTICONS + emoticon];
                batch.container_quad(q, position.x + 24.0, position.y - 40.0, 1.0, 1.0);
                batch.set_color([1.0; 4]);
                batch.set_rotation(0.0);
            }
        }
    }
}
