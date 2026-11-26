//! Name plates (PLAN §13.2, with the scoreboard). `plate` is C++
//! `CNamePlates::RenderNamePlateGame` (game/client/components/nameplates.cpp:625): which
//! parts a tee's plate shows, its colour and alpha, the direction arrows and the
//! strong/weak hook mark; `plates` is `OnRender`'s walk. The drawing is ours (`show`).
//! Checked against copies of the C++ code by `rs_parity scoreboard`
//! (tests/scoreboard.rs).

use ddnet::clients::Clients;
use ddnet::physics::vmath::{Vec2, distance, vec2};

use crate::render::map::hsl_to_rgb;

/// C++ `SPEC_FREEVIEW`.
const SPEC_FREEVIEW: i32 = -1;

/// The name plate settings (C++ `cl_nameplates*`, `cl_show_direction`, ...).
#[derive(Clone, Copy, Debug, Default)]
pub struct PlateConfig {
    pub name_plates: bool,
    pub own: bool,
    pub friend_mark: bool,
    pub ids: bool,
    pub size: i32,
    pub ids_separate_line: bool,
    pub ids_size: i32,
    pub clan: bool,
    pub clan_size: i32,
    /// 0 off, 1 strong/weak, 2 with the ids.
    pub strong: i32,
    pub strong_size: i32,
    pub direction_size: i32,
    pub always: bool,
    pub offset: i32,
    pub show_others_alpha: i32,
    pub teamcolors: bool,
    /// 0 off, 1 others, 2 everyone, 3 only self.
    pub show_direction: i32,
    /// C++ `g_Config.m_Debug`.
    pub debug: bool,
}

/// C++ `EHookStrongWeakState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrongWeak {
    Weak,
    Neutral,
    Strong,
}

/// C++ `CNamePlateData`, plus where it is drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Plate {
    pub id: i32,
    /// The plate's anchor: the tee's position minus `cl_nameplates_offset`.
    pub pos: Vec2,
    pub color: [f32; 4],
    pub show_name: bool,
    pub name: String,
    pub show_friend_mark: bool,
    pub show_client_id: bool,
    pub font_size_client_id: f32,
    pub client_id_separate_line: bool,
    pub font_size: f32,
    pub show_clan: bool,
    pub clan: String,
    pub font_size_clan: f32,
    pub show_direction: bool,
    pub dir_left: bool,
    pub dir_jump: bool,
    pub dir_right: bool,
    pub font_size_direction: f32,
    pub show_hook_strong_weak: bool,
    pub hook_strong_weak: StrongWeak,
    pub show_hook_strong_weak_id: bool,
    pub hook_strong_weak_id: i32,
    pub font_size_hook_strong_weak: f32,
}

/// A character of the snapshot as name plates read it (C++ `m_Snap.m_aCharacters`).
#[derive(Clone, Copy, Debug, Default)]
pub struct CharView {
    /// C++ `m_Active`: in this and the previous snapshot.
    pub active: bool,
    pub direction: i32,
    pub jumped: i32,
    /// C++ `m_HasExtendedData` / `m_ExtendedData.m_StrongWeakId`.
    pub strong_weak_id: Option<i32>,
}

/// What `RenderNamePlateGame` reads.
pub struct Ctx<'a> {
    /// C++ `Graphics()->GetScreen()`: the world rect on screen (top left, bottom right).
    pub screen: (Vec2, Vec2),
    pub clients: &'a Clients,
    pub is_other_team: &'a dyn Fn(i32) -> bool,
    pub team: &'a dyn Fn(i32) -> i32,
    pub team_play: bool,
    /// C++ `m_Controls.m_aTargetPos[cl_dummy]`.
    pub target_pos: Vec2,
    /// C++ `m_aLocalIds` and `cl_dummy`.
    pub local_ids: [i32; 2],
    pub dummy: usize,
    /// C++ `m_Controls.m_aInputData[n]`: (direction, jump).
    pub inputs: [(i32, i32); 2],
    pub chars: &'a [CharView],
    /// C++ `m_Snap.m_SpecInfo.m_Active` and `m_SpectatorId`.
    pub spec: (bool, i32),
    pub config: PlateConfig,
}

