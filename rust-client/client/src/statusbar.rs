//! The DDRace status bar (PLAN §10.3, 7.7; replaces `CHud::RenderPlayerState`,
//! hud.cpp:842): jumps left, weapons owned and active, the ninja timer, capabilities,
//! disabled interactions, modes and freeze, in one HUD bar. `status` is the C++ decision
//! logic, ported line by line; the drawing is ours.

use ddnet::physics::core::{CharacterCore, PHYSICAL_SIZE, read_ddnet};
use ddnet::physics::vmath::vec2;
use ddnet::protocol::generated::{self as g, ObjCharacter, ObjDDNetCharacter};
use egui::{Color32, ColorImage, TextureHandle, Ui};

use crate::assets;
use crate::hud::{Anchor, HudConfig, HudData, Placement, Widget, item_toggles};
use crate::icons;
use crate::image::Format;
use crate::render::content::{SPRITES, SPRITESETS};
use crate::skin::sprite_image;
use crate::statusview;
use crate::storage::Storage;
use crate::widgets as w;

/// C++ `NUM_WEAPONS` and the weapon ids.
const NUM_WEAPONS: usize = 6;
const WEAPON_GUN: usize = 1;
const WEAPON_SHOTGUN: usize = 2;
const WEAPON_GRENADE: usize = 3;
const WEAPON_LASER: usize = 4;
const WEAPON_NINJA: usize = 5;
/// C++: `g_pData->m_Weapons.m_Ninja.m_Duration` (ms) and `GameTickSpeed()`.
const NINJA_DURATION: i32 = 15000;
const TICK_SPEED: i32 = 50;

/// One indicator, in C++'s order within its row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Indicator {
    // capabilities
    EndlessJump,
    EndlessHook,
    Jetpack,
    TeleGun,
    TeleGrenade,
    TeleLaser,
    // prohibited capabilities
    Solo,
    CollisionDisabled,
    HookHitDisabled,
    HammerHitDisabled,
    GunHitDisabled,
    ShotgunHitDisabled,
    GrenadeHitDisabled,
    LaserHitDisabled,
    // modes and freeze
    LockMode,
    PracticeMode,
    Team0Mode,
    DeepFrozen,
    LiveFrozen,
    // ours: the dummy's actions (C++ `CHud::RenderDummyActions`, a box of its own there)
    DummyHammer,
    DummyCopy,
}

/// What `CHud::RenderPlayerState` shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    /// Jumps left and the jumps shown in all (up to 10).
    pub jumps: (i32, i32),
    /// Weapons owned, by id, and whether it is the active one.
    pub weapons: Vec<(usize, bool)>,
    /// The ninja timer (1 = just picked up), when C++ draws its bar.
    pub ninja: Option<f32>,
    pub capabilities: Vec<Indicator>,
    pub prohibited: Vec<Indicator>,
    pub modes: Vec<Indicator>,
}

/// What `RenderPlayerState` reads for a client.
pub struct Input<'a> {
    /// C++ `m_aClients[id].m_Predicted`.
    pub core: &'a CharacterCore,
    /// C++ `m_aClients[id].m_RenderCur`.
    pub player: &'a ObjCharacter,
    /// `Collision()->IsOnGround(player position, PhysicalSize())`.
    pub grounded: bool,
    /// C++ `m_Snap.m_aCharacters[id].m_ExtendedData` (present: the HUD only shows with
    /// `m_HasExtendedData`).
    pub ext: &'a ObjDDNetCharacter,
    /// C++ `m_HasExtendedDisplayInfo`.
    pub display_info: bool,
    /// `Client()->GameTick(g_Config.m_ClDummy)`.
    pub game_tick: i32,
}

