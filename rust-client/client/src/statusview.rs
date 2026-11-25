//! The race status bar's look (7.7, the user's pick: chips). Rule: a state that changes
//! something already shown changes that element instead of adding an icon. So the
//! weapon pill shows every owned weapon as a bold silhouette (held: full, others dim)
//! with its marks: struck through (hits off), a sparkle (teleport), a flame on the gun
//! (jetpack), the timer ring around the sword (ninja); the jump counter in the same pill
//! reads "∞" with endless jump; freeze tints the whole pill. What is left (endless hook,
//! hooking others off, solo, collision off, the modes) are plain bold glyphs, one small
//! chip each, without icon backgrounds. DDNet's HUD icons are the reference for meaning.

use egui::{Color32, CornerRadius, Margin, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, vec2};

use crate::icons;
use crate::statusbar::{Indicator, Status};
use crate::style::{Glass, Weight, color, icon_font, radius, size, space};
use crate::widgets as w;

/// C++ weapon ids.
const GUN: usize = 1;
const NINJA: usize = 5;
const WEAPONS: [&str; 6] = ["Hammer", "Gun", "Shotgun", "Grenade", "Laser", "Ninja"];

/// The icon size (the pill is ~25 % larger than the other HUD rows, user review).
const ICON: f32 = size::ICON_STATUS;

/// How the tee is frozen; it tints the weapon pill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freeze {
    None,
    /// Frozen for a while (a freeze tile).
    Frozen,
    /// Deep freeze: frozen until undeep (C++ `m_DeepFrozen`).
    Deep,
    /// Live freeze: movement off, the tee not frozen (C++ `m_LiveFrozen`).
    Live,
}

impl Freeze {
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Frozen => "Frozen",
            Self::Deep => "Deep freeze",
            Self::Live => "Live freeze",
        }
    }
}

/// The marks one weapon carries.
#[derive(Clone, Copy, Debug, Default)]
struct Marks {
    hits_off: bool,
    teleport: bool,
    jetpack: bool,
}

/// What an indicator does to a weapon.
#[derive(Clone, Copy, Debug)]
enum Mark {
    HitsOff,
    Teleport,
    Jetpack,
}

