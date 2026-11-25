//! The watermark (PLAN §10.3, 7.5; SPEC §1): the logo and stats, the Dynamic Island's
//! idle state (7.9; the island is the one widget, `island.rs`). Settings under the
//! island's id: `hud_set island compact 0|1`, `hud_set island <item> 0|1`,
//! `hud_set island order "<item>,..."` and the look sliders.

use ddnet::map::ints_to_str;
use ddnet::physics::vmath::round_to_int;
use ddnet::protocol::generated as g;
use egui::{CornerRadius, Frame, Margin, Ui};

use crate::hud::{HudConfig, HudData};
use crate::icons;
use crate::style::{Glass, HUD_SHADOW, LOGO_GLOW, Weight, color, size, space};
use crate::widgets as w;

/// Stat keys in their default order, whether each shows by default (SPEC §1, except the
/// name: off, it already shows above the tee; user review 2026-11-25) and the
/// name in the editor.
pub const ITEMS: [(&str, bool, &str); 8] = [
    ("user", false, "Name"),
    ("server", false, "Server address"),
    ("ping", true, "Ping"),
    ("fps", true, "Framerate"),
    ("speed", false, "Speed"),
    ("time", false, "Local time"),
    ("uid", false, "Client ID"),
    ("tps", false, "Tick rate"),
];

/// The settings id: the island's (the watermark's settings were migrated to it).
pub const ID: &str = "island";

/// C++ `SERVER_TICK_SPEED`.
const TICK_SPEED: i32 = 50;

/// One stat: icon, value, optional unit.
pub type Stat = (&'static str, String, Option<&'static str>);

/// The watermark's tunable look (`hud_set watermark corner|logo_radius|logo_height|glow`),
/// set with sliders in the HUD editor until the user picks final defaults.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// The container's corner radius in percent of its height (0 square .. 50 pill).
    pub corner: f32,
    /// The logo's corner radius in points.
    pub logo_radius: f32,
    /// The logo's height in points (the mark plus padding).
    pub logo_height: f32,
    /// The logo's glow in percent.
    pub glow: f32,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            corner: 40.0,
            logo_radius: 8.0,
            // the earlier, larger logo: the mark and 12 above and below
            logo_height: size::ICON + 2.0 * space::M,
            glow: 100.0,
        }
    }
}

/// The sliders: (key, name, range).
const LOOK: [(&str, &str, std::ops::RangeInclusive<f32>); 4] = [
    ("corner", "Corner radius %", 0.0..=50.0),
    ("logo_radius", "Logo radius", 0.0..=20.0),
    ("logo_height", "Logo height", size::ICON..=48.0),
    ("glow", "Glow %", 0.0..=100.0),
];

impl Look {
    pub fn of(config: &HudConfig) -> Self {
        let d = Self::default();
        let n = |key, default| config.number(ID, key, default);
        Self {
            corner: n("corner", d.corner).clamp(0.0, 50.0),
            logo_radius: n("logo_radius", d.logo_radius).clamp(0.0, 20.0),
            logo_height: n("logo_height", d.logo_height).clamp(size::ICON, 48.0),
            glow: n("glow", d.glow).clamp(0.0, 100.0),
        }
    }
}

/// The container's padding: the logo 12 from the edges, the last stat 24.
pub fn margin(has_stats: bool) -> Margin {
    let right = if has_stats { space::XL } else { space::M };
    Margin {
        left: space::M as i8,
        right: right as i8,
        top: space::M as i8,
        bottom: space::M as i8,
    }
}

/// The container's corner radius for its `height`.
pub fn corner(look: Look, height: f32) -> u8 {
    (look.corner / 100.0 * height).round() as u8
}

/// The logo: a light rounded rectangle with a soft glow, the tee mark and "DDNet-rs"
/// (the mark only in compact mode).
pub fn logo(ui: &mut Ui, compact: bool, look: Look) {
    // padding from the height; at least 8 left and right
    let pad = ((look.logo_height - size::ICON) / 2.0).max(0.0);
    let logo_margin = Margin::symmetric(pad.max(space::S) as i8, pad as i8);
    let glow = egui::Shadow {
        color: LOGO_GLOW.color.gamma_multiply(look.glow / 100.0),
        ..LOGO_GLOW
    };
    Frame::new()
        .fill(color::LOGO)
        .corner_radius(CornerRadius::same(look.logo_radius.round() as u8))
        .inner_margin(logo_margin)
        .shadow(glow)
        .show(ui, |ui| {
            ui.spacing_mut().interact_size.y = size::ICON;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = space::S;
                w::tee_mark(ui, size::ICON, color::LOGO_TEXT, color::LOGO);
                if !compact {
                    w::text(
                        ui,
                        "DDNet-rs",
                        size::CAPTION,
                        Weight::Regular,
                        color::LOGO_TEXT,
                    );
                }
            });
        });
}

/// Draws the watermark alone (the gallery): a HUD bar with the logo, then the stats.
/// Spacing measured from the reference (PROGRESS 7.5).
pub fn draw(ui: &mut Ui, glass: Glass, compact: bool, stats: &[Stat], look: Look) {
    // the logo is the tallest item
    let row_height = look.logo_height;
    let corner = corner(look, row_height + 2.0 * space::M);
    let margin = margin(!stats.is_empty());
    w::hud_bar_with(ui, glass, corner, margin, row_height, HUD_SHADOW, |ui| {
        logo(ui, compact, look);
        if !stats.is_empty() {
            ui.add_space(space::XXL);
        }
        self::stats(ui, stats);
    });
}

