//! The in-game menu (Esc, PLAN §13.5, C++ `menus_ingame.cpp`) in our design: a compact
//! bar across the top over the running game. Tabs as a segmented control, the Game tab's
//! actions as pills, Disconnect apart in muted red, icon buttons on the right; the other
//! tabs open a panel under the bar. Parts whose feature comes later stay hidden (Ghost
//! 13.8, Vote 13.7, Record demo 13.8, Dummy 13.6, Browser M11; no editor).

use egui::{Align, Align2, CornerRadius, Layout, Rect, RichText, Sense, Ui, UiBuilder};

use ddnet::clients::Clients;
use ddnet::net::serverinfo::{FLAG_PASSWORD, ServerInfo};
use ddnet::protocol::generated as g;

use crate::icons;
use crate::style::{Glass, Weight, color, font, icon_font, radius, size, space};
use crate::teehead::{TeeHeads, TeeLook};
use crate::widgets as w;

/// A Game-page button of C++ `CMenus::RenderGame`, in drawing order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameButton {
    Spectate,
    JoinRed,
    JoinBlue,
    JoinGame,
    /// C++ "Kill" (`SendKill`); ours says Respawn.
    Kill,
    /// `say /pause`, labelled "Pause" while playing ...
    Pause,
    /// ... and "Join game" while paused or spectating.
    Unpause,
    /// The auto spectator camera toggle (comes with spectating, 13.7).
    AutoCamera,
}

impl GameButton {
    /// The C++ label (the button list `rs_parity menus` writes).
    pub fn cpp_label(self) -> &'static str {
        match self {
            Self::Spectate => "Spectate",
            Self::JoinRed => "Join red",
            Self::JoinBlue => "Join blue",
            Self::JoinGame | Self::Unpause => "Join game",
            Self::Kill => "Kill",
            Self::Pause => "Pause",
            Self::AutoCamera => "<auto camera>",
        }
    }
}

/// What `RenderGame` reads to choose its buttons.
#[derive(Clone, Copy, Debug, Default)]
pub struct GameState {
    /// C++ `m_Snap.m_LocalClientId >= 0`.
    pub has_local_id: bool,
    /// C++ `m_Snap.m_pLocalInfo->m_Team`, `None` without local info.
    pub local_team: Option<i32>,
    /// C++ `m_Snap.m_pGameInfoObj != nullptr`.
    pub game_info: bool,
    /// The local client's `m_Paused` and `m_Spec`.
    pub paused: bool,
    pub spec: bool,
    pub team_play: bool,
    pub received_ddnet_player: bool,
    /// C++ `ShowDDRaceButtons` (`MainView.w > 855`): our bar always has room.
    pub show_ddrace_buttons: bool,
    pub dummy_connecting: bool,
}

/// C++: the buttons of `CMenus::RenderGame` (menus_ingame.cpp:140-251), Disconnect,
/// dummy and demo aside.
pub fn game_buttons(s: &GameState) -> Vec<GameButton> {
    use GameButton::*;
    let (paused, spec) = if s.has_local_id {
        (s.paused, s.spec)
    } else {
        (false, false)
    };
    let room = s.show_ddrace_buttons || !s.team_play;
    let connecting = s.dummy_connecting;
    let mut out = Vec::new();
    if let Some(team) = s.local_team
        && s.game_info
        && !paused
        && !spec
    {
        if team != g::TEAM_SPECTATORS && !connecting {
            out.push(Spectate);
        }
        if s.team_play {
            if team != g::TEAM_RED && !connecting {
                out.push(JoinRed);
            }
            if team != g::TEAM_BLUE && !connecting {
                out.push(JoinBlue);
            }
        } else if team != g::TEAM_RED && !connecting {
            // C++ TEAM_GAME = TEAM_RED
            out.push(JoinGame);
        }
        if team != g::TEAM_SPECTATORS && room {
            out.push(Kill);
        }
    }
    if let Some(team) = s.local_team
        && s.received_ddnet_player
        && room
        && (team != g::TEAM_SPECTATORS || paused || spec)
    {
        out.push(if !paused && !spec { Pause } else { Unpause });
    }
    if let Some(team) = s.local_team
        && (team == g::TEAM_SPECTATORS || paused || spec)
    {
        out.push(AutoCamera);
    }
    out
}

