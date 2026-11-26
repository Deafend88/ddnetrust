//! PLAN §13.1: the chat logic against the C++ `CChat`. `chat.txt` comes from
//! `rs_parity chat`, which runs word-for-word copies of `AddLine`, `OnInput`,
//! `OnMessage`, `SendChatQueued` and the rest on random games and events (see
//! src/tools/rs_parity_chat.cpp); after every event the chat state must be identical.

use ddnet::clients::Clients;
use ddnet::protocol::generated::{GameMsg, MsgSvCommandInfo, MsgSvCommandInfoRemove};
use ddnet_rs::chat::{Chat, ChatConfig, Ctx, Key, Line, Mode, Sound, parse_save};

fn golden(name: &str) -> String {
    let path = format!("{}/../tests/golden/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(path).unwrap()
}

fn unhex(s: &str) -> Vec<u8> {
    if s == "-" {
        return Vec::new();
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn hex(b: &[u8]) -> String {
    if b.is_empty() {
        return "-".into();
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// `DumpLine` of the C++ tool.
fn line(l: &Line) -> String {
    format!(
        "{} {} {} {} {} {} {} {} {} {} {} {} {} {}",
        l.initialized as i32,
        l.time,
        l.client_id,
        l.team_number,
        l.team as i32,
        l.whisper as i32,
        l.name_color,
        hex(l.name.as_bytes()),
        hex(l.text.as_bytes()),
        l.friend as i32,
        l.highlighted as i32,
        l.custom_color.is_some() as i32,
        l.times_repeated,
        l.tee.is_some() as i32
    )
}

/// `DumpState` of the C++ tool.
fn state(c: &Chat, released: bool) -> String {
    let (used, chosen, offset, length, pending) = c.state();
    let mode = match c.mode {
        Mode::None => 0,
        Mode::All => 1,
        Mode::Team => 2,
    };
    format!(
        "= {mode} {} {} {} {chosen} {offset} {length} {pending} {} {} {} | {}",
        hex(c.input.text.as_bytes()),
        c.input.cursor,
        used as i32,
        c.current,
        c.show as i32,
        released as i32,
        line(&c.lines[c.current])
    )
}

/// The C++ `CSounds::Play` ids of the chat sounds.
fn sound_id(s: Sound) -> i32 {
    match s {
        Sound::Server => 32,
        Sound::Client => 33,
        Sound::Highlight => 34,
    }
}

#[test]
fn chat_matches_cpp() {
    let text = golden("chat.txt");
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().unwrap().split(' ').collect();
    let (runs, events) = (
        header[1].parse::<usize>().unwrap(),
        header[2].parse().unwrap(),
    );
    let mut checked = 0;
    for _ in 0..runs {
        let f: Vec<i64> = (lines.next().unwrap().split(' ').skip(2))
            .map(|s| s.parse().unwrap())
            .collect();
        let config = ChatConfig {
            show_chat_system: f[0] != 0,
            show_chat_friends: f[1] != 0,
            show_chat_team_members_only: f[2] != 0,
            chat_reset: f[3] != 0,
            snd_server_message: f[4] != 0,
            snd_highlight: f[5] != 0,
            snd_team_chat: f[6] != 0,
            snd_chat: f[7] != 0,
            message_client_color: 9633471,
        };
        let (team_play, demo) = (f[8] != 0, f[9] != 0);
        let mut clients = Clients::default();
        let mut teams = [0; 128];
        let mut other = [false; 128];
        let local_ids;
        loop {
            let l = lines.next().unwrap();
            let w: Vec<&str> = l.split(' ').collect();
            if w[0] == "local" {
                clients.local_id = w[1].parse().unwrap();
                local_ids = [w[2].parse().unwrap(), w[3].parse().unwrap()];
                clients.by_name = w[5..].iter().map(|s| s.parse().unwrap()).collect();
                break;
            }
            let id: usize = w[1].parse().unwrap();
            let v: Vec<i32> = w[3..].iter().map(|s| s.parse().unwrap()).collect();
            let c = &mut clients.clients[id];
            c.name = String::from_utf8(unhex(w[2])).unwrap();
            (c.active, c.chat_ignore, c.friend, c.foe) =
                (v[0] != 0, v[1] != 0, v[2] != 0, v[3] != 0);
            c.team = v[4];
            other[id] = v[5] != 0;
            teams[id] = v[6];
        }
        let team = |id: i32| if id >= 0 { teams[id as usize] } else { 0 };
        let is_other_team = |id: i32| other[id as usize];
        let mut chat = Chat::default();
        let mut now = f[10];
        for _ in 0..events {
            let l = lines.next().unwrap();
            let (event, want_state) = l.split_once(" = ").unwrap();
            let w: Vec<&str> = event.split(' ').collect();
            // the event's own words, then what it printed (sounds, saves, sends)
            let n = match w[0] {
                "msg" => 4,
                "cmd" | "type" | "key" | "say" | "active" => 3,
                "cmdrm" | "enable" | "time" => 2,
                _ => 1,
            };
            let want_out = w[n..].join(" ");
            let mut got = String::new();
            let mut released = false;
            if w[0] == "active" {
                let id: usize = w[1].parse().unwrap();
                clients.clients[id].active = w[2] == "1";
                let mut by_name: Vec<i32> = (0..128)
                    .filter(|&i| clients.clients[i as usize].active)
                    .collect();
                by_name.sort_by(|&a, &b| {
                    let name = |i: i32| clients.clients[i as usize].name.as_bytes();
                    ddnet::text::comp_nocase(name(a), name(b))
                });
                clients.by_name = by_name;
            }
            let ctx = Ctx {
                clients: &clients,
                local_ids,
                team: &team,
                is_other_team: &is_other_team,
                team_play,
                demo,
                config,
            };
            match w[0] {
                "msg" => {
                    let mut text = unhex(w[3]);
                    let (id, team) = (w[1].parse().unwrap(), w[2].parse().unwrap());
                    chat.add_line(id, team, &mut text, &ctx, now);
                    if !demo && id == -1 {
                        got += &format!(" save:{}", hex(&text));
                    }
                }
                "cmd" | "cmdrm" => {
                    let name = String::from_utf8(unhex(w[1])).unwrap();
                    let msg = if w[0] == "cmd" {
                        GameMsg::SvCommandInfo(MsgSvCommandInfo {
                            name,
                            args_format: String::from_utf8(unhex(w[2])).unwrap(),
                            help_text: "help".into(),
                        })
                    } else {
                        GameMsg::SvCommandInfoRemove(MsgSvCommandInfoRemove { name })
                    };
                    chat.on_message(&msg, &ctx, now);
                }
                "enable" => chat.enable_mode(w[1].parse().unwrap(), demo),
                "type" => {
                    released = chat.on_key(Key::Other, false, &ctx, config.chat_reset, now);
                    if chat.is_active() {
                        chat.input.set(&String::from_utf8(unhex(w[1])).unwrap());
                        chat.input.set_cursor(w[2].parse().unwrap());
                    }
                }
                "key" => {
                    let key = match w[1].parse::<i32>().unwrap() {
                        43 => Key::Tab,
                        40 => Key::Enter,
                        41 => Key::Escape,
                        82 => Key::Up,
                        81 => Key::Down,
                        225 => Key::Shift,
                        _ => Key::Other,
                    };
                    released = chat.on_key(key, w[2] == "1", &ctx, config.chat_reset, now);
                }
                "time" => {
                    now = w[1].parse().unwrap();
                    chat.on_render(now);
                }
                "render" => chat.on_render(now),
                "say" => {
                    chat.enable_mode(w[1].parse().unwrap(), demo);
                    chat.on_key(Key::Other, false, &ctx, config.chat_reset, now);
                    if chat.is_active() {
                        chat.input.set(&String::from_utf8(unhex(w[2])).unwrap());
                    }
                    released = chat.on_key(Key::Enter, false, &ctx, config.chat_reset, now);
                }
                "active" => {}
                e => panic!("unknown event {e}"),
            }
            for (team, text) in chat.sends.drain(..) {
                got += &format!(" send{team}:{}", hex(text.as_bytes()));
            }
            // C++ prints the sounds while adding the line, before StoreSave
            let sounds: String = (chat.sounds.drain(..))
                .map(|s| format!(" sound{}", sound_id(s)))
                .collect();
            let got_out = format!("{sounds}{got}");
            assert_eq!(got_out.trim_start(), want_out, "event: {l}");
            assert_eq!(
                state(&chat, released),
                format!("= {want_state}"),
                "event: {l}"
            );
            checked += 1;
        }
        let l = lines.next().unwrap();
        let want: Vec<&str> = l.strip_prefix("lines ").unwrap().split(" ;").collect();
        for (i, w) in want.iter().filter(|w| !w.trim().is_empty()).enumerate() {
            assert_eq!(
                w.trim(),
                format!("{i} {}", line(&chat.lines[i])),
                "line {i}"
            );
        }
    }
    assert_eq!(checked, 40 * 400);
}

#[test]
fn save_lines() {
    assert_eq!(
        parse_save(
            "Team successfully saved by brainless tee. Use '/load code word here' on ddnet.org to continue"
        ),
        Some(("brainless tee".into(), "code word here".into()))
    );
    assert_eq!(
        parse_save("Team successfully saved by a. Use '/load abc' to continue"),
        Some(("a".into(), "abc".into()))
    );
    assert_eq!(parse_save("Team successfully saved by a"), None);
}
