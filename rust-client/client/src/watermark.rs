//! The watermark (PLAN §10.3, 7.5; SPEC §1): one glass pill top left with the logo and
//! stats. Settings: `hud_set watermark compact 0|1`, `hud_set watermark <item> 0|1`,
//! `hud_set watermark order "<item>,..."`.

use ddnet::map::ints_to_str;
use ddnet::physics::vmath::round_to_int;
use ddnet::protocol::generated as g;
use egui::Ui;

use crate::hud::{Anchor, HudConfig, HudData, Placement, Widget};
use crate::icons;
use crate::style::{Glass, Weight, color, size};
use crate::widgets as w;

/// Stat keys in their default order, and whether each shows by default (SPEC §1).
pub const ITEMS: [(&str, bool); 6] = [
    ("user", true),
    ("server", false),
    ("ping", true),
    ("fps", true),
    ("uid", false),
    ("tps", false),
];

/// C++ `SERVER_TICK_SPEED`.
const TICK_SPEED: i32 = 50;

/// One stat: icon, value, optional unit.
pub type Stat = (&'static str, String, Option<&'static str>);

/// Draws the watermark pill (also used by the gallery).
pub fn draw(ui: &mut Ui, glass: Glass, compact: bool, stats: &[Stat]) {
    w::glass_pill(ui, glass, false, |ui| {
        w::pill(ui, color::LOGO, |ui| {
            w::icon(ui, icons::BOLT, size::ICON_SMALL, color::LOGO_TEXT);
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
        for (i, (glyph, value, unit)) in stats.iter().enumerate() {
            if i > 0 {
                w::dot(ui);
            }
            w::stat(ui, glyph, value, *unit);
        }
    });
}

/// The keys in the configured order: known keys from `order`, then the missing ones in
/// default order.
pub fn order(config: &HudConfig) -> Vec<&'static str> {
    let saved = config.value("watermark", "order").unwrap_or("");
    let saved = saved
        .split(',')
        .filter_map(|k| ITEMS.iter().find(|(i, _)| *i == k.trim()));
    let mut keys = Vec::new();
    for (k, _) in saved.chain(&ITEMS) {
        if !keys.contains(k) {
            keys.push(*k);
        }
    }
    keys
}

/// Whether item `key` shows.
pub fn shown(config: &HudConfig, key: &str) -> bool {
    match config.value("watermark", key) {
        Some(v) => v.trim() != "0",
        None => ITEMS.iter().any(|&(k, on)| k == key && on),
    }
}

/// The value of item `key`, if known.
pub fn stat(key: &str, data: &HudData) -> Option<Stat> {
    let local = data.client.local_id();
    let snap = data.client.snapshot();
    Some(match key {
        "user" => {
            let info = snap
                .iter()
                .find(|it| it.type_id == g::NETOBJTYPE_CLIENTINFO && Some(it.id) == local)?;
            let name = ints_to_str(&g::ObjClientInfo::from_ints(&info.data).name)?;
            (icons::USER, name, None)
        }
        "uid" => (icons::FINGERPRINT, local?.to_string(), Some("uid")),
        // C++: CHud::RenderTextInfo, round_to_int(1.0f / FrameTimeAverage())
        "fps" => {
            let fps = round_to_int(1.0 / data.frame_time_average);
            (icons::BOLT, fps.to_string(), Some("fps"))
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
            (icons::CLOUD, ms.to_string(), Some("ms"))
        }
        "server" => (icons::GLOBE, data.server.to_string(), None),
        "tps" => (icons::PULSE, TICK_SPEED.to_string(), Some("tps")),
        _ => return None,
    })
}

pub struct Watermark;

impl Widget for Watermark {
    fn id(&self) -> &'static str {
        "watermark"
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
        let compact = config
            .value("watermark", "compact")
            .is_some_and(|v| v.trim() != "0");
        draw(ui, data.glass, compact, &stats);
    }
}
