//! The race status bar's look (7.7 review): one filled icon per DDRace state and three
//! layout directions to choose from. The icons share one grammar: a filled base (a
//! Phosphor glyph, or the weapon's silhouette cut from the game skin) and at most one
//! modifier: a cut slash for "off", or a badge: infinity (endless), sparkle (teleport),
//! lock (deep). DDNet's HUD icons are only the reference for what each one means.

use egui::{Color32, CornerRadius, Margin, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, vec2};

use crate::icons;
use crate::statusbar::{Indicator, Status, WeaponIcons};
use crate::style::{Glass, Weight, color, icon_font, radius, size, space};
use crate::widgets as w;

/// What an icon is drawn from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Base {
    Glyph(&'static str),
    /// A weapon's silhouette (hammer 0 .. ninja 5).
    Weapon(usize),
}

/// The one modifier an icon may carry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mark {
    None,
    /// Off: a slash cut through the base.
    Slash,
    /// A small round badge with a glyph, bottom right.
    Badge(&'static str),
    /// Endless: an "∞" badge (Inter's own sign reads at this size, the glyph does not).
    Endless,
    /// Deep: a heavy ring around the base.
    Ring,
}

/// A state's icon and its name (the HUD editor's tooltip, the gallery's label).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StateIcon {
    pub name: &'static str,
    pub base: Base,
    pub mark: Mark,
}

const fn icon(name: &'static str, base: Base, mark: Mark) -> StateIcon {
    StateIcon { name, base, mark }
}

/// Frozen (not a status bar indicator in C++: its freeze bar is above the tee).
pub const FREEZE: StateIcon = icon("Frozen", Base::Glyph(icons::SNOWFLAKE), Mark::None);
/// Holding ninja (the weapon row shows it; listed for the sheet).
pub const NINJA: StateIcon = icon("Ninja", Base::Weapon(NINJA_ID), Mark::None);

/// C++ `WEAPON_NINJA`.
const NINJA_ID: usize = 5;

/// The icon of a status bar indicator.
pub fn state_icon(ind: Indicator) -> StateIcon {
    use Base::*;
    use Indicator::*;
    use Mark::*;
    let off = Slash;
    let tele = Badge(icons::SPARKLE);
    match ind {
        EndlessJump => icon("Endless jump", Glyph(icons::ARROW_FAT_LINES_UP), Endless),
        EndlessHook => icon("Endless hook", Glyph(icons::LINK), Endless),
        Jetpack => icon("Jetpack", Glyph(icons::ROCKET), None),
        TeleGun => icon("Teleport gun", Weapon(1), tele),
        TeleGrenade => icon("Teleport grenade", Weapon(3), tele),
        TeleLaser => icon("Teleport laser", Weapon(4), tele),
        Solo => icon("Solo", Glyph(icons::PERSON), None),
        CollisionDisabled => icon("Collision off", Glyph(icons::GHOST), None),
        HookHitDisabled => icon("Hooking others off", Glyph(icons::LINK), off),
        HammerHitDisabled => icon("Hammer hits off", Glyph(icons::HAMMER), off),
        GunHitDisabled => icon("Gun hits off", Weapon(1), off),
        ShotgunHitDisabled => icon("Shotgun hits off", Weapon(2), off),
        GrenadeHitDisabled => icon("Grenade hits off", Weapon(3), off),
        LaserHitDisabled => icon("Laser hits off", Weapon(4), off),
        LockMode => icon("Lock mode", Glyph(icons::LOCK), None),
        PracticeMode => icon("Practice", Glyph(icons::BARBELL), None),
        Team0Mode => icon("Team 0 mode", Glyph(icons::USERS), None),
        DeepFrozen => icon("Deep freeze", Glyph(icons::SNOWFLAKE), Ring),
        LiveFrozen => icon("Live freeze", Glyph(icons::PERSON), Badge(icons::SNOWFLAKE)),
    }
}

