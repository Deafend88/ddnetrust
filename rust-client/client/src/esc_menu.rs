//! The in-game menu (Esc, PLAN §13.5, C++ `menus_ingame.cpp`) in our design: a compact
//! bar across the top over the running game. Tabs as a segmented control, the Game tab's
//! actions as pills, Disconnect apart in muted red, icon buttons on the right; the other
//! tabs open a panel under the bar. Parts whose feature comes later stay hidden (Ghost
//! 13.8, Vote 13.7, Record demo 13.8, Dummy 13.6, Browser M11; no editor).

use egui::{Align, Align2, CornerRadius, Frame, Layout, Margin, RichText, Stroke, Ui};

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

/// A rounded rectangle of glass (the bar and the tab panels).
fn card<R>(ui: &mut Ui, glass: Glass, add: impl FnOnce(&mut Ui) -> R) -> R {
    w::card(ui, glass, space::M, add).inner
}

/// A pill button with an icon (`danger`: Disconnect's muted red).
fn pill_button(ui: &mut Ui, glyph: &str, label: &str, danger: bool) -> egui::Response {
    let (text, fill) = if danger {
        (color::DANGER, color::DANGER.gamma_multiply(0.14))
    } else {
        (color::TEXT, color::SURFACE_RAISED)
    };
    let b = egui::Button::new((
        RichText::new(glyph).font(icon_font(size::ICON)).color(text),
        RichText::new(label)
            .font(font(size::BODY, Weight::Medium))
            .color(text),
    ))
    .fill(fill)
    .corner_radius(CornerRadius::same(radius::PILL))
    .min_size(egui::vec2(0.0, size::CONTROL));
    ui.add(b)
}

/// A round icon button with a tooltip.
fn round_button(ui: &mut Ui, glyph: &str, tooltip: &str) -> egui::Response {
    let b = egui::Button::new(RichText::new(glyph).font(icon_font(size::ICON)))
        .fill(color::SURFACE_RAISED)
        .corner_radius(CornerRadius::same(radius::PILL))
        .min_size(egui::vec2(size::CONTROL, size::CONTROL));
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
    let b = egui::Button::new(RichText::new(glyph).font(icon_font(size::ICON)).color(c))
        .frame(false)
        .min_size(egui::vec2(size::CONTROL, size::CONTROL));
    ui.add_enabled(enabled, b)
        .on_hover_text(tooltip)
        .on_disabled_hover_text(tooltip)
        .clicked()
}

impl EscMenu {
    /// Draws the bar (and the open tab's panel) and returns what was chosen.
    pub fn show(&mut self, ctx: &egui::Context, v: &mut View) -> Option<Action> {
        let mut action = None;
        let width = ctx.content_rect().width() - 2.0 * space::L;
        let bottom = egui::Area::new(egui::Id::unique("esc_menu"))
            .anchor(Align2::CENTER_TOP, [0.0, space::L])
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing = egui::vec2(space::S, space::S);
                card(ui, v.glass, |ui| {
                    ui.set_width(width - 2.0 * space::M);
                    ui.horizontal(|ui| {
                        w::segmented(ui, &mut self.tab, &TABS);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
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
                    });
                    if self.tab == 0 {
                        ui.add_space(space::XS);
                        action = action.or(self.game_row(ui, v));
                    }
                });
                match self.tab {
                    TAB_PLAYERS => card(ui, v.glass, |ui| {
                        action = action.or(players(ui, v));
                    }),
                    TAB_INFO => card(ui, v.glass, |ui| info(ui, v)),
                    _ => {}
                }
            })
            .response
            .rect
            .bottom();
        // the hint is centred under everything, as wide as its words
        egui::Area::new(egui::Id::unique("esc_hint"))
            .anchor(Align2::CENTER_TOP, [0.0, bottom + space::S])
            .interactable(false)
            .show(ctx, |ui| esc_hint(ui, v.glass));
        action
    }
}

impl EscMenu {
    /// Called when the menu opens: no question left open from before.
    pub fn reset(&mut self) {
        self.confirming = false;
    }

