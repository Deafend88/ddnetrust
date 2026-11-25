//! The position/speed widget (PLAN §10.3, 7.6; replaces the C++ movement information
//! panel): position in tiles, speed in tiles/s and aim angle of the followed tee, each
//! toggleable, in the watermark's style. Bottom right. Settings:
//! `hud_set movement <position|speed|angle> 0|1`.

use ddnet::physics::core::velocity_ramp;
use ddnet::physics::tuning::{Param, Tuning};
use ddnet::physics::vmath::{PI, Vec2, length, mix, vec2};
use ddnet::protocol::generated::ObjCharacter;
use egui::{Margin, Ui};

use crate::hud::{Anchor, HudConfig, HudData, Placement, Widget, item_toggles};
use crate::icons;
use crate::render::players::target_angle;
use crate::style::{Weight, color, font, radius, size, space};
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
/// The values' minimum widths as templates ("-00.00": a sign, two digits, two
/// decimals), so the row does not jump as the numbers change (digits are tabular).
pub const POSITION: &str = "0000.00";
pub const SPEED: &str = "-00.00";
pub const ANGLE: &str = "-000.00";

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
    // the first values come from C++'s cl_showhud_player_* (HudConfig::import_cpp)
    config.on("movement", key, true)
}

/// An item: icon, labelled values ("X", "57.53"), the values' minimum width, unit.
type Item = (
    &'static str,
    Vec<(&'static str, String)>,
    &'static str,
    Option<&'static str>,
);

/// "X" then the value, then the unit if any: label and unit in the secondary grey like
/// the watermark's units.
fn value(ui: &mut Ui, (label, v): &(&str, String), template: &str, unit: Option<&str>) {
    let weak = ui.visuals().weak_text_color();
    w::text(ui, label, size::BODY, Weight::Regular, weak);
    ui.add_space(space::XS);
    w::number(ui, v, template, size::BODY, Weight::Medium, color::TEXT);
    if let Some(unit) = unit {
        w::text(ui, unit, size::BODY, Weight::Regular, weak);
    }
}

/// The items in a row: each icon, then "X 57.53  Y 15.53" with the unit after the last
/// value, or (`stacked`) the values under each other, each with its unit. A one-value
/// item stays a single line, so it and its icon centre on the row like the others.
fn row(ui: &mut Ui, items: &[Item], stacked: bool) {
    for (n, (glyph, values, template, unit)) in items.iter().enumerate() {
        if n > 0 {
            w::dot(ui);
        }
        w::icon(ui, glyph, size::ICON, color::TEXT);
        ui.add_space(space::S);
        if stacked && values.len() > 1 {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = space::XS;
                ui.spacing_mut().interact_size.y = 0.0;
                for v in values {
                    ui.horizontal(|ui| value(ui, v, template, *unit));
                }
            });
            continue;
        }
        for (k, v) in values.iter().enumerate() {
            if k > 0 {
                ui.add_space(space::M);
            }
            value(ui, v, template, unit.filter(|_| k + 1 == values.len()));
        }
    }
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
        let info = of(data);
        let any_on = ITEMS.iter().any(|(k, _)| shown_item(config, k));
        // DDNet's rule: every item off hides the widget (in the editor: a placeholder)
        let Some(i) = info.filter(|_| any_on) else {
            if data.editing {
                w::placeholder(ui, data.glass, self.icon(), self.name());
            }
            return;
        };
        let xy = |v: Vec2| vec![("X", shown(v.x)), ("Y", shown(v.y))];
        let items: Vec<Item> = ITEMS
            .iter()
            .filter(|(k, _)| shown_item(config, k))
            .map(|&(k, _)| match k {
                "position" => (icons::MAP_PIN, xy(i.pos), POSITION, None),
                "speed" => (icons::GAUGE, xy(i.speed), SPEED, Some("t/s")),
                _ => (
                    icons::CROSSHAIR,
                    vec![("aim", shown(i.angle))],
                    ANGLE,
                    Some("°"),
                ),
            })
            .collect();
        let stacked = config.on("movement", "stacked", false);
        let margin = Margin::symmetric(space::XL as i8, space::L as i8);
        // two-row items make it a multi-line widget: a rounded rectangle, not a pill
        let multi_line = stacked && items.iter().any(|i| i.1.len() > 1);
        let line = (ui.ctx()).fonts_mut(|f| f.row_height(&font(size::BODY, Weight::Medium)));
        let row_height = if multi_line {
            2.0 * line + space::XS
        } else {
            line.max(size::ICON)
        };
        w::hud_bar(
            ui,
            data.glass,
            if multi_line {
                radius::CARD
            } else {
                radius::PILL
            },
            margin,
            row_height,
            |ui| row(ui, &items, stacked),
        );
    }

    fn settings(&mut self, ui: &mut Ui, config: &mut HudConfig) {
        let items: Vec<(&str, &str, bool)> = std::iter::once(("stacked", "Two rows", false))
            .chain(ITEMS.iter().map(|&(k, n)| (k, n, true)))
            .collect();
        item_toggles(ui, config, "movement", &items);
    }
}