/// Every icon, for the gallery's sheet.
pub fn sheet() -> Vec<StateIcon> {
    use Indicator::*;
    let mut all = vec![FREEZE];
    all.extend(
        [
            DeepFrozen,
            LiveFrozen,
            EndlessJump,
            EndlessHook,
            Jetpack,
            Solo,
            CollisionDisabled,
            HookHitDisabled,
            HammerHitDisabled,
            GunHitDisabled,
            ShotgunHitDisabled,
            GrenadeHitDisabled,
            LaserHitDisabled,
            TeleGun,
            TeleGrenade,
            TeleLaser,
            LockMode,
            PracticeMode,
            Team0Mode,
        ]
        .map(state_icon),
    );
    all.push(NINJA);
    all
}

/// The weapon names, for tooltips.
const WEAPONS: [&str; 6] = ["Hammer", "Gun", "Shotgun", "Grenade", "Laser", "Ninja"];

/// Draws a base (glyph or silhouette) filling `rect`.
fn draw_base(ui: &Ui, icons: &mut WeaponIcons, base: Base, rect: Rect, tint: Color32) {
    match base {
        Base::Glyph(g) => {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                g,
                // Phosphor's ink is about 80% of the em: fill the square
                icon_font(rect.height() * 1.2),
                tint,
            );
        }
        Base::Weapon(n) => {
            if let Some((id, [tw, th])) = icons.silhouette(ui.ctx(), n) {
                // fit the sprite's aspect into the square
                let scale = (rect.width() / tw).min(rect.height() / th);
                let r = Rect::from_center_size(rect.center(), vec2(tw, th) * scale);
                let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                ui.painter().image(id, r, uv, tint);
            }
        }
    }
}

/// One state icon, `size` square; its name as a tooltip when `tooltip` (the HUD editor).
pub fn draw_icon(
    ui: &mut Ui,
    icons: &mut WeaponIcons,
    icon: StateIcon,
    size: f32,
    tooltip: bool,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    // a ringed base sits inside its ring
    let base = if icon.mark == Mark::Ring {
        rect.shrink(size * 0.22)
    } else {
        rect
    };
    draw_base(ui, icons, icon.base, base, color::TEXT);
    let p = ui.painter();
    match icon.mark {
        Mark::None => {}
        Mark::Slash => {
            let (a, b) = (
                rect.left_top() + vec2(size, size) * 0.12,
                rect.right_bottom() - vec2(size, size) * 0.12,
            );
            // a dark cut, then the white slash in it
            p.line_segment([a, b], Stroke::new(size * 0.24, color::SURFACE));
            p.line_segment([a, b], Stroke::new(size * 0.1, color::TEXT));
        }
        Mark::Endless => {
            let c = rect.right_bottom() - vec2(size, size) * 0.2;
            p.circle_filled(c, size * 0.34, color::SURFACE);
            p.circle_filled(c, size * 0.28, color::TEXT);
            let f = crate::style::font(size * 0.5, Weight::SemiBold);
            p.text(
                c,
                egui::Align2::CENTER_CENTER,
                "\u{221e}",
                f,
                color::SURFACE,
            );
        }
        Mark::Ring => {
            p.circle_stroke(
                rect.center(),
                size * 0.44,
                Stroke::new(size * 0.1, color::TEXT),
            );
        }
        Mark::Badge(g) => {
            let c = rect.right_bottom() - vec2(size, size) * 0.2;
            p.circle_filled(c, size * 0.34, color::SURFACE);
            p.circle_filled(c, size * 0.28, color::TEXT);
            p.text(
                c,
                egui::Align2::CENTER_CENTER,
                g,
                icon_font(size * 0.38),
                color::SURFACE,
            );
        }
    }
    if tooltip {
        response.on_hover_text(icon.name)
    } else {
        response
    }
}

