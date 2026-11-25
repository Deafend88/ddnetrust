//! The HUD system (PLAN §10.3, 7.3): widgets placed at one of nine screen anchors plus
//! an offset, drawn from a read-only view of the game (`HudData`). The layout and each
//! widget's settings are console lines in our settings.cfg:
//! `hud_widget <id> <anchor> <x> <y> <enabled>` and `hud_set <id> <key> <value>`.
//! Widgets only draw: they get `&HudData` and an egui `Ui`, never the client, the
//! controls or the settings, so they cannot change what is sent to the server.

use std::collections::BTreeMap;

use ddnet::net::client::Client;
use ddnet::predict::{Predictor, Rendered};
use ddnet::protocol::generated::GameMsg;
use egui::Align2;

use crate::config::{escape, parse_args, str_toint};
use crate::style::Glass;

/// One of the nine anchor points a widget sits at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    pub const ALL: [Anchor; 9] = [
        Self::TopLeft,
        Self::Top,
        Self::TopRight,
        Self::Left,
        Self::Center,
        Self::Right,
        Self::BottomLeft,
        Self::Bottom,
        Self::BottomRight,
    ];

    /// The name in settings.cfg.
    pub fn name(self) -> &'static str {
        [
            "top_left",
            "top",
            "top_right",
            "left",
            "center",
            "right",
            "bottom_left",
            "bottom",
            "bottom_right",
        ][self as usize]
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.name() == name)
    }

    /// egui places an area by the same point of its own rect (its size is known).
    pub fn align(self) -> Align2 {
        [
            Align2::LEFT_TOP,
            Align2::CENTER_TOP,
            Align2::RIGHT_TOP,
            Align2::LEFT_CENTER,
            Align2::CENTER_CENTER,
            Align2::RIGHT_CENTER,
            Align2::LEFT_BOTTOM,
            Align2::CENTER_BOTTOM,
            Align2::RIGHT_BOTTOM,
        ][self as usize]
    }
}

/// Where a widget is and whether it shows: its anchor point sits at the screen's anchor
/// point, moved by (`x`, `y`) points (right and down are positive).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub anchor: Anchor,
    pub x: i32,
    pub y: i32,
    pub enabled: bool,
}

/// The HUD part of settings.cfg: placements and widget settings by widget id. Ids and
/// keys not known to this version are kept, so a newer config survives a round trip.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HudConfig {
    pub placements: BTreeMap<String, Placement>,
    /// (widget id, key) -> value, as written in `hud_set`.
    pub values: BTreeMap<(String, String), String>,
}

impl HudConfig {
    /// `hud_widget` and `hud_set`; false for any other command. Malformed lines are
    /// ignored, like C++ console commands with bad arguments.
    pub fn execute(&mut self, command: &str, args: &str) -> bool {
        let mut a = Vec::new();
        match command {
            "hud_widget" => {
                if parse_args(args, "s[id] s[anchor] i[x] i[y] i[enabled]", &mut a)
                    && let Some(anchor) = Anchor::from_name(&a[1])
                {
                    // parse_args checked the three numbers with str_toint
                    let int = |i: usize| str_toint(&a[i]).unwrap_or(0);
                    let p = Placement {
                        anchor,
                        x: int(2),
                        y: int(3),
                        enabled: int(4) != 0,
                    };
                    self.placements.insert(a[0].clone(), p);
                }
            }
            "hud_set" => {
                if parse_args(args, "s[id] s[key] r[value]", &mut a) {
                    self.values
                        .insert((a[0].clone(), a[1].clone()), a[2].clone());
                }
            }
            _ => return false,
        }
        true
    }

    pub fn serialize(&self) -> Vec<String> {
        let placements = self.placements.iter().map(|(id, p)| {
            format!(
                "hud_widget \"{}\" {} {} {} {}",
                escape(id),
                p.anchor.name(),
                p.x,
                p.y,
                p.enabled as i32
            )
        });
        let values = self.values.iter().map(|((id, key), v)| {
            format!(
                "hud_set \"{}\" \"{}\" \"{}\"",
                escape(id),
                escape(key),
                escape(v)
            )
        });
        placements.chain(values).collect()
    }

    /// The saved placement of `id`, else its default.
    pub fn placement(&self, id: &str, default: Placement) -> Placement {
        self.placements.get(id).copied().unwrap_or(default)
    }

    /// Sets a widget's setting (`hud_set id key value`).
    pub fn set(&mut self, id: &str, key: &str, value: impl ToString) {
        self.values
            .insert((id.to_string(), key.to_string()), value.to_string());
    }

    /// Forgets the placement and settings of `id`: back to its defaults.
    pub fn reset(&mut self, id: &str) {
        self.placements.remove(id);
        self.values.retain(|(w, _), _| w != id);
    }

    /// An on/off setting (`hud_set id key 0|1`), `default` while unset.
    pub fn on(&self, id: &str, key: &str, default: bool) -> bool {
        self.value(id, key).map_or(default, |v| v.trim() != "0")
    }

    /// A number setting, `default` while unset or not a number.
    pub fn number(&self, id: &str, key: &str, default: f32) -> f32 {
        (self.value(id, key))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(default)
    }

