//! PLAN §13.2: the scoreboard, name plate and freeze bar logic against the C++ code.
//! `scoreboard.txt` comes from `rs_parity scoreboard`, which runs word-for-word copies of
//! the player sorts, `GetScoreComparator`, `CScoreboard::IsActive`/`GetTeamName`, the row
//! walk of `RenderScoreboard`, `RenderNamePlateGame`/`OnRender` and
//! `RenderFreezeBar`/`OnRender` on 300 random games (src/tools/rs_parity_scoreboard.cpp).

use ddnet::clients::{Clients, PlayerInfo};
use ddnet::physics::core::CharacterCore;
use ddnet::physics::vmath::vec2;
use ddnet_rs::freezebars::{self, BarConfig, Tee};
use ddnet_rs::nameplates::{self, CharView, PlateConfig, StrongWeak};
use ddnet_rs::scoreboard::{self, ActiveState, TEAM_SUPER};

fn golden(name: &str) -> String {
    let path = format!("{}/../tests/golden/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(path).unwrap()
}

fn unhex(s: &str) -> String {
    if s == "-" {
        return String::new();
    }
    let b: Vec<u8> = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect();
    String::from_utf8(b).unwrap()
}

fn hex(s: &str) -> String {
    if s.is_empty() {
        return "-".into();
    }
    s.bytes().map(|x| format!("{x:02x}")).collect()
}

fn float(h: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(h, 16).unwrap())
}

fn bits(f: f32) -> String {
    format!("{:08x}", f.to_bits())
}

fn ints(w: &[&str]) -> Vec<i32> {
    w.iter().map(|s| s.parse().unwrap()).collect()
}