/// A weapon silhouette at `alpha` (C++: 0.4 for weapons not held), named on hover.
fn weapon(ui: &mut Ui, icons: &mut WeaponIcons, n: usize, size: f32, alpha: f32, tooltip: bool) {
    let (rect, response) = ui.allocate_exact_size(vec2(size * 1.5, size), Sense::hover());
    draw_base(
        ui,
        icons,
        Base::Weapon(n),
        rect,
        color::TEXT.gamma_multiply(alpha),
    );
    if tooltip {
        response.on_hover_text(WEAPONS[n]);
    }
}

/// The effects: capabilities, disabled interactions, modes and freeze, in C++'s order.
fn effects(s: &Status) -> Vec<Indicator> {
    (s.capabilities.iter())
        .chain(&s.prohibited)
        .chain(&s.modes)
        .copied()
        .collect()
}

/// The three directions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// A card of rows: weapons, a segmented jump meter, the effects.
    Block,
    /// A narrow vertical rail: the held weapon large, the jumps as a number, the effects
    /// stacked.
    Rail,
    /// Separate small pills: weapon and jumps, then one round chip per effect; the ninja
    /// timer is a ring.
    Chips,
}

impl Layout {
    pub const ALL: [(Self, &'static str); 3] = [
        (Self::Block, "Block"),
        (Self::Rail, "Rail"),
        (Self::Chips, "Chips"),
    ];
}

/// Faint: used jumps, empty tracks.
fn faint() -> Color32 {
    color::TEXT.gamma_multiply(0.25)
}

/// Jumps as a segmented meter: one rounded segment per jump, the used ones faint.
fn meter(ui: &mut Ui, (available, total): (i32, i32)) {
    let (seg, gap, h) = (space::M, space::XS, space::S);
    let width = total as f32 * (seg + gap) - gap;
    let (rect, _) = ui.allocate_exact_size(vec2(width.max(0.0), size::ICON_HUD), Sense::hover());
    for n in 0..total {
        let x = rect.left() + n as f32 * (seg + gap);
        let r = Rect::from_min_size(Pos2::new(x, rect.center().y - h / 2.0), vec2(seg, h));
        let fill = if n < available { color::TEXT } else { faint() };
        ui.painter()
            .rect_filled(r, CornerRadius::same(radius::PILL), fill);
    }
}

/// The ninja timer as a bar: white on a faint track.
fn bar(ui: &mut Ui, progress: f32, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, space::XS), Sense::hover());
    let round = CornerRadius::same(radius::PILL);
    ui.painter().rect_filled(rect, round, faint());
    let mut fill = rect;
    fill.set_width(rect.width() * progress);
    ui.painter().rect_filled(fill, round, color::TEXT);
}

/// A ring around `rect`: the faint track and `progress` of it from the top, clockwise.
fn ring(ui: &Ui, rect: Rect, progress: f32) {
    let c = rect.center();
    let r = rect.width() / 2.0;
    let stroke = space::XS - 1.0;
    ui.painter()
        .circle_stroke(c, r, Stroke::new(stroke, faint()));
    let n = 48;
    let points: Vec<Pos2> = (0..=((n as f32 * progress) as usize))
        .map(|i| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
            c + vec2(a.cos(), a.sin()) * r
        })
        .collect();
    if points.len() > 1 {
        ui.painter()
            .add(Shape::line(points, Stroke::new(stroke, color::TEXT)));
    }
}