/// C++: `GetDDTeamColor(DDTeam, Lightness)` (gameclient.cpp:1008).
pub fn dd_team_color(team: i32, lightness: f32) -> [f32; 3] {
    const GOLDEN: f32 = 137.507_76 / 360.0;
    let hue = ((team - 1) as f32 * GOLDEN) % 1.0;
    hsl_to_rgb([hue, 1.0, lightness])
}

/// C++ `18.0f + 20.0f * Size / 100.0f`.
fn font_size(size: i32) -> f32 {
    18.0 + 20.0 * size as f32 / 100.0
}

/// C++: `CNamePlates::RenderNamePlateGame` (nameplates.cpp:625): the plate of player
/// `id` (local or not) at `pos`, or `None` when it is far off screen.
pub fn plate(pos: Vec2, id: i32, local: bool, mut alpha: f32, c: &Ctx) -> Option<Plate> {
    let cfg = &c.config;
    let (mut tl, mut br) = c.screen;
    // Assume that the name plate fits into a 800x800 box placed directly above the tee
    tl.x -= 400.0;
    br.x += 400.0;
    br.y += 800.0;
    let inside = |v: f32, lo: f32, hi: f32| lo <= v && v <= hi;
    if !(inside(pos.x, tl.x, br.x) && inside(pos.y, tl.y, br.y)) {
        return None;
    }
    let client = &c.clients.clients[id as usize];
    let other_team = (c.is_other_team)(id);
    let show_name = if local { cfg.own } else { cfg.name_plates };
    let separate = cfg.ids_separate_line;
    let font = font_size(cfg.size);

    if !cfg.always {
        alpha *= (1.0 - (distance(c.target_pos, pos) / 200.0).powf(16.0)).clamp(0.0, 1.0);
    }
    if other_team {
        alpha *= cfg.show_others_alpha as f32 / 100.0;
    }
    let mut color = [1.0, 1.0, 1.0];
    if cfg.teamcolors {
        if c.team_play {
            if client.team == 0 {
                color = [1.0, 0.5, 0.5];
            } else if client.team == 1 {
                color = [0.7, 0.7, 1.0];
            }
        } else {
            let team = (c.team)(id);
            if team != 0 {
                color = dd_team_color(team, 0.75);
            }
        }
    }

    let show_direction = match cfg.show_direction {
        1 => !local,
        2 => true,
        3 => local,
        _ => false,
    };
    let (mut left, mut jump, mut right) = (false, false, false);
    if show_direction {
        let dummy_id = c.local_ids[1 - c.dummy.min(1)];
        let input = if id == dummy_id {
            Some(c.inputs[1 - c.dummy.min(1)])
        } else if local {
            Some(c.inputs[c.dummy.min(1)])
        } else {
            None
        };
        match input {
            Some((dir, j)) => (left, jump, right) = (dir == -1, j == 1, dir == 1),
            None => {
                let ch = &c.chars[id as usize];
                (left, jump, right) = (ch.direction == -1, ch.jumped & 1 != 0, ch.direction == 1);
            }
        }
    }

    let (mut show_sw, mut sw_state, mut show_sw_id, mut sw_id) =
        (false, StrongWeak::Neutral, false, 0);
    let following = c.spec.0 && c.spec.1 != SPEC_FREEVIEW;
    if c.clients.local_id != -1 || following {
        let selected = if following {
            c.spec.1
        } else {
            c.clients.local_id
        };
        let sel = &c.chars[selected as usize];
        let spec_char = c.clients.spec_chars[selected as usize].is_some();
        if let (true, Some(other)) = (
            sel.strong_weak_id.is_some() || spec_char,
            c.chars[id as usize].strong_weak_id,
        ) {
            let selected_sw = sel.strong_weak_id.unwrap_or(0);
            sw_id = other;
            show_sw_id = cfg.debug || cfg.strong == 2;
            if selected == id {
                show_sw = show_sw_id;
            } else {
                sw_state = if selected_sw > other {
                    StrongWeak::Strong
                } else {
                    StrongWeak::Weak
                };
                show_sw = cfg.debug || cfg.strong > 0;
            }
        }
    }

    Some(Plate {
        id,
        pos: pos - vec2(0.0, cfg.offset as f32),
        color: [color[0], color[1], color[2], alpha],
        show_name,
        name: client.name.clone(),
        show_friend_mark: show_name && cfg.friend_mark && client.friend,
        show_client_id: show_name && (cfg.debug || cfg.ids),
        font_size_client_id: if separate {
            font_size(cfg.ids_size)
        } else {
            font
        },
        client_id_separate_line: separate,
        font_size: font,
        show_clan: show_name && cfg.clan,
        clan: client.clan.clone(),
        font_size_clan: font_size(cfg.clan_size),
        show_direction,
        dir_left: left,
        dir_jump: jump,
        dir_right: right,
        font_size_direction: font_size(cfg.direction_size),
        show_hook_strong_weak: show_sw,
        hook_strong_weak: sw_state,
        show_hook_strong_weak_id: show_sw_id,
        hook_strong_weak_id: sw_id,
        font_size_hook_strong_weak: font_size(cfg.strong_size),
    })
}

