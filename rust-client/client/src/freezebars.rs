//! Freeze bars (PLAN §13.2, with the scoreboard): how much of a frozen tee's freeze is
//! left, under the tee. `bar` is C++ `CFreezeBars::RenderFreezeBar`
//! (game/client/components/freezebars.cpp:5) without the drawing, `bars` its `OnRender`
//! walk (others first, then the local tee). The drawing is ours (a pill in the freeze
//! colour, `show`). Checked against copies of the C++ code by `rs_parity scoreboard`
//! (tests/scoreboard.rs).

use ddnet::physics::core::CharacterCore;
use ddnet::physics::vmath::{Vec2, vec2};

/// C++ `FreezeBarWidth`, `FreezeBarHalfWidth`, `FreezeBarHeight` in world units.
pub const WIDTH: f32 = 64.0;
pub const HEIGHT: f32 = 16.0;

/// One bar: top-left corner in the world, the freeze left (0..1] and the alpha.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar {
    pub id: i32,
    pub pos: Vec2,
    pub progress: f32,
    pub alpha: f32,
}

/// A tee as freeze bars read it.
#[derive(Clone, Copy, Debug)]
pub struct Tee<'a> {
    pub id: i32,
    /// C++ `m_aClients[id].m_Predicted`.
    pub core: &'a CharacterCore,
    /// C++ `m_HasExtendedDisplayInfo` and `m_ExtendedData.m_FreezeEnd`.
    pub has_display_info: bool,
    pub ext_freeze_end: i32,
    /// The tick its core is at: `PredGameTick` for the predicted local tee,
    /// `GetPredictionTick` for other predicted tees, else `GameTick`.
    pub tick: i32,
    /// C++ `m_RenderPos`.
    pub pos: Vec2,
    pub other_team: bool,
    /// C++ `IsPlayerInfoAvailable`: an active character with this and the previous
    /// snapshot's player info.
    pub available: bool,
}

/// C++ `cl_freezebars_alpha_inside_freeze` and `cl_show_others_alpha`.
#[derive(Clone, Copy, Debug)]
pub struct BarConfig {
    pub alpha_inside_freeze: i32,
    pub show_others_alpha: i32,
}

/// C++: `CFreezeBars::RenderFreezeBar` (freezebars.cpp:5).
pub fn bar(t: &Tee, cfg: &BarConfig) -> Option<Bar> {
    let c = t.core;
    if c.freeze_end <= 0
        || c.freeze_start == 0
        || c.freeze_end <= c.freeze_start
        || !t.has_display_info
        || (c.is_in_freeze && cfg.alpha_inside_freeze == 0)
    {
        return None;
    }
    // only show the freeze bar once the server confirmed the freeze
    if t.ext_freeze_end <= 0 {
        return None;
    }
    let max = c.freeze_end as i64 - c.freeze_start as i64;
    let left = (max - (t.tick as i64 - c.freeze_start as i64)).clamp(0, max);
    let progress = left as f32 / max as f32;
    if progress <= 0.0 {
        return None;
    }
    let mut alpha = if t.other_team {
        cfg.show_others_alpha as f32 / 100.0
    } else {
        1.0
    };
    if c.is_in_freeze {
        alpha *= cfg.alpha_inside_freeze as f32 / 100.0;
    }
    Some(Bar {
        id: t.id,
        pos: vec2(t.pos.x - WIDTH / 2.0, t.pos.y + 32.0),
        progress,
        alpha,
    })
}

/// C++: `CFreezeBars::OnRender` (freezebars.cpp:214): everyone else's bar, then our own,
/// for tees within 120 units of the screen (`screen`: top left, bottom right).
pub fn bars(tees: &[Tee], local_id: i32, screen: (Vec2, Vec2), cfg: &BarConfig) -> Vec<Bar> {
    let (tl, br) = (screen.0 - vec2(120.0, 120.0), screen.1 + vec2(120.0, 120.0));
    let inside = |p: Vec2| tl.x <= p.x && p.x <= br.x && tl.y <= p.y && p.y <= br.y;
    let others = (tees.iter()).filter(|t| t.id != local_id && t.available && inside(t.pos));
    let local = (tees.iter()).filter(|t| t.id == local_id && local_id != -1 && t.available);
    others.chain(local).filter_map(|t| bar(t, cfg)).collect()
}

/// Draws the freeze bars as pills in the world (in C++'s 64 x 16 unit box under the tee):
/// the part of the freeze still left in the freeze colour on a dark track.
pub fn show(ctx: &egui::Context, bars: &[Bar], view: &crate::nameplates::WorldView) {
    use crate::style::color;
    use egui::{Color32, CornerRadius, Rect};
    let layer = egui::LayerId::new(egui::Order::Background, egui::Id::unique("freezebars"));
    let painter = ctx.layer_painter(layer);
    for b in bars {
        let (tl, br) = (
            view.to_screen(b.pos),
            view.to_screen(b.pos + vec2(WIDTH, HEIGHT)),
        );
        // a slimmer pill centred in C++'s box
        let full = Rect::from_min_max(tl, br).shrink2(egui::vec2(0.0, (br.y - tl.y) * 0.25));
        let r = CornerRadius::same(255);
        painter.rect_filled(full, r, Color32::from_black_alpha((150.0 * b.alpha) as u8));
        let inner = full.shrink(full.height() * 0.18);
        let left = egui::vec2(inner.width() * b.progress, inner.height());
        painter.rect_filled(
            Rect::from_min_size(inner.min, left),
            r,
            color::FREEZE.gamma_multiply(b.alpha),
        );
    }
}