/// C++: `CHud::RenderPlayerState` (hud.cpp:842) with `cl_showhud_jumps_indicator 1`:
/// which indicators show, the jump count and the ninja timer. Integer maths wraps like
/// C++ on the (malformed) extremes instead of panicking.
pub fn status(i: &Input) -> Status {
    let (c, p) = (i.core, i.player);
    let mut s = Status::default();
    // C++ absolute(int)
    let abs = |v: i32| v.wrapping_abs();
    if i.display_info {
        let mut used_jumps = c.jumped_total;
        if c.jumps > 1 {
            used_jumps = used_jumps.wrapping_add(!i.grounded as i32);
        } else if c.jumps == 1 {
            // If the player has only one jump, each jump is the last one
            used_jumps = p.jumped & 2;
        } else if c.jumps == -1 {
            // The player has only one ground jump
            used_jumps = !i.grounded as i32;
        }
        if c.endless_jump && used_jumps >= abs(c.jumps) {
            used_jumps = abs(c.jumps).wrapping_sub(1);
        }
        let mut unused_jumps = abs(c.jumps).wrapping_sub(used_jumps);
        if p.jumped & 2 == 0 && unused_jumps <= 0 {
            // In some edge cases when the player just got another number of jumps,
            // UnusedJumps is not correct
            unused_jumps = 1;
        }
        let total = abs(c.jumps).clamp(0, 10);
        s.jumps = (unused_jumps.clamp(0, total), total);
    } else {
        let n = abs(i.ext.jumps).clamp(0, 10);
        s.jumps = (n, n);
    }

    for (weapon, stat) in c.weapons.iter().enumerate().take(NUM_WEAPONS) {
        if stat.got {
            s.weapons.push((weapon, p.weapon == weapon as i32));
        }
    }
    if c.weapons[WEAPON_NINJA].got {
        let max = NINJA_DURATION * TICK_SPEED / 1000;
        let left = (c.ninja.activation_tick)
            .wrapping_add(max)
            .wrapping_sub(i.game_tick);
        let progress = left.clamp(0, max) as f32 / max as f32;
        if progress > 0.0 && i.display_info {
            s.ninja = Some(progress);
        }
    }

    let got = |w: usize| c.weapons[w].got;
    let caps = &mut s.capabilities;
    for (on, ind) in [
        (c.endless_jump, Indicator::EndlessJump),
        (c.endless_hook, Indicator::EndlessHook),
        (c.jetpack, Indicator::Jetpack),
        (c.has_telegun_gun && got(WEAPON_GUN), Indicator::TeleGun),
        (
            c.has_telegun_grenade && got(WEAPON_GRENADE),
            Indicator::TeleGrenade,
        ),
        (
            c.has_telegun_laser && got(WEAPON_LASER),
            Indicator::TeleLaser,
        ),
    ] {
        if on {
            caps.push(ind);
        }
    }
    for (on, ind) in [
        (c.solo, Indicator::Solo),
        (c.collision_disabled, Indicator::CollisionDisabled),
        (c.hook_hit_disabled, Indicator::HookHitDisabled),
        (c.hammer_hit_disabled, Indicator::HammerHitDisabled),
        // C++ quirk (DECISIONS.md): the gun indicator tests the grenade flag and the
        // telegun
        (
            c.grenade_hit_disabled && c.has_telegun_gun && got(WEAPON_GUN),
            Indicator::GunHitDisabled,
        ),
        (
            c.shotgun_hit_disabled && got(WEAPON_SHOTGUN),
            Indicator::ShotgunHitDisabled,
        ),
        (
            c.grenade_hit_disabled && got(WEAPON_GRENADE),
            Indicator::GrenadeHitDisabled,
        ),
        (
            c.laser_hit_disabled && got(WEAPON_LASER),
            Indicator::LaserHitDisabled,
        ),
    ] {
        if on {
            s.prohibited.push(ind);
        }
    }
    let flag = |f: i32| i.display_info && i.ext.flags & f != 0;
    for (on, ind) in [
        (flag(g::CHARACTERFLAG_LOCK_MODE), Indicator::LockMode),
        (
            flag(g::CHARACTERFLAG_PRACTICE_MODE),
            Indicator::PracticeMode,
        ),
        (flag(g::CHARACTERFLAG_TEAM0_MODE), Indicator::Team0Mode),
        (c.deep_frozen, Indicator::DeepFrozen),
        (c.live_frozen, Indicator::LiveFrozen),
    ] {
        if on {
            s.modes.push(ind);
        }
    }
    s
}