/// C++: `CNamePlates::OnRender` (nameplates.cpp:856): the plates of every player, a
/// spectator char's (40 % alpha) before its tee's. `render_pos(id)`: C++
/// `m_aClients[id].m_RenderPos` of active characters.
pub fn plates(c: &Ctx, render_pos: &dyn Fn(i32) -> Vec2) -> Vec<Plate> {
    let cfg = &c.config;
    if !cfg.name_plates && !cfg.own && cfg.show_direction == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (id, info) in c.clients.infos.iter().enumerate() {
        let Some(info) = info else { continue };
        let id = id as i32;
        if let Some((x, y)) = c.clients.spec_chars[id as usize] {
            out.extend(plate(vec2(x, y), id, info.local, 0.4, c));
        }
        if c.chars[id as usize].active {
            out.extend(plate(render_pos(id), id, info.local, 1.0, c));
        }
    }
    out
}

/// The world rectangle the camera shows (C++ `Graphics()->GetScreen()` for the game
/// layer: `MapScreenToWorld`) and where it is on screen, for drawing world things in egui.
#[derive(Clone, Copy, Debug)]
pub struct WorldView {
    /// Left, top, right, bottom in world units.
    pub world: [f32; 4],
    pub screen: egui::Rect,
}

impl WorldView {
    pub fn to_screen(&self, p: Vec2) -> egui::Pos2 {
        let [l, t, r, b] = self.world;
        let s = self.screen;
        egui::pos2(
            s.left() + (p.x - l) / (r - l) * s.width(),
            s.top() + (p.y - t) / (b - t) * s.height(),
        )
    }

    /// Points per world unit.
    pub fn scale(&self) -> f32 {
        self.screen.height() / (self.world[3] - self.world[1])
    }
}

mod draw {
    use egui::{Color32, Shadow};

    use super::*;
    use crate::style::{Weight, color, font, icon_font};