/// Stats separated by dots.
pub fn stats(ui: &mut Ui, stats: &[Stat]) {
    for (i, (glyph, value, unit)) in stats.iter().enumerate() {
        if i > 0 {
            w::dot(ui);
        }
        w::stat(ui, glyph, value, *unit);
    }
}

/// The keys in the configured order: known keys from `order`, then the missing ones in
/// default order.
pub fn order(config: &HudConfig) -> Vec<&'static str> {
    let saved = config.value(ID, "order").unwrap_or("");
    let saved = saved
        .split(',')
        .filter_map(|k| ITEMS.iter().find(|i| i.0 == k.trim()));
    let mut keys = Vec::new();
    for (k, ..) in saved.chain(&ITEMS) {
        if !keys.contains(k) {
            keys.push(*k);
        }
    }
    keys
}

pub fn compact(config: &HudConfig) -> bool {
    config.on(ID, "compact", false)
}

/// Whether item `key` shows.
pub fn shown(config: &HudConfig, key: &str) -> bool {
    let default = ITEMS.iter().any(|&(k, on, _)| k == key && on);
    config.on(ID, key, default)
}

/// The value of item `key`, if known.
pub fn stat(key: &str, data: &HudData) -> Option<Stat> {
    let local = data.client.local_id();
    let snap = data.client.snapshot();
    Some(match key {
        "user" => {
            let name = snap
                .iter()
                .find(|it| it.type_id == g::NETOBJTYPE_CLIENTINFO && Some(it.id) == local)
                .and_then(|info| ints_to_str(&g::ObjClientInfo::from_ints(&info.data).name));
            let name = name.unwrap_or_else(|| data.player_name.to_string());
            (icons::USER, name, None)
        }
        "uid" => (icons::FINGERPRINT, local?.to_string(), Some("uid")),
        // C++: CHud::RenderTextInfo, round_to_int(1.0f / FrameTimeAverage())
        "fps" => {
            let fps = round_to_int(1.0 / data.frame_time_average);
            (icons::MONITOR, fps.to_string(), Some("fps"))
        }
        // C++: CScoreboard::RenderPlayers, std::clamp(m_Latency, 0, 999)
        "ping" => {
            let info = snap.iter().find(|it| {
                it.type_id == g::NETOBJTYPE_PLAYERINFO
                    && g::ObjPlayerInfo::from_ints(&it.data).local != 0
            })?;
            let ms = g::ObjPlayerInfo::from_ints(&info.data)
                .latency
                .clamp(0, 999);
            (icons::WIFI, ms.to_string(), Some("ms"))
        }
        "server" => (icons::GLOBE, data.server.to_string(), None),
        // SPEC §1 "bps": the horizontal speed of the movement information (7.6)
        "speed" => {
            let speed = crate::movement::of(data)?.speed.x;
            (icons::GAUGE, crate::movement::shown(speed), Some("t/s"))
        }
        "time" => (icons::CLOCK, clock(&chrono::Local::now()), None),
        "tps" => (icons::PULSE, TICK_SPEED.to_string(), Some("tps")),
        _ => return None,
    })
}

/// HH:MM, 24-hour.
pub fn clock(t: &impl chrono::Timelike) -> String {
    format!("{:02}:{:02}", t.hour(), t.minute())
}

/// The watermark settings in the island's popover: compact, then each item: on/off and
/// its place in the order, then the look.
pub fn settings(ui: &mut Ui, config: &mut HudConfig) {
    let mut order = order(config);
    let mut swap = None;
    egui::Grid::new("watermark").num_columns(3).show(ui, |ui| {
        let mut on = compact(config);
        if w::toggle(ui, &mut on).changed() {
            config.set(ID, "compact", on as i32);
        }
        ui.label("Compact");
        ui.end_row();
        for (i, key) in order.iter().enumerate() {
            let mut on = shown(config, key);
            if w::toggle(ui, &mut on).changed() {
                config.set(ID, key, on as i32);
            }
            ui.label(ITEMS.iter().find(|it| it.0 == *key).map_or(*key, |it| it.2));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = space::XS;
                let up =
                    ui.add_enabled_ui(i > 0, |ui| w::icon_button(ui, icons::CARET_UP, "Move up"));
                if up.inner.clicked() {
                    swap = Some(i - 1);
                }
                let down = ui.add_enabled_ui(i + 1 < order.len(), |ui| {
                    w::icon_button(ui, icons::CARET_DOWN, "Move down")
                });
                if down.inner.clicked() {
                    swap = Some(i);
                }
            });
            ui.end_row();
        }
    });
    if let Some(i) = swap {
        order.swap(i, i + 1);
        config.set(ID, "order", order.join(","));
    }
    // the look, until the user settles the defaults
    egui::Grid::new("watermark_look")
        .num_columns(2)
        .show(ui, |ui| {
            let look = Look::of(config);
            let values = [look.corner, look.logo_radius, look.logo_height, look.glow];
            for ((key, name, range), mut v) in LOOK.into_iter().zip(values) {
                ui.label(name);
                if w::slider(ui, &mut v, range).changed() {
                    config.set(ID, key, v.round());
                }
                ui.end_row();
            }
        });
}
