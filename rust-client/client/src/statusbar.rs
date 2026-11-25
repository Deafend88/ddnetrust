//! The DDRace status bar (PLAN §10.3, 7.7; replaces `CHud::RenderPlayerState`,
//! hud.cpp:842): jumps left, weapons owned and active, the ninja timer, capabilities,
//! disabled interactions, modes and freeze, in one HUD bar. `status` is the C++ decision
//! logic, ported line by line; the drawing is ours.

use ddnet::physics::core::{CharacterCore, PHYSICAL_SIZE, read_ddnet};
use ddnet::physics::vmath::vec2;
use ddnet::protocol::generated::{self as g, ObjCharacter, ObjDDNetCharacter};
use egui::{Color32, ColorImage, CornerRadius, Margin, Sense, TextureHandle, Ui};

use crate::assets;
use crate::hud::{Anchor, HudConfig, HudData, Placement, Widget, item_toggles};
use crate::icons;
use crate::image::Format;
use crate::render::content::{SPRITES, SPRITESETS};
use crate::skin::sprite_image;
use crate::storage::Storage;
use crate::style::{color, radius, size, space};
use crate::widgets as w;

/// C++ `NUM_WEAPONS` and the weapon ids.
const NUM_WEAPONS: usize = 6;
const WEAPON_HAMMER: usize = 0;
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

/// The weapon an indicator is about, if any (drawn with its sprite).
fn weapon_of(ind: Indicator) -> Option<usize> {
    use Indicator::*;
    Some(match ind {
        TeleGun | GunHitDisabled => WEAPON_GUN,
        TeleGrenade | GrenadeHitDisabled => WEAPON_GRENADE,
        TeleLaser | LaserHitDisabled => WEAPON_LASER,
        ShotgunHitDisabled => WEAPON_SHOTGUN,
        HammerHitDisabled => WEAPON_HAMMER,
        _ => return None,
    })
}

/// An indicator's icon.
fn icon_of(ind: Indicator) -> &'static str {
    use Indicator::*;
    match ind {
        EndlessJump => icons::ARROW_FAT_LINES_UP,
        EndlessHook => icons::LINK,
        Jetpack => icons::ROCKET,
        TeleGun | TeleGrenade | TeleLaser => icons::SPARKLE,
        Solo => icons::PERSON,
        CollisionDisabled => icons::GHOST,
        HookHitDisabled => icons::LINK_BREAK,
        HammerHitDisabled | GunHitDisabled | ShotgunHitDisabled | GrenadeHitDisabled
        | LaserHitDisabled => icons::PROHIBIT,
        LockMode => icons::LOCK,
        PracticeMode => icons::BARBELL,
        Team0Mode => icons::USERS,
        DeepFrozen => icons::SNOWFLAKE,
        LiveFrozen => icons::PAUSE_CIRCLE,
    }
}

/// The game skin's pickup sprites of the six weapons (C++
/// `m_GameSkin.m_aSpritePickupWeapons`), cut from `game.png` in the data dir.
fn weapon_images(storage: &Storage) -> Vec<Option<ColorImage>> {
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
    let asset = assets::load_asset(storage, assets::GAME, "default");
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

/// The six weapons' pickup sprites as egui images (the status bar, the killfeed).
pub struct WeaponIcons {
    images: Vec<Option<ColorImage>>,
    textures: Vec<Option<TextureHandle>>,
}

impl WeaponIcons {
    pub fn new(storage: &Storage) -> Self {
        Self {
            images: weapon_images(storage),
            textures: Vec::new(),
        }
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
    pub fn new(storage: &Storage) -> Self {
        Self(WeaponIcons::new(storage))
    }
}

/// Jumps as pips: available ones white, used ones faint.
fn pips(ui: &mut Ui, (available, total): (i32, i32)) {
    let r = 5.0;
    let width = total as f32 * (2.0 * r + space::XS) - space::XS;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width.max(0.0), ICON), Sense::hover());
    for n in 0..total {
        let x = rect.left() + r + n as f32 * (2.0 * r + space::XS);
        let fill = if n < available {
            color::TEXT
        } else {
            color::TEXT.gamma_multiply(0.25)
        };
        ui.painter()
            .circle_filled(egui::pos2(x, rect.center().y), r, fill);
    }
}

/// The ninja timer: a short bar, white on a faint track.
fn timer(ui: &mut Ui, progress: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(space::XL, ICON), Sense::hover());
    let track = egui::Rect::from_center_size(rect.center(), egui::vec2(space::XL, space::XS));
    let p = ui.painter();
    let round = CornerRadius::same(radius::PILL);
    p.rect_filled(track, round, color::TEXT.gamma_multiply(0.25));
    let mut fill = track;
    fill.set_width(track.width() * progress);
    p.rect_filled(fill, round, color::TEXT);
}

/// The block's icon and sprite size.
const ICON: f32 = size::ICON_HUD;

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
        // under the watermark (16 + its 57 pt + 15)
        Placement {
            anchor: Anchor::TopLeft,
            x: 16,
            y: 88,
            enabled: true,
        }
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
        // rows: weapons, jumps, active effects (only when something is on); an item
        // that is off is not drawn, every item off hides the widget
        let on = |key| config.on("status", key, true);
        let effects: Vec<Indicator> = (s.capabilities.iter())
            .chain(&s.prohibited)
            .chain(&s.modes)
            .copied()
            .collect();
        let rows = [
            on("weapons") && !s.weapons.is_empty(),
            on("jumps") && s.jumps.1 > 0,
            on("effects") && !effects.is_empty(),
        ];
        if !rows.contains(&true) {
            if data.editing {
                w::placeholder(ui, data.glass, self.icon(), self.name());
            }
            return;
        }
        // several rows: a rounded rectangle (CLAUDE.md shape rule), tight spacing
        let n = rows.iter().filter(|&&r| r).count();
        let corner = if n > 1 { radius::CARD } else { radius::PILL };
        let margin = Margin::same(space::M as i8);
        w::hud_bar(ui, data.glass, corner, margin, ICON, |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(space::XS, space::XS);
                ui.spacing_mut().interact_size.y = ICON;
                if rows[0] {
                    ui.horizontal(|ui| {
                        for &(weapon, active) in &s.weapons {
                            // C++: 0.4 for weapons not held
                            self.0
                                .draw(ui, weapon, ICON, if active { 1.0 } else { 0.4 });
                            if weapon == WEAPON_NINJA
                                && let Some(p) = s.ninja
                            {
                                timer(ui, p);
                            }
                        }
                    });
                }
                if rows[1] {
                    ui.horizontal(|ui| pips(ui, s.jumps));
                }
                if rows[2] {
                    ui.horizontal(|ui| {
                        for &ind in &effects {
                            w::icon(ui, icon_of(ind), ICON, color::TEXT);
                            if let Some(weapon) = weapon_of(ind) {
                                self.0.draw(ui, weapon, ICON, 1.0);
                            }
                        }
                    });
                }
            });
        });
    }

    fn settings(&mut self, ui: &mut Ui, config: &mut HudConfig) {
        // jumps: C++ cl_showhud_jumps_indicator (imported)
        let items = [
            ("weapons", "Weapons", true),
            ("jumps", "Jumps", true),
            ("effects", "Active effects", true),
        ];
        item_toggles(ui, config, "status", &items);
    }
}