/// What the player chose in the bar this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// C++ `SendSwitchTeam`.
    SetTeam(i32),
    /// C++ `SendKill`.
    Kill,
    /// C++ `say /pause`.
    Pause,
    Disconnect,
    Settings,
    EditHud,
    Quit,
    /// C++: `m_ChatIgnore ^= 1` of this client.
    ToggleChatIgnore(i32),
    /// C++: `AddFriend` / `RemoveFriend` of this client's name and clan.
    ToggleFriend(i32),
}

const TABS: [&str; 3] = ["Game", "Players", "Info"];
const TAB_PLAYERS: usize = 1;
const TAB_INFO: usize = 2;

/// The menu's own state.
#[derive(Default)]
pub struct EscMenu {
    pub tab: usize,
    /// Disconnect asks first (C++ `PopupConfirm`).
    confirming: bool,
    /// The Game tab's actions' width last frame, to centre them.
    actions_width: f32,
}

/// What the bar shows.
pub struct View<'a> {
    pub glass: Glass,
    pub game: GameState,
    pub clients: &'a Clients,
    /// C++ `cl_show_chat_friends`.
    pub show_chat_friends: bool,
    pub heads: &'a mut TeeHeads,
    pub server: Option<&'a ServerInfo>,
    pub address: &'a str,
    pub player_name: &'a str,
    pub map: &'a str,
    pub game_info: Option<g::ObjGameInfo>,
    pub motd: &'a str,
    /// Disconnect needs a confirmation (C++: race time over `cl_confirm_disconnect_time`).
    pub confirm_disconnect: bool,
}

/// The tab panels' width at most.
const PANEL_WIDTH: f32 = 960.0;

/// A rounded rectangle of glass with the menu's inset on every side.
fn card<R>(ui: &mut Ui, glass: Glass, add: impl FnOnce(&mut Ui) -> R) -> R {
    w::card(ui, glass, space::XL, add).inner
}

/// A pill button with an icon (`danger`: Disconnect's muted red).
fn pill_button(ui: &mut Ui, glyph: &str, label: &str, danger: bool) -> egui::Response {
    let (text, fill) = if danger {
        (color::DANGER, color::DANGER.gamma_multiply(0.14))
    } else {
        (color::TEXT, color::SURFACE_RAISED)
    };
    let b = egui::Button::new((
        RichText::new(glyph)
            .font(icon_font(size::ICON_LARGE))
            .color(text),
        RichText::new(label)
            .font(font(size::TITLE, Weight::Medium))
            .color(text),
    ))
    .fill(fill)
    .corner_radius(CornerRadius::same(radius::PILL))
    .min_size(egui::vec2(0.0, size::CONTROL_LARGE));
    ui.add(b)
}

/// A round icon button with a tooltip.
fn round_button(ui: &mut Ui, glyph: &str, tooltip: &str) -> egui::Response {
    let b = egui::Button::new(RichText::new(glyph).font(icon_font(size::ICON_LARGE)))
        .fill(color::SURFACE_RAISED)
        .corner_radius(CornerRadius::same(radius::PILL))
        .min_size(egui::vec2(size::CONTROL_LARGE, size::CONTROL_LARGE));
    ui.add(b).on_hover_text(tooltip)
}

/// A toggle drawn as an icon: the accent when on, muted when off (`enabled` false:
/// C++ draws it checked and inactive).
fn icon_toggle(ui: &mut Ui, glyph: &str, on: bool, enabled: bool, tooltip: &str) -> bool {
    let c = if on {
        color::ACCENT
    } else {
        color::TEXT_SECONDARY
    };
    let b = egui::Button::new(
        RichText::new(glyph)
            .font(icon_font(size::ICON_LARGE))
            .color(c),
    )
    .frame(false)
    .min_size(egui::vec2(size::CONTROL_LARGE, size::CONTROL_LARGE));
    ui.add_enabled(enabled, b)
        .on_hover_text(tooltip)
        .on_disabled_hover_text(tooltip)
        .clicked()
}

/// A child of `ui` with `layout` inside `rect`; returns what it used.
fn row_in<R>(ui: &mut Ui, rect: Rect, layout: Layout, add: impl FnOnce(&mut Ui) -> R) -> (R, Rect) {
    let r = ui.scope_builder(UiBuilder::new().max_rect(rect).layout(layout), add);
    (r.inner, r.response.rect)
}