    /// The Game tab's row: the C++ buttons as pills, a divider, Disconnect (with the
    /// C++ confirmation as Yes/No pills in its place).
    fn game_row(&mut self, ui: &mut Ui, v: &View) -> Option<Action> {
        let mut action = None;
        ui.horizontal(|ui| {
            for b in game_buttons(&v.game) {
                let (glyph, label, a) = match b {
                    GameButton::Spectate => (
                        icons::BINOCULARS,
                        "Spectate",
                        Action::SetTeam(g::TEAM_SPECTATORS),
                    ),
                    GameButton::JoinRed => (icons::USERS, "Join red", Action::SetTeam(g::TEAM_RED)),
                    GameButton::JoinBlue => {
                        (icons::USERS, "Join blue", Action::SetTeam(g::TEAM_BLUE))
                    }
                    GameButton::JoinGame => {
                        (icons::JOIN, "Join game", Action::SetTeam(g::TEAM_RED))
                    }
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
                w::text(ui, "Disconnect?", size::BODY, Weight::Medium, weak);
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
        });
        action
    }
}

/// The "Esc to close" hint under the bar: a key cap and the words.
fn esc_hint(ui: &mut Ui, glass: Glass) {
    w::glass_pill(ui, glass, false, |ui| {
        ui.spacing_mut().item_spacing.x = space::S;
        let weak = glass.text_secondary();
        Frame::new()
            .stroke(Stroke::new(1.0, weak))
            .corner_radius(CornerRadius::same(radius::COVER))
            .inner_margin(Margin::symmetric(space::XS as i8, 0))
            .show(ui, |ui| {
                w::text(ui, "Esc", size::CAPTION, Weight::Medium, weak)
            });
        w::text(ui, "to close", size::BODY, Weight::Regular, weak);
    });
}

/// C++ `RenderPlayers` (menus_ingame.cpp:492): every other player by name with chat
/// ignore and friend toggles.
fn players(ui: &mut Ui, v: &mut View) -> Option<Action> {
    let mut action = None;
    let local = v.clients.local_id;
    let ids: Vec<i32> = (v.clients.by_name.iter())
        .copied()
        .filter(|&id| id != local)
        .collect();
    if ids.is_empty() {
        let weak = v.glass.text_secondary();
        w::text(
            ui,
            "You are the only player here.",
            size::BODY,
            Weight::Regular,
            weak,
        );
        return None;
    }
    egui::ScrollArea::vertical()
        .max_height(ui.ctx().content_rect().height() / 2.0)
        .show(ui, |ui| {
            egui::Grid::new("esc_players")
                .num_columns(4)
                .spacing([space::L, space::XS])
                .show(ui, |ui| {
                    for id in ids {
                        let c = &v.clients.clients[id as usize];
                        let look = TeeLook {
                            skin: c.skin.clone(),
                            custom_color: c.use_custom_color != 0,
                            color_body: c.color_body,
                            color_feet: c.color_feet,
                        };
                        let ctx = ui.ctx().clone();
                        match v.heads.get(&ctx, &look) {
                            Some(t) => {
                                let img =
                                    egui::Image::new((t.id(), egui::vec2(size::TEE, size::TEE)));
                                ui.add(img).on_hover_text(&c.skin);
                            }
                            None => {
                                ui.label("");
                            }
                        }
                        ui.horizontal(|ui| {
                            w::text(ui, &c.name, size::BODY, Weight::Medium, color::TEXT);
                            if !c.clan.is_empty() {
                                let weak = v.glass.text_secondary();
                                w::text(ui, &c.clan, size::BODY, Weight::Regular, weak);
                            }
                        });
                        // C++: with cl_show_chat_friends only friends' chat shows anyway
                        if v.show_chat_friends && !c.friend {
                            icon_toggle(ui, icons::CHAT_OFF, true, false, "Ignore chat");
                        } else if icon_toggle(
                            ui,
                            icons::CHAT_OFF,
                            c.chat_ignore,
                            true,
                            "Ignore chat",
                        ) {
                            action = Some(Action::ToggleChatIgnore(id));
                        }
                        if icon_toggle(ui, icons::FRIEND, c.friend, true, "Friend") {
                            action = Some(Action::ToggleFriend(id));
                        }
                        ui.end_row();
                    }
                });
        });
    action
}

/// C++ `RenderServerInfo` (menus_ingame.cpp:622): server, game, MOTD.
fn info(ui: &mut Ui, v: &View) {
    let weak = v.glass.text_secondary();
    // label rows as tall as their text, like the plain lines
    ui.spacing_mut().interact_size.y = 0.0;
    let line = |ui: &mut Ui, label: &str, value: &str| {
        ui.horizontal(|ui| {
            w::text(ui, label, size::BODY, Weight::Regular, weak);
            w::text(ui, value, size::BODY, Weight::Medium, color::TEXT);
        });
    };
    let empty = ServerInfo::default();
    let s = v.server.unwrap_or(&empty);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            w::text(ui, "Server", size::TITLE, Weight::SemiBold, color::TEXT);
            w::text(ui, &s.name, size::BODY, Weight::Medium, color::TEXT);
            line(ui, "Address", v.address);
            if let Some(ping) = local_latency(v.clients) {
                line(ui, "Ping", &ping.to_string());
            }
            line(ui, "Version", &s.version);
            let password = if s.flags & FLAG_PASSWORD != 0 {
                "Yes"
            } else {
                "No"
            };
            line(ui, "Password", password);
            ui.add_space(space::S);
            // C++ "Copy info"
            if w::button(ui, Some(icons::COPY), "Copy info").clicked() {
                ui.ctx().copy_text(format!(
                    "{}\nAddress: ddnet://{}\nMy IGN: {}\n",
                    s.name, v.address, v.player_name
                ));
            }
        });
        ui.add_space(space::XXL);
        ui.vertical(|ui| {
            w::text(ui, "Game", size::TITLE, Weight::SemiBold, color::TEXT);
            line(ui, "Game type", &s.game_type);
            line(ui, "Map", if s.map.is_empty() { v.map } else { &s.map });
            if let Some(gi) = v.game_info {
                if gi.score_limit != 0 {
                    line(ui, "Score limit", &gi.score_limit.to_string());
                }
                if gi.time_limit != 0 {
                    line(ui, "Time limit", &format!("{} min", gi.time_limit));
                }
                if gi.round_current != 0 && gi.round_num != 0 {
                    line(
                        ui,
                        "Round",
                        &format!("{}/{}", gi.round_current, gi.round_num),
                    );
                }
            }
        });
    });
    if !v.motd.is_empty() {
        ui.add_space(space::M);
        w::text(
            ui,
            "Message of the day",
            size::TITLE,
            Weight::SemiBold,
            color::TEXT,
        );
        ui.label(
            RichText::new(v.motd)
                .font(font(size::BODY, Weight::Regular))
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
