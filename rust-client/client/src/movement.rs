//! The position/speed widget (PLAN §10.3, 7.6; replaces the C++ movement information
//! panel): position in tiles, speed in tiles/s and aim angle of the followed tee, each
//! toggleable, in the watermark's style. Bottom right. Settings:
//! `hud_set movement <position|speed|angle> 0|1`.

use ddnet::physics::core::velocity_ramp;
use ddnet::physics::tuning::{Param, Tuning};
use ddnet::physics::vmath::{PI, Vec2, length, mix, vec2};
use ddnet::protocol::generated::ObjCharacter;
use egui::Ui;

use crate::hud::{Anchor, HudConfig, HudData, Placement, Widget};
use crate::icons;
use crate::render::players::target_angle;
use crate::watermark::{self, Stat};
use crate::widgets as w;

/// C++ `SERVER_TICK_SPEED` (`Client()->GameTickSpeed()`).
const TICK_SPEED: f32 = 50.0;

/// C++: `CHud::CMovementInformation`.
#[derive(Clone, Copy, Debug)]
pub struct Info {
    pub pos: Vec2,
    pub speed: Vec2,
    pub angle: f32,
}

/// C++: `CHud::GetMovementInformation` (hud.cpp:1424) for a character in the snapshot
/// (the spectator paths, freeview and `m_SpecChar`, are not ported). `aim` is
/// `CPlayers::GetPlayerTargetAngle`'s result, `intra` `IntraGameTick`.
pub fn info(
    prev: &ObjCharacter,
    cur: &ObjCharacter,
    intra: f32,
    tuning: &Tuning,
    aim: f32,
) -> Info {
    let div = |v: Vec2, d: f32| vec2(v.x / d, v.y / d);
    // To make the player position relative to blocks we need to divide by the block size
    let pos = mix(
        vec2(prev.x as f32, prev.y as f32),
        vec2(cur.x as f32, cur.y as f32),
        intra,
    );
    let vel = mix(
        vec2(prev.vel_x as f32, prev.vel_y as f32),
        vec2(cur.vel_x as f32, cur.vel_y as f32),
        intra,
    );
    let mut velspeed_x = vel.x / 256.0 * TICK_SPEED;
    if (-1.0..=1.0).contains(&vel.x) {
        velspeed_x = 0.0;
    }
    let mut velspeed_y = vel.y / 256.0 * TICK_SPEED;
    if (-128.0..=128.0).contains(&vel.y) {
        velspeed_y = 0.0;
    }
    // We show the speed in Blocks per Second (Bps) and therefore have to divide by the
    // block size
    let velspeed_length = length(div(vel, 256.0)) * TICK_SPEED;
    // C++ uses the local tuning for every player (a "Todo" there)
    let ramp = velocity_ramp(
        velspeed_length,
        tuning.get(Param::VelrampStart),
        tuning.get(Param::VelrampRange),
        tuning.get(Param::VelrampCurvature),
    );
    let mut angle = aim;
    if angle < 0.0 {
        angle += 2.0 * PI;
    }
    Info {
        pos: div(pos, 32.0),
        speed: vec2(velspeed_x / 32.0 * ramp, velspeed_y / 32.0),
        angle: angle * 180.0 / PI,
    }
}

/// The followed tee's movement information, as `CHud::RenderMovementInformation` gets it.
pub fn of(data: &HudData) -> Option<Info> {
    let tee = data.tee?;
    let client = data.client;
    let aim = target_angle(client, tee, client.local_id());
    // C++ m_aTuning: the server's tuning is not received yet, so the default
    let tuning = Tuning::default();
    Some(info(&tee.prev, &tee.cur, client.intra_tick, &tuning, aim))
}

/// C++: `CHud::UpdateMovementInformationTextContainer` (hud.cpp:1401): rounded to two
/// decimals, then `%.2f`.
pub fn shown(value: f32) -> String {
    let value = (value * 100.0).round() / 100.0;
    format!("{value:.2}")
}

/// Item keys and editor names; all shown once the widget is on (C++
/// `cl_showhud_player_position`/`_speed`/`_angle` default to 0: the widget is off by
/// default instead).
pub const ITEMS: [(&str, &str); 3] = [
    ("position", "Position"),
    ("speed", "Speed"),
    ("angle", "Aim angle"),
];

fn shown_item(config: &HudConfig, key: &str) -> bool {
    config
        .value("movement", key)
        .is_none_or(|v| v.trim() != "0")
}

pub struct Movement;

impl Widget for Movement {
    fn id(&self) -> &'static str {
        "movement"
    }

    fn name(&self) -> &'static str {
        "Position and speed"
    }

    fn icon(&self) -> &'static str {
        icons::GAUGE
    }

    fn default_placement(&self) -> Placement {
        Placement {
            anchor: Anchor::BottomRight,
            x: -16,
            y: -16,
            enabled: false,
        }
    }

    fn show(&mut self, ui: &mut Ui, data: &HudData, config: &HudConfig) {
        let Some(i) = of(data) else { return };
        let pair = |v: Vec2| format!("{}, {}", shown(v.x), shown(v.y));
        let stats: Vec<Stat> = ITEMS
            .iter()
            .filter(|(k, _)| shown_item(config, k))
            .map(|&(k, _)| match k {
                "position" => (icons::MAP_PIN, pair(i.pos), None),
                "speed" => (icons::GAUGE, pair(i.speed), Some("tiles/s")),
                _ => (icons::COMPASS, shown(i.angle), Some("°")),
            })
            .collect();
        if stats.is_empty() {
            return;
        }
        w::glass_pill(ui, data.glass, false, |ui| watermark::stats(ui, &stats));
    }

    fn settings(&mut self, ui: &mut Ui, config: &mut HudConfig) {
        egui::Grid::new("movement").num_columns(2).show(ui, |ui| {
            for (key, name) in ITEMS {
                let mut on = shown_item(config, key);
                if w::toggle(ui, &mut on).changed() {
                    config.set("movement", key, on as i32);
                }
                ui.label(name);
                ui.end_row();
            }
        });
    }
}