impl EscMenu {
    /// Draws the bar (and the open tab's panel) and returns what was chosen.
    pub fn show(&mut self, ctx: &egui::Context, v: &mut View) -> Option<Action> {
        let mut action = None;
        let screen = ctx.content_rect();
        let width = screen.width() - 2.0 * space::XL;
        let bar_bottom = egui::Area::new(egui::Id::unique("esc_menu"))
            .anchor(Align2::CENTER_TOP, [0.0, space::XL])
            .show(ctx, |ui| {
                ui.set_width(width);
                card(ui, v.glass, |ui| {
                    ui.set_width(width - 2.0 * space::XL);
                    action = self.bar(ui, v);
                });
            })
            .response
            .rect
            .bottom();
        if self.tab == 0 {
            return action;
        }
        let panel_width = PANEL_WIDTH.min(width);
        let top = bar_bottom - screen.top() + space::L;
        // the panel ends an inset above the screen's bottom; longer content scrolls
        let max_height = screen.height() - top - space::XL - 2.0 * space::XL;
        egui::Area::new(egui::Id::unique("esc_panel"))
            .anchor(Align2::CENTER_TOP, [0.0, top])
            .show(ctx, |ui| {
                // an Area offers last frame's size; the panel may grow up to its limit
                let room = egui::vec2(panel_width, max_height + 2.0 * space::XL);
                let rect = Rect::from_min_size(ui.max_rect().min, room);
                ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
                    card(ui, v.glass, |ui| {
                        ui.set_width(panel_width - 2.0 * space::XL);
                        ui.spacing_mut().item_spacing = egui::vec2(space::M, space::M);
                        egui::ScrollArea::vertical().max_height(max_height).show(
                            ui,
                            |ui| match self.tab {
                                TAB_PLAYERS => action = action.or(players(ui, v)),
                                TAB_INFO => info(ui, v),
                                _ => {}
                            },
                        );
                    });
                });
            });
        action
    }

    /// Called when the menu opens: no question left open from before.
    pub fn reset(&mut self) {
        self.confirming = false;
    }

    /// The bar: tabs on the left, icon buttons on the right, the Game tab's actions
    /// centred on the bar between them (on a row of their own when they do not fit).
    fn bar(&mut self, ui: &mut Ui, v: &View) -> Option<Action> {
        let mut action = None;
        ui.spacing_mut().item_spacing = egui::vec2(space::M, space::M);
        ui.spacing_mut().button_padding = egui::vec2(space::L, space::S);
        ui.spacing_mut().icon_spacing = space::S;
        let h = size::CONTROL_LARGE;
        let full = egui::vec2(ui.available_width(), h);
        let (row, _) = ui.allocate_exact_size(full, Sense::hover());
        let ltr = Layout::left_to_right(Align::Center);
        // top-aligned: a Frame in a centred row lands its margin too low, and the
        // control is as tall as the row anyway
        let (_, tabs) = row_in(ui, row, Layout::left_to_right(Align::Min), |ui| {
            w::segmented_sized(ui, &mut self.tab, &TABS, size::TITLE, h);
        });
        let (_, buttons) = row_in(ui, row, Layout::right_to_left(Align::Center), |ui| {
            if round_button(ui, icons::POWER, "Quit").clicked() {
                action = Some(Action::Quit);
            }
            if round_button(ui, icons::GEAR, "Settings").clicked() {
                action = Some(Action::Settings);
            }
            if round_button(ui, icons::MOVE, "Edit HUD").clicked() {
                action = Some(Action::EditHud);
            }
        });
        if self.tab != 0 {
            return action;
        }
        // centred by last frame's width; a changed width draws the frame again
        let aw = self.actions_width;
        let mut at = row;
        at.min.x = row.center().x - aw / 2.0;
        if at.min.x < tabs.right() + space::XL || at.min.x + aw > buttons.left() - space::XL {
            (at, _) = ui.allocate_exact_size(full, Sense::hover());
            at.min.x = (at.center().x - aw / 2.0).max(at.left());
        }
        let (a, used) = row_in(ui, at, ltr, |ui| self.game_row(ui, v));
        if (used.width() - aw).abs() > 0.5 {
            self.actions_width = used.width();
            ui.ctx().request_discard("esc menu actions re-centred");
        }
        action.or(a)
    }

    /// The Game tab's actions: the C++ buttons as pills, a divider, Disconnect (with the
    /// C++ confirmation as Yes/No pills in its place).
    fn game_row(&mut self, ui: &mut Ui, v: &View) -> Option<Action> {
        let mut action = None;
        for b in game_buttons(&v.game) {
            let (glyph, label, a) = match b {
                GameButton::Spectate => (
                    icons::BINOCULARS,
                    "Spectate",
                    Action::SetTeam(g::TEAM_SPECTATORS),
                ),
                GameButton::JoinRed => (icons::USERS, "Join red", Action::SetTeam(g::TEAM_RED)),
                GameButton::JoinBlue => (icons::USERS, "Join blue", Action::SetTeam(g::TEAM_BLUE)),
                GameButton::JoinGame => (icons::JOIN, "Join game", Action::SetTeam(g::TEAM_RED)),
                GameButton::Kill => (icons::RESPAWN, "Respawn", Action::Kill),
                GameButton::Pause => (icons::PAUSE_GAME, "Pause", Action::Pause),
                GameButton::Unpause => (icons::JOIN, "Join game", Action::Pause),
                // hidden until spectating lands (13.7)
                GameButton::AutoCamera => continue,
            };
            if pill_button(ui, glyph, label, false).clicked() {
                action = Some(a);
            }
        }
        ui.add(egui::Separator::default().vertical().spacing(space::M));
        if self.confirming {
            let weak = v.glass.text_secondary();
            w::text(ui, "Disconnect?", size::TITLE, Weight::Medium, weak);
            if pill_button(ui, icons::DISCONNECT, "Yes", true).clicked() {
                action = Some(Action::Disconnect);
            }
            if pill_button(ui, icons::GAME, "No", false).clicked() {
                self.confirming = false;
            }
        } else if pill_button(ui, icons::DISCONNECT, "Disconnect", true).clicked() {
            if v.confirm_disconnect {
                self.confirming = true;
            } else {
                action = Some(Action::Disconnect);
            }
        }
        action
    }
}