/// Draws the status in `layout`. `tooltip`: icons named on hover (the HUD editor).
pub fn draw(
    ui: &mut Ui,
    glass: Glass,
    layout: Layout,
    s: &Status,
    icons: &mut WeaponIcons,
    tooltip: bool,
) {
    let icon_size = size::ICON_HUD;
    let fx = effects(s);
    match layout {
        Layout::Block => {
            let margin = Margin::same(space::M as i8);
            w::hud_bar(ui, glass, radius::CARD, margin, icon_size, |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing = vec2(space::XS, space::XS);
                    ui.spacing_mut().interact_size.y = icon_size;
                    if !s.weapons.is_empty() {
                        ui.horizontal(|ui| {
                            for &(n, active) in &s.weapons {
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = space::XS;
                                    weapon(
                                        ui,
                                        icons,
                                        n,
                                        icon_size,
                                        if active { 1.0 } else { 0.35 },
                                        tooltip,
                                    );
                                    if n == NINJA_ID
                                        && let Some(p) = s.ninja
                                    {
                                        bar(ui, p, icon_size * 1.5);
                                    }
                                });
                            }
                        });
                    }
                    if s.jumps.1 > 0 {
                        ui.horizontal(|ui| meter(ui, s.jumps));
                    }
                    if !fx.is_empty() {
                        ui.horizontal(|ui| {
                            for &ind in &fx {
                                draw_icon(ui, icons, state_icon(ind), icon_size, tooltip);
                            }
                        });
                    }
                });
            });
        }
        Layout::Rail => {
            let margin = Margin::symmetric(space::S as i8, space::M as i8);
            w::hud_bar(ui, glass, radius::CARD, margin, icon_size, |ui| {
                // as narrow as the held weapon
                ui.set_max_width(icon_size * 2.0);
                ui.vertical_centered(|ui| {
                    ui.spacing_mut().item_spacing = vec2(space::XS, space::S);
                    if let Some(&(n, _)) = s.weapons.iter().find(|w| w.1).or(s.weapons.first()) {
                        weapon(ui, icons, n, size::ICON_HUD + space::XS, 1.0, tooltip);
                        if n == NINJA_ID
                            && let Some(p) = s.ninja
                        {
                            bar(ui, p, icon_size * 1.5);
                        }
                    }
                    if s.jumps.1 > 0 {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = space::XS;
                            w::icon(
                                ui,
                                icons::ARROW_FAT_LINES_UP,
                                size::ICON_SMALL,
                                ui.visuals().weak_text_color(),
                            );
                            w::text(
                                ui,
                                &s.jumps.0.to_string(),
                                size::LARGE,
                                Weight::SemiBold,
                                color::TEXT,
                            );
                        });
                    }
                    for &ind in &fx {
                        draw_icon(ui, icons, state_icon(ind), icon_size, tooltip);
                    }
                });
            });
        }
        Layout::Chips => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = space::XS;
                let chip = Margin::symmetric(space::S as i8, space::XS as i8);
                if let Some(&(n, _)) = s.weapons.iter().find(|w| w.1).or(s.weapons.first()) {
                    w::hud_bar(
                        ui,
                        glass,
                        radius::PILL,
                        Margin::symmetric(space::M as i8, space::XS as i8),
                        icon_size,
                        |ui| {
                            ui.spacing_mut().item_spacing.x = space::S;
                            let (rect, _) = ui.allocate_exact_size(
                                vec2(icon_size * 1.5, icon_size),
                                Sense::hover(),
                            );
                            draw_base(
                                ui,
                                icons,
                                Base::Weapon(n),
                                rect.shrink(if n == NINJA_ID && s.ninja.is_some() {
                                    space::XS
                                } else {
                                    0.0
                                }),
                                color::TEXT,
                            );
                            if n == NINJA_ID
                                && let Some(p) = s.ninja
                            {
                                ring(
                                    ui,
                                    Rect::from_center_size(
                                        rect.center(),
                                        vec2(icon_size, icon_size) + vec2(space::XS, space::XS),
                                    ),
                                    p,
                                );
                            }
                            if s.jumps.1 > 0 {
                                let text = format!("{}/{}", s.jumps.0, s.jumps.1);
                                w::text(ui, &text, size::BODY, Weight::Medium, color::TEXT);
                            }
                        },
                    );
                }
                for &ind in &fx {
                    w::hud_bar(ui, glass, radius::PILL, chip, icon_size, |ui| {
                        draw_icon(ui, icons, state_icon(ind), icon_size, tooltip);
                    });
                }
            });
        }
    }
}
