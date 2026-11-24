//! The watermark (PLAN §10.3, 7.5; SPEC §1): one glass pill top left with the logo and
//! stats. Settings: `hud_set watermark compact 0|1`, `hud_set watermark <item> 0|1`,
//! `hud_set watermark order "<item>,..."`.

use ddnet::map::ints_to_str;
use ddnet::physics::vmath::round_to_int;
use ddnet::protocol::generated as g;
use egui::{CornerRadius, Frame, Margin, Ui};

use crate::hud::{Anchor, HudConfig, HudData, Placement, Widget};
use crate::icons;
use crate::style::{Glass, LOGO_GLOW, Weight, color, radius, size, space};
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

/// C++ `SERVER_TICK_SPEED`.
const TICK_SPEED: i32 = 50;

/// One stat: icon, value, optional unit.
pub type Stat = (&'static str, String, Option<&'static str>);

/// Draws the watermark (also used by the gallery): a HUD bar with the logo (a light
/// rounded rectangle with a soft glow: the tee mark and "DDNet-rs", the mark only in
/// compact mode), then the stats. Spacing measured from the reference (PROGRESS 7.5).
pub fn draw(ui: &mut Ui, glass: Glass, compact: bool, stats: &[Stat]) {
    let right = if stats.is_empty() {
        space::M
    } else {
        space::XL
    };
    let margin = Margin {
        left: space::M as i8,
        right: right as i8,
        top: space::M as i8,
        bottom: space::M as i8,
    };
    // the logo is the tallest item: the mark and its padding
    let row_height = size::ICON + 2.0 * space::S;
    w::hud_bar(ui, glass, false, margin, row_height, |ui| {
        Frame::new()
            .fill(color::LOGO)
            .corner_radius(CornerRadius::same(radius::SMALL))
            .inner_margin(Margin::symmetric(space::M as i8, space::S as i8))
            .shadow(LOGO_GLOW)
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
    let saved = config.value("watermark", "order").unwrap_or("");
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

fn compact(config: &HudConfig) -> bool {
    config
        .value("watermark", "compact")
        .is_some_and(|v| v.trim() != "0")
}

/// Whether item `key` shows.
pub fn shown(config: &HudConfig, key: &str) -> bool {
    match config.value("watermark", key) {
        Some(v) => v.trim() != "0",
        None => ITEMS.iter().any(|&(k, on, _)| k == key && on),
    }
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

pub struct Watermark;

impl Widget for Watermark {
    fn id(&self) -> &'static str {
        "watermark"
    }

    fn name(&self) -> &'static str {
        "Watermark"
    }

    fn icon(&self) -> &'static str {
        icons::INFO
    }

    fn default_placement(&self) -> Placement {
        Placement {
            anchor: Anchor::TopLeft,
            x: 16,
            y: 16,
            enabled: true,
        }
    }

    fn show(&mut self, ui: &mut Ui, data: &HudData, config: &HudConfig) {
        let stats: Vec<Stat> = order(config)
            .into_iter()
            .filter(|k| shown(config, k))
            .filter_map(|k| stat(k, data))
            .collect();
        draw(ui, data.glass, compact(config), &stats);
    }

    /// Compact, then each item: on/off and its place in the order.
    fn settings(&mut self, ui: &mut Ui, config: &mut HudConfig) {
        let mut order = order(config);
        let mut swap = None;
        egui::Grid::new("watermark").num_columns(3).show(ui, |ui| {
            let mut on = compact(config);
            if w::toggle(ui, &mut on).changed() {
                config.set("watermark", "compact", on as i32);
            }
            ui.label("Compact");
            ui.end_row();
            for (i, key) in order.iter().enumerate() {
                let mut on = shown(config, key);
                if w::toggle(ui, &mut on).changed() {
                    config.set("watermark", key, on as i32);
                }
                ui.label(ITEMS.iter().find(|it| it.0 == *key).map_or(*key, |it| it.2));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = space::XS;
                    let up = ui
                        .add_enabled_ui(i > 0, |ui| w::icon_button(ui, icons::CARET_UP, "Move up"));
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
            config.set("watermark", "order", order.join(","));
        }
    }
}