/// C++ `RenderPlayers` (menus_ingame.cpp:492): every other player with chat ignore and
/// friend toggles, one row each: tee, name, clan and the toggles in fixed columns.
fn players(ui: &mut Ui, v: &mut View) -> Option<Action> {
    let mut action = None;
    let local = v.clients.local_id;
    let ids: Vec<i32> = (v.clients.by_name.iter())
        .copied()
        .filter(|&id| id != local)
        .collect();
    let weak = v.glass.text_secondary();
    if ids.is_empty() {
        let s = "You are the only player here.";
        w::text(ui, s, size::TITLE, Weight::Regular, weak);
        return None;
    }
    let h = size::CONTROL_LARGE;
    // tee, name, clan, two toggles, with the row's gap between them
    let text_w = ui.available_width() - size::TEE_LARGE - 2.0 * h - 4.0 * space::M;
    let ltr = Layout::left_to_right(Align::Center);
    for (i, id) in ids.into_iter().enumerate() {
        if i > 0 {
            ui.separator();
        }
        let c = &v.clients.clients[id as usize];
        ui.horizontal(|ui| {
            let look = TeeLook {
                skin: c.skin.clone(),
                custom_color: c.use_custom_color != 0,
                color_body: c.color_body,
                color_feet: c.color_feet,
            };
            let ctx = ui.ctx().clone();
            let tee = egui::vec2(size::TEE_LARGE, size::TEE_LARGE);
            match v.heads.get(&ctx, &look) {
                Some(t) => {
                    ui.add(egui::Image::new((t.id(), tee)))
                        .on_hover_text(&c.skin);
                }
                None => {
                    ui.allocate_exact_size(tee, Sense::hover());
                }
            }
            // name and clan: 60/40 of the text width, long ones cut short
            for (s, width, weight, col) in [
                (&c.name, text_w * 0.6, Weight::Medium, color::TEXT),
                (&c.clan, text_w * 0.4, Weight::Regular, weak),
            ] {
                ui.allocate_ui_with_layout(egui::vec2(width, h), ltr, |ui| {
                    ui.set_min_width(width);
                    let t = RichText::new(s).font(font(size::TITLE, weight)).color(col);
                    ui.add(egui::Label::new(t).truncate());
                });
            }
            // C++: with cl_show_chat_friends only friends' chat shows anyway
            if v.show_chat_friends && !c.friend {
                icon_toggle(ui, icons::CHAT_OFF, true, false, "Ignore chat");
            } else if icon_toggle(ui, icons::CHAT_OFF, c.chat_ignore, true, "Ignore chat") {
                action = Some(Action::ToggleChatIgnore(id));
            }
            if icon_toggle(ui, icons::FRIEND, c.friend, true, "Friend") {
                action = Some(Action::ToggleFriend(id));
            }
        });
    }
    action
}