/// The weapon an indicator changes, and how.
fn weapon_mark(ind: Indicator) -> Option<(usize, Mark)> {
    use Indicator::*;
    use Mark::{HitsOff, Teleport};
    Some(match ind {
        HammerHitDisabled => (0, HitsOff),
        GunHitDisabled => (GUN, HitsOff),
        ShotgunHitDisabled => (2, HitsOff),
        GrenadeHitDisabled => (3, HitsOff),
        LaserHitDisabled => (4, HitsOff),
        TeleGun => (GUN, Teleport),
        TeleGrenade => (3, Teleport),
        TeleLaser => (4, Teleport),
        Indicator::Jetpack => (GUN, Mark::Jetpack),
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug)]
enum Chip {
    Glyph(&'static str),
    Hook { endless: bool, off: bool },
}

/// The indicators that keep a chip of their own: the glyph (or the drawn hook) and name.
fn chip_of(ind: Indicator) -> Option<(Chip, &'static str)> {
    use Indicator::*;
    Some(match ind {
        EndlessHook => (
            Chip::Hook {
                endless: true,
                off: false,
            },
            "Endless hook",
        ),
        HookHitDisabled => (
            Chip::Hook {
                endless: false,
                off: true,
            },
            "Hooking others off",
        ),
        Solo => (Chip::Glyph(icons::PERSON), "Solo"),
        CollisionDisabled => (Chip::Glyph(icons::GHOST), "Collision off"),
        LockMode => (Chip::Glyph(icons::LOCK), "Lock mode"),
        PracticeMode => (Chip::Glyph(icons::BARBELL), "Practice"),
        Team0Mode => (Chip::Glyph(icons::USERS), "Team 0 mode"),
        _ => return None,
    })
}

/// Faint: tracks.
fn faint() -> Color32 {
    color::TEXT.gamma_multiply(0.25)
}

/// The "off" strike: a dark cut with a white line in it, corner to corner.
fn strike(ui: &Ui, rect: Rect) {
    let d = rect.height() * 0.12;
    let (a, b) = (
        rect.left_top() + vec2(d, d),
        rect.right_bottom() - vec2(d, d),
    );
    let p = ui.painter();
    p.line_segment([a, b], Stroke::new(rect.height() * 0.24, color::SURFACE));
    p.line_segment([a, b], Stroke::new(rect.height() * 0.1, color::TEXT));
}

/// A small mark glyph at a corner, outlined dark so it reads over the silhouette (no
/// background).
fn corner_glyph(ui: &Ui, at: Pos2, glyph: &str) {
    let f = icon_font(ICON * 0.6);
    for d in [
        vec2(-1.0, 0.0),
        vec2(1.0, 0.0),
        vec2(0.0, -1.0),
        vec2(0.0, 1.0),
    ] {
        let a = egui::Align2::CENTER_CENTER;
        ui.painter()
            .text(at + d, a, glyph, f.clone(), color::SURFACE);
    }
    ui.painter()
        .text(at, egui::Align2::CENTER_CENTER, glyph, f, color::TEXT);
}

/// A ring around `rect`: the faint track and `progress` of it from the top, clockwise.
fn ring(ui: &Ui, rect: Rect, progress: f32) {
    let (c, r) = (rect.center(), rect.width() / 2.0);
    let stroke = 3.0;
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

/// A bold hook: eye, shaft and the curve with its point, drawn (Phosphor has none).
fn hook(ui: &Ui, rect: Rect) {
    let (w, h) = (rect.width(), rect.height());
    let s = Stroke::new(h * 0.14, color::TEXT);
    let x = rect.left() + w * 0.58;
    let p = ui.painter();
    p.circle_stroke(Pos2::new(x, rect.top() + h * 0.14), h * 0.09, s);
    let shaft = [
        Pos2::new(x, rect.top() + h * 0.23),
        Pos2::new(x, rect.top() + h * 0.62),
    ];
    p.line_segment(shaft, s);
    // the curve: half a circle from the shaft's foot round to the left and up
    let c = Pos2::new(x - w * 0.2, rect.top() + h * 0.62);
    let r = w * 0.2;
    let points: Vec<Pos2> = (0..=16)
        .map(|i| {
            let a = i as f32 / 16.0 * std::f32::consts::PI;
            c + vec2(a.cos(), a.sin()) * r
        })
        .collect();
    p.add(Shape::line(points, s));
    // the point
    let tip = c + vec2(-r, 0.0);
    p.line_segment([tip, tip + vec2(w * 0.12, -h * 0.14)], s);
}

/// One weapon as a clean filled shape in `rect` (1.5 : 1, pointing right), its parts
/// not overlapping so a flat colour stays flat. Drawn, not cut from the game skin: the
/// pickup sprites turn into blobs as silhouettes.
fn weapon_shape(ui: &Ui, n: usize, rect: Rect, fill: Color32) {
    let p = ui.painter();
    let h = rect.height();
    // a point in the box's units: x 0..1.5, y 0..1 (of the height)
    let at = |x: f32, y: f32| rect.left_top() + vec2(x * h, y * h);
    let block = |x0: f32, y0: f32, x1: f32, y1: f32, r: f32| {
        let rr = Rect::from_min_max(at(x0, y0), at(x1, y1));
        p.rect_filled(rr, CornerRadius::same((r * h) as u8), fill);
    };
    let poly = |pts: &[(f32, f32)]| {
        let pts = pts.iter().map(|&(x, y)| at(x, y)).collect();
        p.add(Shape::convex_polygon(pts, fill, Stroke::NONE));
    };
    match n {
        // hammer: a long handle and a tall head across its end
        0 => {
            block(0.08, 0.47, 0.98, 0.61, 0.05);
            block(1.0, 0.1, 1.38, 0.92, 0.08);
        }
        // gun: slide and a slanted grip
        1 => {
            block(0.22, 0.24, 1.36, 0.48, 0.06);
            poly(&[(0.3, 0.5), (0.66, 0.5), (0.56, 0.9), (0.2, 0.9)]);
        }
        // shotgun: a long thin barrel, the pump under it, a stock
        2 => {
            block(0.52, 0.3, 1.46, 0.44, 0.04);
            block(0.86, 0.47, 1.24, 0.62, 0.06);
            poly(&[
                (0.04, 0.32),
                (0.5, 0.32),
                (0.5, 0.5),
                (0.36, 0.56),
                (0.04, 0.78),
            ]);
        }
        // grenade launcher: a fat tube with a wide muzzle, grip and stock
        3 => {
            block(0.42, 0.24, 1.26, 0.6, 0.12);
            block(1.28, 0.16, 1.44, 0.68, 0.05);
            poly(&[(0.56, 0.62), (0.8, 0.62), (0.72, 0.92), (0.5, 0.92)]);
            block(0.04, 0.34, 0.4, 0.52, 0.06);
        }
        // laser: a body, a thin emitter with a lens, two fins, grip
        4 => {
            block(0.3, 0.3, 1.1, 0.56, 0.08);
            block(1.12, 0.38, 1.36, 0.48, 0.03);
            p.circle_filled(at(1.4, 0.43), 0.07 * h, fill);
            block(0.48, 0.14, 0.58, 0.28, 0.02);
            block(0.72, 0.14, 0.82, 0.28, 0.02);
            poly(&[(0.4, 0.58), (0.64, 0.58), (0.56, 0.9), (0.32, 0.9)]);
            block(0.04, 0.36, 0.28, 0.5, 0.05);
        }
        // ninja: handle, guard, a long blade with a point
        _ => {
            block(0.04, 0.43, 0.36, 0.55, 0.05);
            block(0.38, 0.26, 0.46, 0.72, 0.03);
            poly(&[
                (0.48, 0.42),
                (1.3, 0.42),
                (1.46, 0.49),
                (1.3, 0.56),
                (0.48, 0.56),
            ]);
        }
    }
}

/// One owned weapon in the pill. `tooltip`: its name and marks on hover.
fn weapon(
    ui: &mut Ui,
    n: usize,
    held: bool,
    marks: Marks,
    ninja: Option<f32>,
    tooltip: bool,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ICON * 1.5, ICON), Sense::hover());
    let ringed = n == NINJA && ninja.is_some();
    let area = if ringed {
        rect.shrink(space::XS + 1.0)
    } else {
        rect
    };
    // C++ draws weapons not held at 0.4: the same shape in a flat 45 % grey
    let fill = if held { color::TEXT } else { color::WEAPON_DIM };
    let r = Rect::from_center_size(area.center(), vec2(area.height() * 1.5, area.height()));
    weapon_shape(ui, n, r.intersect(area), fill);
    if let Some(p) = ninja.filter(|_| ringed) {
        let d = ICON + space::S;
        ring(ui, Rect::from_center_size(rect.center(), vec2(d, d)), p);
    }
    if marks.hits_off {
        strike(ui, rect);
    }
    if marks.teleport {
        corner_glyph(ui, rect.right_top() + vec2(-2.0, 3.0), icons::SPARKLE);
    }
    if marks.jetpack {
        // behind the gun, where the push comes from
        corner_glyph(ui, rect.left_bottom() + vec2(3.0, -4.0), icons::FLAME);
    }
    if !tooltip {
        return response;
    }
    let mut text = WEAPONS[n].to_string();
    for (on, what) in [
        (marks.hits_off, "hits off"),
        (marks.teleport, "teleport"),
        (marks.jetpack, "jetpack"),
    ] {
        if on {
            text += &format!(", {what}");
        }
    }
    response.on_hover_text(text)
}

/// Draws the status as chips. `freeze` tints the weapon pill; `tooltip`: everything is
/// named on hover (the HUD editor).
pub fn draw(ui: &mut Ui, glass: Glass, s: &Status, freeze: Freeze, tooltip: bool) {
    let all: Vec<Indicator> = (s.capabilities.iter())
        .chain(&s.prohibited)
        .chain(&s.modes)
        .copied()
        .collect();
    let mut marks = [Marks::default(); 6];
    for ind in &all {
        if let Some((n, mark)) = weapon_mark(*ind) {
            let m = &mut marks[n];
            match mark {
                Mark::HitsOff => m.hits_off = true,
                Mark::Teleport => m.teleport = true,
                Mark::Jetpack => m.jetpack = true,
            }
        }
    }
    let endless_jump = all.contains(&Indicator::EndlessJump);
    let chips: Vec<(Chip, &str)> = all.iter().filter_map(|&i| chip_of(i)).collect();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::XS;
        let show_pill = !s.weapons.is_empty() || s.jumps.1 > 0 || freeze != Freeze::None;
        if show_pill {
            let margin = Margin::symmetric(space::L as i8, space::S as i8);
            let pill = w::hud_bar(ui, glass, radius::PILL, margin, ICON, |ui| {
                ui.spacing_mut().item_spacing.x = space::S;
                if freeze != Freeze::None {
                    let glyph = if freeze == Freeze::Live {
                        icons::PERSON
                    } else {
                        icons::SNOWFLAKE
                    };
                    let r = w::icon(ui, glyph, ICON, color::FREEZE);
                    if tooltip {
                        r.on_hover_text(freeze.name());
                    }
                }
                for &(n, held) in &s.weapons {
                    weapon(ui, n, held, marks[n], s.ninja, tooltip);
                }
                if s.jumps.1 > 0 || endless_jump {
                    // endless jump: "∞" instead of the count
                    let text = if endless_jump {
                        "\u{221e}".to_string()
                    } else {
                        format!("{}/{}", s.jumps.0, s.jumps.1)
                    };
                    let r = w::text(ui, &text, size::TITLE, Weight::SemiBold, color::TEXT);
                    if tooltip {
                        r.on_hover_text(if endless_jump {
                            "Endless jump"
                        } else {
                            "Jumps left"
                        });
                    }
                }
            });
            // freeze tints the whole pill; deep freeze also rings it
            let rect = pill.response.rect;
            let round = CornerRadius::same(radius::PILL);
            let (alpha, border) = match freeze {
                Freeze::None => (0.0, false),
                Freeze::Frozen => (0.28, false),
                Freeze::Deep => (0.4, true),
                Freeze::Live => (0.16, false),
            };
            if alpha > 0.0 {
                ui.painter()
                    .rect_filled(rect, round, color::FREEZE.gamma_multiply(alpha));
            }
            if border {
                let stroke = Stroke::new(2.0, color::FREEZE);
                ui.painter()
                    .rect_stroke(rect, round, stroke, egui::StrokeKind::Inside);
            }
        }
        for (chip, name) in chips {
            let margin = Margin::symmetric(space::M as i8, space::S as i8);
            let r = w::hud_bar(ui, glass, radius::PILL, margin, ICON, |ui| {
                ui.spacing_mut().item_spacing.x = space::XS;
                match chip {
                    Chip::Glyph(g) => {
                        w::icon(ui, g, ICON, color::TEXT);
                    }
                    Chip::Hook { endless, off } => {
                        let (rect, _) = ui.allocate_exact_size(vec2(ICON, ICON), Sense::hover());
                        hook(ui, rect);
                        if off {
                            strike(ui, rect);
                        }
                        if endless {
                            w::text(ui, "\u{221e}", size::TITLE, Weight::SemiBold, color::TEXT);
                        }
                    }
                }
            });
            if tooltip {
                r.response.on_hover_text(name);
            }
        }
    });
}

