//! Scoreboard (PLAN §13.2). The logic of C++ `CScoreboard`
//! (game/client/components/scoreboard.cpp): when it shows (`IsActive`), the team names
//! (`GetTeamName`), the rows in `m_apInfoByDDTeamScore` order with their DDRace team
//! groups and labels, the score and time texts; the player order itself is
//! `ddnet::clients`. The drawing is ours (`show`). Checked against copies of the C++ code
//! by `rs_parity scoreboard` (tests/scoreboard.rs).

use ddnet::clients::{Clients, NOT_FINISHED_MILLIS, NOT_FINISHED_TIMESCORE};

use crate::timefmt::{TimeFormat, str_time};

pub use ddnet::physics::teams::TEAM_SUPER;

/// What `CScoreboard::IsActive` reads.
#[derive(Clone, Copy, Debug, Default)]
pub struct ActiveState {
    /// C++ `m_Active` (`+scoreboard` held).
    pub held: bool,
    /// C++ `m_Statboard.IsActive()`.
    pub statboard: bool,
    /// C++ `m_Snap.m_pLocalInfo` present.
    pub local_info: bool,
    /// C++ `m_Snap.m_SpecInfo.m_Active`.
    pub spectating: bool,
    /// C++ `m_Snap.m_pLocalCharacter` present.
    pub local_character: bool,
    /// C++ `cl_scoreboard_on_death`.
    pub on_death: bool,
    /// The game info's `m_GameStateFlags`, if there is a game info.
    pub game_state_flags: Option<i32>,
}

/// C++: `CScoreboard::IsActive` (scoreboard.cpp:1206).
pub fn is_active(s: &ActiveState) -> bool {
    use ddnet::protocol::generated::{GAMESTATEFLAG_GAMEOVER, GAMESTATEFLAG_PAUSED};
    if s.statboard {
        return false;
    }
    if s.held {
        return true;
    }
    let flags = s.game_state_flags;
    // we are not a spectator, check if we are dead and the game isn't paused
    if s.local_info
        && !s.spectating
        && !s.local_character
        && s.on_death
        && !flags.is_some_and(|f| f & GAMESTATEFLAG_PAUSED != 0)
    {
        return true;
    }
    // if the game is over
    flags.is_some_and(|f| f & GAMESTATEFLAG_GAMEOVER != 0)
}

/// C++: `CScoreboard::GetTeamName` (scoreboard.cpp:1231): the clan all players of `team`
/// share, if more than one plays and it is not empty.
pub fn team_name(clients: &Clients, team: i32) -> Option<&str> {
    let mut clan: Option<&str> = None;
    let mut players = 0;
    for &id in &clients.by_score {
        let Some(info) = clients.infos[id as usize] else {
            continue;
        };
        if info.team != team {
            continue;
        }
        let c = clients.clients[id as usize].clan.as_str();
        match clan {
            None => {
                clan = Some(c);
                players += 1;
            }
            Some(first) if first == c => players += 1,
            Some(_) => return None,
        }
    }
    clan.filter(|c| players > 1 && !c.is_empty())
}

/// One scoreboard row: a player, its DDRace team and whether a team group ends here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: i32,
    pub ddteam: i32,
    /// At a group's last row (C++: the next row is another team), the number of its
    /// rows, for its label (`label`).
    pub group_size: Option<i32>,
    /// The row begins a DDRace team group.
    pub group_start: bool,
}

/// C++: the row walk of `RenderScoreboard` (scoreboard.cpp:614-710) for game team `team`
/// (`TEAM_RED` 0 for a game without teams): rows `start..end` of the players of that
/// team in `m_apInfoByDDTeamScore` order, with DDRace team groups and labels.
/// `group_size`: C++ `CScoreboardRenderState::m_CurrentDDTeamSize`, which counts only
/// drawn rows and carries over from one column (call) to the next.
pub fn rows(
    clients: &Clients,
    order: &[i32],
    team: i32,
    ddteam: &dyn Fn(i32) -> i32,
    start: usize,
    end: usize,
    group_size: &mut i32,
) -> Vec<Row> {
    let in_team: Vec<i32> = (order.iter().copied())
        .filter(|&id| clients.infos[id as usize].is_some_and(|p| p.team == team))
        .collect();
    let mut rows = Vec::new();
    for (n, &id) in in_team.iter().enumerate().skip(start) {
        let t = ddteam(id);
        let prev = n.checked_sub(1).map(|p| ddteam(in_team[p]));
        let next = in_team.get(n + 1).map_or(0, |&j| ddteam(j));
        let mut size = None;
        if t != 0 {
            *group_size += 1;
            if next != t {
                size = Some(*group_size);
                *group_size = 0;
            }
        }
        rows.push(Row {
            id,
            ddteam: t,
            group_size: size,
            group_start: t != 0 && prev != Some(t),
        });
        if n + 1 == end {
            break;
        }
    }
    rows
}