/// C++ `RenderServerInfo` (menus_ingame.cpp:622): Server and Game side by side, the MOTD
/// below.
fn info(ui: &mut Ui, v: &View) {
    let weak = v.glass.text_secondary();
    let empty = ServerInfo::default();
    let s = v.server.unwrap_or(&empty);
    let heading = |ui: &mut Ui, t: &str| w::text(ui, t, size::LARGE, Weight::SemiBold, color::TEXT);
    // label and value columns, every row the same gap apart
    let table = |ui: &mut Ui, id: &str, rows: &[(&str, String)]| {
        egui::Grid::new(id)
            .num_columns(2)
            .spacing([space::L, space::M])
            .show(ui, |ui| {
                for (label, value) in rows {
                    w::text(ui, label, size::TITLE, Weight::Regular, weak);
                    w::text(ui, value, size::TITLE, Weight::Medium, color::TEXT);
                    ui.end_row();
                }
            });
    };
    ui.columns(2, |cols| {
        let ui = &mut cols[0];
        ui.spacing_mut().item_spacing = egui::vec2(space::M, space::M);
        heading(ui, "Server");
        let mut rows = vec![("Name", s.name.clone()), ("Address", v.address.to_owned())];
        if let Some(ping) = local_latency(v.clients) {
            rows.push(("Ping", ping.to_string()));
        }
        rows.push(("Version", s.version.clone()));
        let password = if s.flags & FLAG_PASSWORD != 0 {
            "Yes"
        } else {
            "No"
        };
        rows.push(("Password", password.to_owned()));
        table(ui, "esc_info_server", &rows);
        // C++ "Copy info"
        ui.spacing_mut().button_padding = egui::vec2(space::L, space::S);
        ui.spacing_mut().icon_spacing = space::S;
        let copy = egui::Button::new((
            RichText::new(icons::COPY).font(icon_font(size::ICON_LARGE)),
            RichText::new("Copy info").font(font(size::TITLE, Weight::Medium)),
        ))
        .fill(color::SURFACE_RAISED)
        .corner_radius(CornerRadius::same(radius::PILL))
        .min_size(egui::vec2(0.0, size::CONTROL_LARGE));
        if ui.add(copy).clicked() {
            ui.ctx().copy_text(format!(
                "{}\nAddress: ddnet://{}\nMy IGN: {}\n",
                s.name, v.address, v.player_name
            ));
        }

        let ui = &mut cols[1];
        ui.spacing_mut().item_spacing = egui::vec2(space::M, space::M);
        heading(ui, "Game");
        let map = if s.map.is_empty() { v.map } else { &s.map };
        let mut rows = vec![("Game type", s.game_type.clone()), ("Map", map.to_owned())];
        if let Some(gi) = v.game_info {
            if gi.score_limit != 0 {
                rows.push(("Score limit", gi.score_limit.to_string()));
            }
            if gi.time_limit != 0 {
                rows.push(("Time limit", format!("{} min", gi.time_limit)));
            }
            if gi.round_current != 0 && gi.round_num != 0 {
                rows.push(("Round", format!("{}/{}", gi.round_current, gi.round_num)));
            }
        }
        table(ui, "esc_info_game", &rows);
    });
    if !v.motd.is_empty() {
        ui.separator();
        heading(ui, "Message of the day");
        ui.label(
            RichText::new(v.motd)
                .font(font(size::TITLE, Weight::Regular))
                .color(color::TEXT),
        );
    }
}

/// C++ `m_Snap.m_pLocalInfo->m_Latency`.
fn local_latency(c: &Clients) -> Option<i32> {
    let info = c.infos.get(usize::try_from(c.local_id).ok()?)?.as_ref()?;
    Some(info.latency)
}

/// C++: `CMotd::OnMessage` (motd.cpp:115): `\n` written as two characters becomes a line
/// break; at most the 900-byte buffer (C++ may cut a character there, we do not).
pub fn motd_text(message: &str) -> String {
    let s = message.replace("\\n", "\n");
    String::from_utf8_lossy(&ddnet::text::truncate(s.as_bytes(), 900)).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motd_line_breaks() {
        assert_eq!(motd_text("a\\nb\\\\n"), "a\nb\\\n");
    }
}