/// The pill's freeze from the indicators and the core's freeze (deep and live first).
pub fn freeze_of(s: &Status, frozen: bool) -> Freeze {
    if s.modes.contains(&Indicator::DeepFrozen) {
        Freeze::Deep
    } else if s.modes.contains(&Indicator::LiveFrozen) {
        Freeze::Live
    } else if frozen {
        Freeze::Frozen
    } else {
        Freeze::None
    }
}

/// Every state, for the gallery page: (label, status, freeze).
pub fn examples() -> Vec<(&'static str, Status, Freeze)> {
    use Indicator::*;
    let base = |weapons: Vec<(usize, bool)>| Status {
        jumps: (2, 2),
        weapons,
        ..Default::default()
    };
    let two = || vec![(0, false), (GUN, true)];
    let all = || vec![(0, false), (GUN, true), (2, false), (3, false), (4, false)];
    let with = |mut s: Status, caps: Vec<Indicator>, off: Vec<Indicator>, modes: Vec<Indicator>| {
        (s.capabilities, s.prohibited, s.modes) = (caps, off, modes);
        s
    };
    let ninja = Status {
        ninja: Some(0.6),
        ..base(vec![(0, false), (GUN, false), (NINJA, true)])
    };
    let off = vec![
        HammerHitDisabled,
        ShotgunHitDisabled,
        GrenadeHitDisabled,
        LaserHitDisabled,
    ];
    let no_jumps = || Status {
        jumps: (0, 2),
        ..base(two())
    };
    vec![
        (
            "Spawn: hammer, gun held, 2 of 2 jumps",
            base(two()),
            Freeze::None,
        ),
        (
            "One jump used",
            Status {
                jumps: (1, 2),
                ..base(two())
            },
            Freeze::None,
        ),
        (
            "All weapons, jetpack (a flame on the gun)",
            with(base(all()), vec![Jetpack], vec![], vec![]),
            Freeze::None,
        ),
        (
            "Endless jump (the counter reads \u{221e})",
            with(base(two()), vec![EndlessJump], vec![], vec![]),
            Freeze::None,
        ),
        (
            "Endless hook; hooking others off",
            with(
                base(two()),
                vec![EndlessHook],
                vec![HookHitDisabled],
                vec![],
            ),
            Freeze::None,
        ),
        ("Ninja held: the timer ring", ninja, Freeze::None),
        (
            "Hits off: hammer, shotgun, grenade, laser (struck through)",
            with(base(all()), vec![], off, vec![]),
            Freeze::None,
        ),
        (
            "Teleport gun, grenade and laser (a sparkle)",
            with(
                base(all()),
                vec![TeleGun, TeleGrenade, TeleLaser],
                vec![],
                vec![],
            ),
            Freeze::None,
        ),
        (
            "Solo; collision off",
            with(base(two()), vec![], vec![Solo, CollisionDisabled], vec![]),
            Freeze::None,
        ),
        (
            "Lock mode; practice; team 0 mode",
            with(
                base(two()),
                vec![],
                vec![],
                vec![LockMode, PracticeMode, Team0Mode],
            ),
            Freeze::None,
        ),
        ("Frozen", no_jumps(), Freeze::Frozen),
        (
            "Deep freeze",
            with(no_jumps(), vec![], vec![], vec![DeepFrozen]),
            Freeze::Deep,
        ),
        (
            "Live freeze",
            with(base(two()), vec![], vec![], vec![LiveFrozen]),
            Freeze::Live,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Merging: weapon states become marks on their weapon, endless jump the counter,
    /// freeze the pill; only the rest keep chips.
    #[test]
    fn states_merge_into_what_is_shown() {
        use Indicator::*;
        let left: Vec<Indicator> = [
            EndlessJump,
            EndlessHook,
            Jetpack,
            TeleGun,
            TeleGrenade,
            TeleLaser,
            Solo,
            CollisionDisabled,
            HookHitDisabled,
            HammerHitDisabled,
            GunHitDisabled,
            ShotgunHitDisabled,
            GrenadeHitDisabled,
            LaserHitDisabled,
            LockMode,
            PracticeMode,
            Team0Mode,
            DeepFrozen,
            LiveFrozen,
        ]
        .into_iter()
        .filter(|&i| weapon_mark(i).is_none() && chip_of(i).is_none())
        .collect();
        // shown by the counter and the pill tint
        assert_eq!(left, [EndlessJump, DeepFrozen, LiveFrozen]);
        let s = Status {
            modes: vec![LiveFrozen, DeepFrozen],
            ..Default::default()
        };
        assert_eq!(freeze_of(&s, false), Freeze::Deep);
        assert_eq!(freeze_of(&Status::default(), true), Freeze::Frozen);
    }
}