/// C++: a group's label (the `NumPlayers <= 8` texts; the compact layout's `3` and
/// `(2/4)` on two lines say the same): `Super`, `Team 3` or `Team 3 (2/4)`.
pub fn label(ddteam: i32, size: i32, max_team_size: i32) -> String {
    if ddteam == TEAM_SUPER {
        "Super".into()
    } else if size > 1 {
        format!("Team {ddteam} ({size}/{max_team_size})")
    } else {
        format!("Team {ddteam}")
    }
}

/// The score column: a time `(seconds, fraction)` for race scores, else the score.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Score {
    /// C++ `Ui()->RenderTime`: `str_time(|s| * 100, HOURS)` and, under an hour,
    /// `.%02d`/`.%03d` milliseconds; nothing for an unfinished player.
    Time(String, Option<String>),
    Points(String),
    None,
}

/// C++: `CUi::RenderTime`'s texts (ui.cpp:1680).
fn render_time(seconds: i32, not_finished: bool, millis: i32, true_millis: bool) -> Score {
    if not_finished {
        return Score::None;
    }
    let whole = str_time((seconds as i64).abs() * 100, TimeFormat::Hours);
    let fraction = (millis >= 0 && seconds < 60 * 60).then(|| {
        let millis = millis % 1000;
        if true_millis {
            format!(".{millis:03}")
        } else {
            // C++ std::round(Millis / 10): the integer division already rounded down
            format!(".{:02}", millis / 10)
        }
    });
    Score::Time(whole, fraction)
}

/// C++: the score column of `RenderScoreboard` (scoreboard.cpp:768-796), without the 0.7
/// race branch.
pub fn score(clients: &Clients, id: i32, time_score: bool) -> Score {
    let (Some(info), c) = (clients.infos[id as usize], &clients.clients[id as usize]) else {
        return Score::None;
    };
    if clients.received_finish_times {
        render_time(
            c.finish_time_seconds,
            c.finish_time_seconds == NOT_FINISHED_MILLIS,
            c.finish_time_millis,
            clients.received_finish_times_millis,
        )
    } else if time_score {
        render_time(info.score, info.score == NOT_FINISHED_TIMESCORE, -1, false)
    } else {
        Score::Points(info.score.clamp(-999, 99999).to_string())
    }
}

/// C++: `RenderTitleScore`'s race branch: the map's best time (`m_MapBestTimeSeconds`),
/// if the server sent one.
pub fn map_best(clients: &Clients, time_score: bool) -> Score {
    if !(clients.received_finish_times || time_score)
        || clients.map_best.0 == ddnet::clients::FINISH_UNSET
    {
        return Score::None;
    }
    let (s, ms) = clients.map_best;
    render_time(
        s,
        s == NOT_FINISHED_MILLIS,
        ms,
        clients.received_finish_times_millis,
    )
}

/// What the scoreboard view reads besides the client table.
pub struct View<'a> {
    pub glass: crate::style::Glass,
    pub clients: &'a Clients,
    /// C++ `m_Teams.Team(id)` and `MaxTeamSize()`.
    pub ddteam: &'a dyn Fn(i32) -> i32,
    pub max_team_size: i32,
    pub time_score: bool,
    /// The snapshot's game info and game data.
    pub game_info: Option<ddnet::protocol::generated::ObjGameInfo>,
    pub game_data: Option<ddnet::protocol::generated::ObjGameData>,
    /// C++ `Map()->BaseName()`.
    pub map: &'a str,
    /// The highlighted row: the local player, or the followed one while spectating.
    pub highlight: i32,
    pub heads: &'a mut crate::teehead::TeeHeads,
    /// C++ `cl_enable_ping_color`.
    pub ping_color: bool,
}