    /// A part of a plate line: an icon or a text.
    enum Part<'a> {
        Icon(&'static str, Color32),
        Text(&'a str, Weight, Color32),
    }

    /// Draws the name plates (C++ `CNamePlate::Render` order, bottom up: direction keys,
    /// name line, clan, the client id on its own line, the strong/weak hook mark), as text
    /// with a soft shadow; sizes follow the C++ world sizes, so they zoom with the tees.
    pub fn show(ctx: &egui::Context, plates: &[Plate], view: &WorldView) {
        let layer = egui::LayerId::new(egui::Order::Background, egui::Id::unique("nameplates"));
        let painter = ctx.layer_painter(layer);
        let k = view.scale();
        for p in plates {
            let a = p.color[3];
            if a <= 0.0 {
                continue;
            }
            let c = |f: f32| (f.clamp(0.0, 1.0) * 255.0).round() as u8;
            let tint =
                Color32::from_rgb(c(p.color[0]), c(p.color[1]), c(p.color[2])).gamma_multiply(a);
            let text = color::TEXT.gamma_multiply(a);
            let weak = color::TEXT_SECONDARY.gamma_multiply(a);
            let id = p.id.to_string();
            let sw_id = p.hook_strong_weak_id.to_string();
            let mut lines: Vec<(f32, Vec<Part>)> = Vec::new();
            if p.show_direction {
                let keys = [
                    (p.dir_left, crate::icons::DIR_LEFT),
                    (p.dir_jump, crate::icons::DIR_UP),
                    (p.dir_right, crate::icons::DIR_RIGHT),
                ];
                let parts = (keys.iter())
                    .filter(|k| k.0)
                    .map(|k| Part::Icon(k.1, text))
                    .collect();
                lines.push((p.font_size_direction, parts));
            }
            let mut name = Vec::new();
            if p.show_friend_mark {
                name.push(Part::Icon(crate::icons::FRIEND, tint));
            }
            if p.show_client_id && !p.client_id_separate_line {
                name.push(Part::Text(&id, Weight::Medium, weak));
            }
            if p.show_name {
                name.push(Part::Text(&p.name, Weight::Medium, tint));
            }
            lines.push((p.font_size, name));
            if p.show_clan {
                lines.push((
                    p.font_size_clan,
                    vec![Part::Text(&p.clan, Weight::Regular, weak)],
                ));
            }
            if p.show_client_id && p.client_id_separate_line {
                lines.push((
                    p.font_size_client_id,
                    vec![Part::Text(&id, Weight::Medium, weak)],
                ));
            }
            let mut hook = Vec::new();
            if p.show_hook_strong_weak {
                hook.push(match p.hook_strong_weak {
                    StrongWeak::Strong => {
                        Part::Icon(crate::icons::HOOK_STRONG, color::SUCCESS.gamma_multiply(a))
                    }
                    StrongWeak::Weak => {
                        Part::Icon(crate::icons::HOOK_WEAK, color::DANGER.gamma_multiply(a))
                    }
                    StrongWeak::Neutral => Part::Icon(crate::icons::HOOK_STRONG, weak),
                });
            }
            if p.show_hook_strong_weak_id {
                hook.push(Part::Text(&sw_id, Weight::Medium, weak));
            }
            lines.push((p.font_size_hook_strong_weak, hook));

            let mut bottom = view.to_screen(p.pos);
            for (world_size, parts) in &lines {
                if parts.is_empty() {
                    continue;
                }
                let size = world_size * k * 0.8;
                let gap = size * 0.25;
                let galleys: Vec<_> = (parts.iter())
                    .map(|part| match part {
                        Part::Icon(g, c) => {
                            painter.layout_no_wrap(g.to_string(), icon_font(size), *c)
                        }
                        Part::Text(s, w, c) => {
                            painter.layout_no_wrap(s.to_string(), font(size, *w), *c)
                        }
                    })
                    .collect();
                let width = galleys.iter().map(|g| g.size().x).sum::<f32>()
                    + gap * (galleys.len() - 1) as f32;
                let height = galleys.iter().map(|g| g.size().y).fold(0.0, f32::max);
                let mut x = bottom.x - width / 2.0;
                for g in galleys {
                    let at = egui::pos2(x, bottom.y - height + (height - g.size().y) / 2.0);
                    let rect = egui::Rect::from_min_size(at, g.size());
                    // a soft dark shadow keeps the text readable on any map
                    let shadow = Shadow {
                        offset: [0, 1],
                        blur: (size * 0.6) as u8,
                        spread: 0,
                        color: Color32::from_black_alpha((60.0 * a) as u8),
                    };
                    let behind = rect.shrink2(egui::vec2(0.0, size * 0.15));
                    painter.add(crate::widgets::soft_shadow(shadow, behind, size * 0.3));
                    x += g.size().x + gap;
                    painter.galley(at, g, Color32::WHITE);
                }
                bottom.y -= height;
            }
        }
    }
}

pub use draw::show;