    /// The C++ HUD settings as first values, for what our settings.cfg does not set yet
    /// (DDNet behaviour: an item that is off is not shown; a widget with no item on is
    /// hidden). `cpp` reads a C++ variable as on/off; `default` is a widget's default
    /// placement, whose on/off follows C++ too.
    pub fn import_cpp(
        &mut self,
        cpp: impl Fn(&str) -> bool,
        default: impl Fn(&str) -> Option<Placement>,
    ) {
        for (id, key, var) in [
            ("movement", "position", "cl_showhud_player_position"),
            ("movement", "speed", "cl_showhud_player_speed"),
            ("movement", "angle", "cl_showhud_player_angle"),
            ("status", "jumps", "cl_showhud_jumps_indicator"),
            ("killfeed", "kills", "cl_showkillmessages"),
            ("killfeed", "finishes", "cl_show_finish_messages"),
            ("island", "timer", "cl_showhud_timer"),
            ("island", "checkpoint", "cl_showhud_time_cp_diff"),
            ("island", "broadcasts", "cl_show_broadcasts"),
            ("island", "spectator", "cl_showhud_spectator"),
        ] {
            if self.value(id, key).is_none() {
                self.set(id, key, cpp(var) as i32);
            }
        }
        // C++ draws the movement panel when one of its lines is on, the DDRace HUD with
        // cl_showhud_ddrace
        let movement = ["position", "speed", "angle"].map(|k| self.on("movement", k, false));
        for (id, on) in [
            ("movement", movement.contains(&true)),
            ("status", cpp("cl_showhud_ddrace")),
        ] {
            if !self.placements.contains_key(id)
                && let Some(p) = default(id)
            {
                self.placements
                    .insert(id.into(), Placement { enabled: on, ..p });
            }
        }
    }

    /// A widget's setting as written in `hud_set`.
    pub fn value(&self, id: &str, key: &str) -> Option<&str> {
        self.values
            .get(&(id.to_string(), key.to_string()))
            .map(String::as_str)
    }
}

/// What widgets read each frame: a read-only view of the game (the ping is the local
/// player's `PlayerInfo` latency in `client.snapshot()`, as the C++ scoreboard shows
/// it).
pub struct HudData<'a> {
    pub client: &'a Client,
    /// The server's address as typed.
    pub server: &'a str,
    /// The name we asked for (`player_name`, else C++'s default), shown until the
    /// server's `ClientInfo` (which may rename us) arrives.
    pub player_name: &'a str,
    /// C++ `CClient::m_FrameTimeAverage` (seconds).
    pub frame_time_average: f32,
    /// The glass strength panels are drawn with.
    pub glass: Glass,
    /// The HUD editor is open: widgets with nothing to show draw a sample, so they can
    /// be placed.
    pub editing: bool,
    pub predictor: Option<&'a Predictor>,
    /// The followed tee (the local one, or the one followed with `--follow`); the
    /// spectator camera is not ported yet, so spectating keeps the local id.
    pub tee: Option<&'a Rendered>,
    /// Game messages received since the last frame (broadcasts, kill and finish
    /// messages, records), oldest first.
    pub events: &'a [GameMsg],
}

/// A HUD widget.
pub trait Widget {
    /// The id in settings.cfg.
    fn id(&self) -> &'static str;
    /// The name and icon in the HUD editor.
    fn name(&self) -> &'static str;
    fn icon(&self) -> &'static str;
    fn default_placement(&self) -> Placement;
    /// Draws the widget; `data` and `config` are all it reads.
    fn show(&mut self, ui: &mut egui::Ui, data: &HudData, config: &HudConfig);
    /// The widget's own settings in its editor popover.
    fn settings(&mut self, _ui: &mut egui::Ui, _config: &mut HudConfig) {}
}

/// On/off toggles for a widget's items (`hud_set id key 0|1`), each `(key, name,
/// default)`.
pub fn item_toggles(
    ui: &mut egui::Ui,
    config: &mut HudConfig,
    id: &str,
    items: &[(&str, &str, bool)],
) {
    egui::Grid::new(("hud_items", id))
        .num_columns(2)
        .show(ui, |ui| {
            for &(key, name, default) in items {
                let mut on = config.on(id, key, default);
                if crate::widgets::toggle(ui, &mut on).changed() {
                    config.set(id, key, on as i32);
                }
                ui.label(name);
                ui.end_row();
            }
        });
}

/// The egui area a widget is drawn in.
pub fn area_id(id: &str) -> egui::Id {
    egui::Id::unique(("hud", id))
}

/// Draws every enabled widget at its placement.
pub fn show(
    ctx: &egui::Context,
    widgets: &mut [Box<dyn Widget>],
    data: &HudData,
    config: &HudConfig,
) {
    for w in widgets {
        let p = config.placement(w.id(), w.default_placement());
        if !p.enabled {
            continue;
        }
        egui::Area::new(area_id(w.id()))
            .anchor(p.anchor.align(), [p.x as f32, p.y as f32])
            .interactable(false)
            .order(egui::Order::Background)
            .show(ctx, |ui| w.show(ui, data, config));
    }
}