mod view {
    use egui::{Align2, Color32, CornerRadius, Margin, Pos2, Rect, Ui, pos2, vec2};

    use super::*;
    use crate::nameplates::dd_team_color;
    use crate::style::{Weight, color, font, radius, size, space};
    use crate::widgets as w;
    use ddnet::protocol::generated::{GAMEFLAG_TEAMS, GAMESTATEFLAG_GAMEOVER};

    /// C++ `TEAM_RED_TITLE_COLOR`, `TEAM_BLUE_TITLE_COLOR`.
    const TEAM_COLORS: [[f32; 3]; 2] = [[0.975, 0.17, 0.17], [0.17, 0.46, 0.975]];

    fn rgb([r, g, b]: [f32; 3]) -> Color32 {
        let c = |f: f32| (f.clamp(0.0, 1.0) * 255.0).round() as u8;
        Color32::from_rgb(c(r), c(g), c(b))
    }

    /// Row height and type size: roomy up to 16 rows per column, compact above.
    fn density(rows: usize) -> (f32, f32) {
        if rows <= 16 {
            (space::XXL, size::BODY)
        } else {
            (space::XL, size::CAPTION)
        }
    }

    fn text(
        ui: &Ui,
        at: Pos2,
        align: Align2,
        s: &str,
        size: f32,
        weight: Weight,
        c: Color32,
    ) -> Rect {
        ui.painter().text(at, align, s, font(size, weight), c)
    }

    /// The score column's text: the time (fraction smaller and secondary) or the points.
    fn score_cell(ui: &Ui, right: Pos2, s: &Score, size: f32) {
        match s {
            Score::Time(whole, fraction) => {
                let mut x = right.x;
                if let Some(f) = fraction {
                    let r = text(
                        ui,
                        pos2(x, right.y),
                        Align2::RIGHT_CENTER,
                        f,
                        size * 0.8,
                        Weight::Regular,
                        color::TEXT_SECONDARY,
                    );
                    x = r.left();
                }
                text(
                    ui,
                    pos2(x, right.y),
                    Align2::RIGHT_CENTER,
                    whole,
                    size,
                    Weight::Medium,
                    color::TEXT,
                );
            }
            Score::Points(p) => {
                text(
                    ui,
                    right,
                    Align2::RIGHT_CENTER,
                    p,
                    size,
                    Weight::Medium,
                    color::TEXT,
                );
            }
            Score::None => {}
        }
    }