/// The game skin's pickup sprites of the six weapons (C++
/// `m_GameSkin.m_aSpritePickupWeapons`), cut from `game.png` in the data dir.
fn weapon_images(storage: &Storage, game: &str) -> Vec<Option<ColorImage>> {
    let names = [
        "pickup_hammer",
        "pickup_gun",
        "pickup_shotgun",
        "pickup_grenade",
        "pickup_laser",
        "pickup_ninja",
    ];
    let Some(set) = SPRITESETS.iter().find(|s| s.name == "game") else {
        return vec![None; NUM_WEAPONS];
    };
    let asset = assets::load_asset(storage, assets::GAME, game);
    let image = asset.image.and_then(|mut i| {
        (i.check_divisibility(set.gridx as u32, set.gridy as u32, true) && i.format == Format::Rgba)
            .then_some(i)
    });
    names
        .iter()
        .map(|name| {
            let image = image.as_ref()?;
            let sprite = SPRITES.iter().find(|s| s.name == *name)?;
            let s = sprite_image(image, asset.fallback.as_ref(), sprite);
            (s.width > 0 && s.height > 0).then(|| {
                ColorImage::from_rgba_unmultiplied(
                    [s.width as usize, s.height as usize],
                    &s.to_rgba(),
                )
            })
        })
        .collect()
}

/// The six weapons' pickup sprites as egui images (the status bar, the killfeed), and
/// their white silhouettes (the status icons).
pub struct WeaponIcons {
    images: Vec<Option<ColorImage>>,
    textures: Vec<Option<TextureHandle>>,
    silhouettes: Vec<Option<TextureHandle>>,
}

impl WeaponIcons {
    /// From the game pack `game` (C++ `cl_asset_game`).
    pub fn new(storage: &Storage, game: &str) -> Self {
        Self {
            images: weapon_images(storage, game),
            textures: Vec::new(),
            silhouettes: Vec::new(),
        }
    }

    /// A weapon's white silhouette (the sprite's alpha) and its size in pixels.
    pub fn silhouette(
        &mut self,
        ctx: &egui::Context,
        weapon: usize,
    ) -> Option<(egui::TextureId, [f32; 2])> {
        if self.silhouettes.is_empty() {
            self.silhouettes = (self.images.iter().enumerate())
                .map(|(n, i)| {
                    let i = i.as_ref()?;
                    // cropped to the opaque part: pickup sprites carry wide margins
                    let [w, h] = i.size;
                    let opaque = |x: usize, y: usize| i.pixels[y * w + x].a() > 0;
                    let xs: Vec<usize> = (0..w).filter(|&x| (0..h).any(|y| opaque(x, y))).collect();
                    let ys: Vec<usize> = (0..h).filter(|&y| (0..w).any(|x| opaque(x, y))).collect();
                    let (&x0, &x1, &y0, &y1) = (xs.first()?, xs.last()?, ys.first()?, ys.last()?);
                    let white: Vec<Color32> = (y0..=y1)
                        .flat_map(|y| (x0..=x1).map(move |x| (x, y)))
                        .map(|(x, y)| Color32::from_white_alpha(i.pixels[y * w + x].a()))
                        .collect();
                    let image = ColorImage::new([x1 - x0 + 1, y1 - y0 + 1], white);
                    Some(ctx.load_texture(
                        format!("hud_weapon_silhouette_{n}"),
                        image,
                        Default::default(),
                    ))
                })
                .collect();
        }
        let t = self.silhouettes.get(weapon)?.as_ref()?;
        Some((t.id(), t.size().map(|v| v as f32)))
    }

    /// A weapon sprite, `height` tall, at `alpha`; nothing for an unknown weapon.
    pub fn draw(&mut self, ui: &mut Ui, weapon: usize, height: f32, alpha: f32) {
        if self.textures.is_empty() {
            let ctx = ui.ctx().clone();
            self.textures = (self.images.iter().enumerate())
                .map(|(n, i)| {
                    let i = i.clone()?;
                    Some(ctx.load_texture(format!("hud_weapon_{n}"), i, Default::default()))
                })
                .collect();
        }
        let Some(Some(t)) = self.textures.get(weapon) else {
            return;
        };
        let [tw, th] = t.size().map(|v| v as f32);
        let h = height;
        let tint = Color32::WHITE.gamma_multiply(alpha);
        ui.add(egui::Image::new((t.id(), egui::vec2(h * tw / th, h))).tint(tint));
    }
}