#[test]
fn scoreboard_matches_cpp() {
    let text = golden("scoreboard.txt");
    let mut lines = text.lines().peekable();
    let runs: usize = lines
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let (mut rows_checked, mut plates_checked, mut bars_checked) = (0, 0, 0);
    for _ in 0..runs {
        let h = ints(&lines.next().unwrap().split(' ').skip(2).collect::<Vec<_>>());
        let (time_score, finish, team_play, max_team_size) =
            (h[0] != 0, h[1] != 0, h[2] != 0, h[3]);
        let mut clients = Clients::default();
        let mut ddteams = [0; 128];
        let mut other = [false; 128];
        let mut ids = Vec::new();
        while lines.peek().unwrap().starts_with("player ") {
            let w: Vec<&str> = lines.next().unwrap().split(' ').collect();
            let id: usize = w[1].parse().unwrap();
            let v = ints(&[w[2], w[3], w[4], w[7], w[8], w[9], w[10], w[11]]);
            clients.infos[id] = Some(PlayerInfo {
                local: false,
                client_id: id as i32,
                team: v[0],
                score: v[1],
                latency: v[2],
            });
            let c = &mut clients.clients[id];
            (c.name, c.clan, c.team) = (unhex(w[5]), unhex(w[6]), v[0]);
            (c.finish_time_seconds, c.finish_time_millis) = (v[3], v[4]);
            c.active = true;
            c.friend = v[6] != 0;
            (ddteams[id], other[id]) = (v[5], v[7] != 0);
            ids.push(id as i32);
        }
        let local: i32 = lines
            .next()
            .unwrap()
            .strip_prefix("local ")
            .unwrap()
            .parse()
            .unwrap();
        clients.local_id = local;
        if local >= 0 {
            clients.infos[local as usize].as_mut().unwrap().local = true;
        }
        clients.received_finish_times = finish;
        clients.sort(time_score);
        let team = |id: i32| ddteams[id as usize];
        let ddscore = Clients::by_ddteam(&clients.by_score, team);
        let ddname = Clients::by_ddteam(&clients.by_name, team);
        let list = |l: &[i32]| l.iter().map(|i| format!(" {i}")).collect::<String>();
        let got = format!(
            "sort name{} | score{} | ddscore{} | ddname{} |",
            list(&clients.by_name),
            list(&clients.by_score),
            list(&ddscore),
            list(&ddname)
        );
        assert_eq!(got, lines.next().unwrap(), "sort");

        for _ in 0..4 {
            let l = lines.next().unwrap();
            let (a, want) = l
                .strip_prefix("active ")
                .unwrap()
                .split_once(" = ")
                .unwrap();
            let v = ints(&a.split(' ').collect::<Vec<_>>());
            let s = ActiveState {
                held: v[0] != 0,
                statboard: v[1] != 0,
                local_info: v[2] != 0,
                spectating: v[3] != 0,
                local_character: v[4] != 0,
                on_death: v[5] != 0,
                game_state_flags: (v[6] != 0).then_some(v[7]),
            };
            assert_eq!(
                scoreboard::is_active(&s) as i32,
                want.parse::<i32>().unwrap(),
                "{l}"
            );
        }
        for t in 0..2 {
            let got = scoreboard::team_name(&clients, t).map_or("null".into(), hex);
            assert_eq!(format!("teamname {t} {got}"), lines.next().unwrap());
        }
        for _ in 0..(if team_play { 2 } else { 1 }) {
            let l = lines.next().unwrap();
            let (a, want) = l.strip_prefix("rows ").unwrap().split_once(" =").unwrap();
            let v = ints(&a.split(' ').collect::<Vec<_>>());
            let (t, start, end, size) = (v[0], v[1] as usize, v[2] as usize, v[3]);
            let mut got = String::new();
            let mut group = 0;
            for r in scoreboard::rows(&clients, &ddscore, t, &team, start, end, &mut group) {
                got += " R";
                if r.ddteam == 0 {
                    continue;
                }
                // C++ CORNER_T 3, CORNER_B 12
                let corners =
                    if r.group_start { 3 } else { 0 } | if r.group_size.is_some() { 12 } else { 0 };
                got += &format!(" C:{corners}");
                if let Some(n) = r.group_size {
                    // C++: the compact text for more than 8 players
                    let label = if size > 8 && r.ddteam != TEAM_SUPER {
                        if n <= 1 {
                            r.ddteam.to_string()
                        } else {
                            format!("{}\n({n}/{max_team_size})", r.ddteam)
                        }
                    } else {
                        scoreboard::label(r.ddteam, n, max_team_size)
                    };
                    got += &format!(" L:{}", hex(&label));
                }
            }
            assert_eq!(got, want, "{l}");
            rows_checked += 1;
        }

        // name plates
        for _ in 0..3 {
            let w: Vec<&str> = lines.next().unwrap().split(' ').collect();
            let v = ints(&w[1..19]);
            let config = PlateConfig {
                name_plates: v[0] != 0,
                own: v[1] != 0,
                friend_mark: v[2] != 0,
                ids: v[3] != 0,
                size: v[4],
                ids_separate_line: v[5] != 0,
                ids_size: v[6],
                clan: v[7] != 0,
                clan_size: v[8],
                strong: v[9],
                strong_size: v[10],
                direction_size: v[11],
                always: v[12] != 0,
                offset: v[13],
                show_others_alpha: v[14],
                teamcolors: v[15] != 0,
                show_direction: v[16],
                debug: v[17] != 0,
            };
            let screen = vec2(float(w[19]), float(w[20]));
            let target = vec2(float(w[21]), float(w[22]));
            let r = ints(&w[23..30]);
            let mut chars = vec![CharView::default(); 128];
            let mut pos = vec![vec2(0.0, 0.0); 128];
            clients.spec_chars.fill(None);
            loop {
                let l = lines.next().unwrap();
                if l == "platesout" {
                    break;
                }
                let c: Vec<&str> = l.split(' ').collect();
                let id: usize = c[1].parse().unwrap();
                let v = ints(&c[2..8]);
                chars[id] = CharView {
                    active: v[0] != 0,
                    direction: v[1],
                    jumped: v[2],
                    strong_weak_id: (v[3] != 0).then_some(v[4]),
                };
                if v[5] != 0 {
                    clients.spec_chars[id] = Some((float(c[8]), float(c[9])));
                }
                pos[id] = vec2(float(c[10]), float(c[11]));
            }
            let other_team = |id: i32| other[id as usize];
            let ctx = nameplates::Ctx {
                screen: (screen, screen + vec2(1200.0, 700.0)),
                clients: &clients,
                is_other_team: &other_team,
                team: &team,
                team_play,
                target_pos: target,
                local_ids: [local, r[4]],
                dummy: 0,
                inputs: [(r[0], r[1]), (r[2], r[3])],
                chars: &chars,
                spec: (r[5] != 0, r[6]),
                config,
            };
            for p in nameplates::plates(&ctx, &|id| pos[id as usize]) {
                let sw = match p.hook_strong_weak {
                    StrongWeak::Weak => 0,
                    StrongWeak::Neutral => 1,
                    StrongWeak::Strong => 2,
                };
                let got = format!(
                    " plate {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {}",
                    p.id,
                    bits(p.pos.x),
                    bits(p.pos.y),
                    bits(p.color[0]),
                    bits(p.color[1]),
                    bits(p.color[2]),
                    bits(p.color[3]),
                    p.show_name as i32,
                    hex(&p.name),
                    p.show_friend_mark as i32,
                    p.show_client_id as i32,
                    bits(p.font_size_client_id),
                    p.client_id_separate_line as i32,
                    bits(p.font_size),
                    p.show_clan as i32,
                    hex(&p.clan),
                    bits(p.font_size_clan),
                    p.show_direction as i32,
                    p.dir_left as i32,
                    p.dir_jump as i32,
                    p.dir_right as i32,
                    bits(p.font_size_direction),
                    p.show_hook_strong_weak as i32,
                    sw,
                    p.show_hook_strong_weak_id as i32,
                    p.hook_strong_weak_id,
                    bits(p.font_size_hook_strong_weak)
                );
                assert_eq!(got, lines.next().unwrap());
                plates_checked += 1;
            }
            assert_eq!(lines.next().unwrap(), "end");
        }

        // freeze bars
        for _ in 0..3 {
            let w: Vec<&str> = lines.next().unwrap().split(' ').collect();
            let v = ints(&w[1..6]);
            let cfg = BarConfig {
                alpha_inside_freeze: v[0],
                show_others_alpha: v[1],
            };
            let (tick, pred_tick, prediction_tick) = (v[2], v[3], v[4]);
            let screen = vec2(float(w[6]), float(w[7]));
            let mut cores = Vec::new();
            let mut rest = Vec::new();
            loop {
                let l = lines.next().unwrap();
                if let Some(out) = l.strip_prefix("barsout") {
                    // tees in client id order, like C++'s loop
                    let mut order: Vec<usize> = (0..cores.len()).collect();
                    order.sort_by_key(|&i| ids[i]);
                    let tees: Vec<Tee> = order
                        .iter()
                        .map(|&i| {
                            let (id, available, display, ext_end, predicted, local_pred, p) =
                                rest[i];
                            Tee {
                                id,
                                core: &cores[i],
                                has_display_info: display,
                                ext_freeze_end: ext_end,
                                tick: if local_pred {
                                    pred_tick
                                } else if predicted {
                                    prediction_tick
                                } else {
                                    tick
                                },
                                pos: p,
                                other_team: other[id as usize],
                                available,
                            }
                        })
                        .collect();
                    let got: String = freezebars::bars(
                        &tees,
                        local,
                        (screen, screen + vec2(1200.0, 700.0)),
                        &cfg,
                    )
                    .iter()
                    .map(|b| {
                        format!(
                            " bar {} {} {} {} {} {}",
                            bits(b.pos.x),
                            bits(b.pos.y),
                            bits(freezebars::WIDTH),
                            bits(freezebars::HEIGHT),
                            bits(b.progress),
                            bits(b.alpha)
                        )
                    })
                    .collect();
                    assert_eq!(got, out);
                    bars_checked += 1;
                    break;
                }
                let c: Vec<&str> = l.split(' ').collect();
                let v = ints(&c[1..11]);
                let mut core = CharacterCore::default();
                (core.freeze_start, core.freeze_end) = (v[5], v[6]);
                core.is_in_freeze = v[7] != 0;
                cores.push(core);
                // C++ IsPlayerInfoAvailable: active, previous and current player info
                rest.push((
                    v[0],
                    v[1] != 0 && v[2] != 0,
                    v[3] != 0,
                    v[4],
                    v[8] != 0,
                    v[9] != 0,
                    vec2(float(c[11]), float(c[12])),
                ));
            }
        }
    }
    assert!(lines.next().is_none());
    assert!(rows_checked >= 300 && plates_checked > 1000 && bars_checked == 900);
}