    /// One column of rows (C++ `RenderScoreboard`): score, tee, name, clan, ping.
    #[allow(clippy::too_many_arguments)] // one column of the C++ layout
    fn column(
        ui: &mut Ui,
        v: &mut View,
        team: i32,
        start: usize,
        end: usize,
        width: f32,
        rows_for_size: usize,
        group: &mut i32,
    ) {
        let (row_h, size) = density(rows_for_size);
        let order = Clients::by_ddteam(&v.clients.by_score, v.ddteam);
        let rows = rows(v.clients, &order, team, v.ddteam, start, end, group);
        let header_h = space::XL;
        let label_h = size::CAPTION + space::XS;
        let labels = rows.iter().filter(|r| r.group_size.is_some()).count() as f32;
        let height = header_h + rows.len() as f32 * row_h + labels * label_h;
        let (rect, _) = ui.allocate_exact_size(vec2(width, height), egui::Sense::hover());
        let score_w = if v.clients.received_finish_times || v.time_score {
            size * 5.2
        } else {
            size * 3.2
        };
        let (score_r, tee_x) = (rect.left() + score_w, rect.left() + score_w + space::M);
        let name_x = tee_x + size::ICON_LARGE + space::S;
        let ping_r = rect.right();
        let clan_c = rect.left() + width * 0.68;
        // C++ headlines: Time/Score, Name, Clan, Ping
        let y = rect.top() + header_h / 2.0;
        let weak = color::TEXT_SECONDARY;
        let race = v.clients.received_finish_times || v.time_score;
        let cap = size::CAPTION;
        text(
            ui,
            pos2(score_r, y),
            Align2::RIGHT_CENTER,
            if race { "Time" } else { "Score" },
            cap,
            Weight::Medium,
            weak,
        );
        text(
            ui,
            pos2(name_x, y),
            Align2::LEFT_CENTER,
            "Name",
            cap,
            Weight::Medium,
            weak,
        );
        text(
            ui,
            pos2(clan_c, y),
            Align2::CENTER_CENTER,
            "Clan",
            cap,
            Weight::Medium,
            weak,
        );
        text(
            ui,
            pos2(ping_r, y),
            Align2::RIGHT_CENTER,
            "Ping",
            cap,
            Weight::Medium,
            weak,
        );

        let mut y = rect.top() + header_h;
        let mut group_top = None;
        for r in &rows {
            let row = Rect::from_min_size(pos2(rect.left(), y), vec2(width, row_h));
            if r.ddteam != 0 && (r.group_start || group_top.is_none()) {
                // the group's tint goes behind its rows (filled in at its end)
                group_top = Some((y, ui.painter().add(egui::Shape::Noop)));
            }
            if r.id == v.highlight {
                let fill = color::SURFACE_HOVER.gamma_multiply(0.9);
                ui.painter().rect_filled(
                    row.expand2(vec2(space::XS, 0.0)),
                    CornerRadius::same(radius::SMALL),
                    fill,
                );
            }
            let c = &v.clients.clients[r.id as usize];
            let info = v.clients.infos[r.id as usize].unwrap_or_default();
            let mid = row.center().y;
            score_cell(
                ui,
                pos2(score_r, mid),
                &score(v.clients, r.id, v.time_score),
                size,
            );
            let look = crate::teehead::TeeLook {
                skin: c.skin.clone(),
                custom_color: c.use_custom_color != 0,
                color_body: c.color_body,
                color_feet: c.color_feet,
            };
            let ctx = ui.ctx().clone();
            if let Some(t) = v.heads.get(&ctx, &look) {
                let s = size::TEE.min(row_h - space::XS);
                let r = Rect::from_center_size(pos2(tee_x + s / 2.0, mid), vec2(s, s));
                ui.painter().image(
                    t.id(),
                    r,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            let name_w = clan_c - name_x - width * 0.12;
            let name = ui.fonts_mut(|f| {
                let mut job = egui::text::LayoutJob::simple_singleline(
                    c.name.clone(),
                    font(size, Weight::Medium),
                    color::TEXT,
                );
                job.wrap = egui::text::TextWrapping::truncate_at_width(name_w);
                f.layout_job(job)
            });
            ui.painter()
                .galley(pos2(name_x, mid - name.size().y / 2.0), name, color::TEXT);
            text(
                ui,
                pos2(clan_c, mid),
                Align2::CENTER_CENTER,
                &c.clan,
                size,
                Weight::Regular,
                weak,
            );
            // C++: the latency clamped to 0..999
            let ping = info.latency.clamp(0, 999).to_string();
            // C++: CRenderTools::GetPingTextColor (render.cpp:596), green 0 to red 300
            let ping_color = if v.ping_color {
                let hue = (300.0 - info.latency.clamp(0, 300) as f32) / 1000.0;
                rgb(crate::render::map::hsl_to_rgb([hue, 1.0, 0.5]))
            } else {
                weak
            };
            text(
                ui,
                pos2(ping_r, mid),
                Align2::RIGHT_CENTER,
                &ping,
                size,
                Weight::Regular,
                ping_color,
            );
            y += row_h;
            if let Some(n) = r.group_size
                && let Some((top, at)) = group_top.take()
            {
                // the DDRace team: a pastel tint of its C++ hue behind its rows (white
                // text stays readable), and its label under them
                let tint = rgb(dd_team_color(r.ddteam, 0.8));
                let group =
                    Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), y + label_h))
                        .expand2(vec2(space::XS, 0.0));
                let shape = egui::Shape::rect_filled(
                    group,
                    CornerRadius::same(radius::SMALL),
                    tint.gamma_multiply(0.24),
                );
                ui.painter().set(at, shape);
                let l = label(r.ddteam, n, v.max_team_size);
                text(
                    ui,
                    pos2(rect.right(), y + label_h / 2.0),
                    Align2::RIGHT_CENTER,
                    &l,
                    size::CAPTION,
                    Weight::Medium,
                    color::TEXT.lerp_to_gamma(tint, 0.5),
                );
                y += label_h;
            }
        }
    }

    /// C++: the column split of `OnRender` (scoreboard.cpp:1141-1173).
    fn split(players: usize) -> Vec<(usize, usize)> {
        if players <= 16 {
            vec![(0, players)]
        } else if players <= 64 {
            let side = match players {
                17..=24 => 12,
                25..=32 => 16,
                33..=48 => 24,
                _ => 32,
            };
            vec![(0, side), (side, 2 * side)]
        } else {
            let per = 43; // C++ std::ceil(128.0f / 3)
            vec![(0, per), (per, 2 * per), (2 * per, 3 * per)]
        }
    }

    /// The title row: title left, the map's best time or the score right.
    fn title(ui: &mut Ui, title: &str, right: &Score, dot: Option<Color32>, width: f32) {
        ui.horizontal(|ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.x = space::S;
            if let Some(c) = dot {
                let (r, _) = ui.allocate_exact_size(vec2(space::S, space::S), egui::Sense::hover());
                ui.painter().circle_filled(r.center(), space::S / 2.0, c);
            }
            w::text(ui, title, size::TITLE, Weight::SemiBold, color::TEXT);
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| match right {
                    Score::Time(whole, fraction) => {
                        w::text(
                            ui,
                            &format!("{whole}{}", fraction.as_deref().unwrap_or("")),
                            size::LABEL,
                            Weight::Medium,
                            color::TEXT,
                        );
                        w::icon(ui, crate::icons::TROPHY, size::ICON, color::TEXT_SECONDARY);
                    }
                    Score::Points(p) => {
                        w::text(ui, p, size::TITLE, Weight::SemiBold, color::TEXT);
                    }
                    Score::None => {}
                },
            );
        });
    }

    /// Draws the scoreboard (C++ `CScoreboard::OnRender`, our look): one rounded panel
    /// centred near the top (two for team games), then the goals and the spectators.
    pub fn show(ctx: &egui::Context, v: &mut View) {
        let info = v.game_info;
        let teams = info.is_some_and(|i| i.game_flags & GAMEFLAG_TEAMS != 0);
        let over = info.is_some_and(|i| i.game_state_flags & GAMESTATEFLAG_GAMEOVER != 0);
        let in_team = |t: i32| {
            (v.clients.infos.iter())
                .filter(|p| p.is_some_and(|p| p.team == t))
                .count()
        };
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::unique("scoreboard"))
            .order(egui::Order::Foreground)
            .anchor(
                Align2::CENTER_TOP,
                vec2(0.0, screen.height() * 75.0 / 600.0),
            )
            .interactable(false)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = vec2(space::M, space::M);
                ui.vertical_centered(|ui| {
                    if teams {
                        if over && let Some(d) = v.game_data {
                            // C++: "Red team wins!" / "Blue team wins!" / "Draw!"
                            let names = [team_name(v.clients, 0), team_name(v.clients, 1)];
                            let won = |t: usize, fallback: &str| {
                                names[t].map_or(fallback.to_string(), |n| format!("{n} wins!"))
                            };
                            let t = match d.teamscore_red.cmp(&d.teamscore_blue) {
                                std::cmp::Ordering::Greater => won(0, "Red team wins!"),
                                std::cmp::Ordering::Less => won(1, "Blue team wins!"),
                                std::cmp::Ordering::Equal => "Draw!".into(),
                            };
                            w::text(ui, &t, size::LARGE, Weight::SemiBold, color::TEXT);
                        }
                        ui.horizontal(|ui| {
                            let mut group = 0;
                            for team in 0..2 {
                                let n = in_team(team);
                                let width = 560.0;
                                w::hud_bar(
                                    ui,
                                    v.glass,
                                    radius::CARD,
                                    Margin::same(space::L as i8),
                                    0.0,
                                    |ui| {
                                        ui.vertical(|ui| {
                                            let name =
                                                team_name(v.clients, team).map(str::to_string);
                                            let fallback = ["Red team", "Blue team"][team as usize];
                                            let score = v.game_data.map_or(Score::None, |d| {
                                                Score::Points(
                                                    [d.teamscore_red, d.teamscore_blue]
                                                        [team as usize]
                                                        .to_string(),
                                                )
                                            });
                                            title(
                                                ui,
                                                name.as_deref().unwrap_or(fallback),
                                                &score,
                                                Some(rgb(TEAM_COLORS[team as usize])),
                                                width,
                                            );
                                            column(
                                                ui,
                                                v,
                                                team,
                                                0,
                                                n.min(64),
                                                width,
                                                n.min(64),
                                                &mut group,
                                            );
                                        });
                                    },
                                );
                            }
                        });
                    } else {
                        let n = in_team(0);
                        let parts = split(n);
                        let column_w = 520.0;
                        let width =
                            column_w * parts.len() as f32 + space::L * (parts.len() - 1) as f32;
                        w::hud_bar(
                            ui,
                            v.glass,
                            radius::CARD,
                            Margin::same(space::L as i8),
                            0.0,
                            |ui| {
                                ui.vertical(|ui| {
                                    let name = if over { "Game over" } else { v.map };
                                    // C++: the map's best time in race games, else our score
                                    let right = if v.clients.received_finish_times || v.time_score {
                                        map_best(v.clients, v.time_score)
                                    } else {
                                        let id = v.highlight;
                                        (v.clients.infos.get(id.max(0) as usize).copied().flatten())
                                            .map_or(Score::None, |p| {
                                                Score::Points(p.score.to_string())
                                            })
                                    };
                                    title(ui, name, &right, None, width);
                                    ui.horizontal_top(|ui| {
                                        ui.spacing_mut().item_spacing.x = space::L;
                                        let mut group = 0;
                                        let rows_for_size = parts.first().map_or(0, |p| p.1 - p.0);
                                        for &(start, end) in &parts {
                                            column(
                                                ui,
                                                v,
                                                0,
                                                start,
                                                end,
                                                column_w,
                                                rows_for_size,
                                                &mut group,
                                            );
                                        }
                                    });
                                });
                            },
                        );
                    }
                    // C++ RenderGoals: score limit, time limit, round
                    if let Some(i) = info {
                        let mut goals = Vec::new();
                        if i.score_limit != 0 {
                            goals.push(format!("Score limit: {}", i.score_limit));
                        }
                        if i.time_limit != 0 {
                            goals.push(format!("Time limit: {} min", i.time_limit));
                        }
                        if i.round_num != 0 && i.round_current != 0 {
                            goals.push(format!("Round {}/{}", i.round_current, i.round_num));
                        }
                        if !goals.is_empty() {
                            ui.horizontal(|ui| {
                                for g in &goals {
                                    let m = Margin::symmetric(space::M as i8, space::XS as i8);
                                    w::hud_bar(ui, v.glass, radius::PILL, m, size::ICON, |ui| {
                                        w::text(
                                            ui,
                                            g,
                                            size::CAPTION,
                                            Weight::Medium,
                                            color::TEXT_SECONDARY,
                                        );
                                    });
                                }
                            });
                        }
                    }
                    // C++ RenderSpectators: "Spectators: a, b, c"
                    let spectators: Vec<&str> = (v.clients.by_name.iter())
                        .filter(|&&id| v.clients.infos[id as usize].is_some_and(|p| p.team == -1))
                        .map(|&id| v.clients.clients[id as usize].name.as_str())
                        .collect();
                    if !spectators.is_empty() {
                        let m = Margin::symmetric(space::L as i8, space::S as i8);
                        let shape = if spectators.len() > 6 {
                            radius::SMALL
                        } else {
                            radius::PILL
                        };
                        w::hud_bar(ui, v.glass, shape, m, size::ICON, |ui| {
                            ui.set_max_width(520.0);
                            ui.spacing_mut().item_spacing.x = space::S;
                            w::icon(
                                ui,
                                crate::icons::EYE,
                                size::ICON_SMALL,
                                color::TEXT_SECONDARY,
                            );
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(spectators.join(", "))
                                        .font(font(size::CAPTION, Weight::Regular))
                                        .color(color::TEXT_SECONDARY),
                                )
                                .wrap(),
                            );
                        });
                    }
                });
            });
    }
}

pub use view::show;