pub struct StatusBar(WeaponIcons);

impl StatusBar {
    pub fn new(storage: &Storage, game: &str) -> Self {
        Self(WeaponIcons::new(storage, game))
    }
}

impl Widget for StatusBar {
    fn id(&self) -> &'static str {
        "status"
    }

    fn name(&self) -> &'static str {
        "Race status"
    }

    fn icon(&self) -> &'static str {
        icons::ARROW_FAT_LINES_UP
    }

    fn default_placement(&self) -> Placement {
        // under the default watermark (16 + its 49 pt + 8); it follows the watermark's
        // actual height (`below`)
        Placement {
            anchor: Anchor::TopLeft,
            x: 16,
            y: 73,
            enabled: true,
        }
    }

    fn below(&self) -> Option<&'static str> {
        Some("watermark")
    }

    fn show(&mut self, ui: &mut Ui, data: &HudData, config: &HudConfig) {
        // C++: RenderPlayerState runs with m_HasExtendedData and the server's
        // m_HudDDRace (cl_showhud_ddrace: the widget's on/off)
        let (Some(tee), Some(predictor)) = (data.tee, data.predictor) else {
            return;
        };
        let Some(ext) = tee.ext else { return };
        if !predictor.hud_ddrace {
            return;
        }
        let snapped;
        let core = match predictor.predicted_core(tee.id) {
            Some(c) => c,
            None => {
                let mut c = CharacterCore::default();
                read_ddnet(&mut c, &ext);
                snapped = c;
                &snapped
            }
        };
        let player = &tee.cur;
        let pos = vec2(player.x as f32, player.y as f32);
        let s = status(&Input {
            core,
            player,
            grounded: predictor.collision().is_on_ground(pos, PHYSICAL_SIZE),
            ext: &ext,
            display_info: ext.jumped_total != -1,
            game_tick: data.client.game_tick().unwrap_or(0),
        });
        // an item that is off is not drawn; every item off hides the widget
        let on = |key| config.on("status", key, true);
        let mut s = s;
        if !on("weapons") {
            (s.weapons, s.ninja) = (Vec::new(), None);
        }
        if !on("jumps") {
            s.jumps = (0, 0);
        }
        if !on("effects") {
            (s.capabilities, s.prohibited, s.modes) = (Vec::new(), Vec::new(), Vec::new());
        }
        // C++ RenderDummyActions: while the dummy is connected and not at game over; ours
        // shows only what is on. ponytail: it rides on this widget, so it needs the
        // DDRace HUD like the rest of it; a widget of its own if vanilla servers matter
        let c = data.client;
        let game_over = (c.snapshot().iter())
            .find(|it| it.type_id == g::NETOBJTYPE_GAMEINFO)
            .is_some_and(|it| {
                g::ObjGameInfo::from_ints(&it.data).game_state_flags & g::GAMESTATEFLAG_GAMEOVER
                    != 0
            });
        if on("dummy") && c.dummy_connected() && !game_over {
            for (active, ind) in [
                (c.dummy_hammer, Indicator::DummyHammer),
                (c.dummy_copy_moves, Indicator::DummyCopy),
            ] {
                if active {
                    s.modes.push(ind);
                }
            }
        }
        let empty = s.weapons.is_empty()
            && s.jumps.1 == 0
            && s.capabilities.is_empty()
            && s.prohibited.is_empty()
            && s.modes.is_empty();
        if empty {
            if data.editing {
                w::placeholder(ui, data.glass, self.icon(), self.name());
            }
            return;
        }
        // C++: frozen while m_FreezeEnd is ahead (the deep and live flags come first)
        let frozen = core.freeze_end > data.client.game_tick().unwrap_or(0);
        let freeze = statusview::freeze_of(&s, frozen);
        statusview::draw(ui, data.glass, &s, freeze, &mut self.0, data.editing);
    }

    fn settings(&mut self, ui: &mut Ui, config: &mut HudConfig) {
        // jumps: C++ cl_showhud_jumps_indicator (imported)
        let items = [
            ("weapons", "Weapons", true),
            ("jumps", "Jumps", true),
            ("effects", "Active effects", true),
            ("dummy", "Dummy actions", true),
        ];
        item_toggles(ui, config, "status", &items);
    }
}
