//! M6.7: mouse, camera, settings and binds against the C++ code. `camera.txt` comes from
//! `rs_parity camera`, which runs the real CControls and CCamera; `settings/*.saved.cfg`
//! are what the C++ client saved after loading `settings/corpus.cfg` (or nothing), from
//! `scripts/cpp_settings.ps1` (see PROGRESS.md).

use ddnet::net::client::calc_screen_params;
use ddnet::physics::vmath::{trunc_i32, vec2};
use ddnet::protocol::generated as g;
use ddnet::protocol::packer::Packer;
use ddnet_rs::camera::{Camera, Frame};
use ddnet_rs::config::{Config, Settings, VAR_NAMES};
use ddnet_rs::controls::{Controls, GameState, MouseInputType, PlayerState};

fn golden(name: &str) -> String {
    let path = format!("{}/../tests/golden/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(path).unwrap()
}

fn bits(f: f32) -> String {
    format!("{:08x}", f.to_bits())
}

fn float(hex: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(hex, 16).unwrap())
}

/// The `config` line of `rs_parity camera`, in its order.
fn config(f: &[&str]) -> Config {
    let v: Vec<i32> = f[1..].iter().map(|s| s.parse().unwrap()).collect();
    Config {
        cl_dyncam: v[0],
        cl_dyncam_max_distance: v[1],
        cl_dyncam_min_distance: v[2],
        cl_dyncam_mousesens: v[3],
        cl_dyncam_deadzone: v[4],
        cl_dyncam_follow_factor: v[5],
        cl_dyncam_smoothness: v[6],
        cl_dyncam_stabilizing: v[7],
        cl_mouse_deadzone: v[8],
        cl_mouse_followfactor: v[9],
        cl_mouse_max_distance: v[10],
        cl_mouse_min_distance: v[11],
        inp_mousesens: v[12],
        cl_default_zoom: v[13],
        cl_smooth_zoom_time: v[14],
        cl_limit_max_zoom_level: 0, // the C++ side cannot ask IGraphics
        ..Config::default()
    }
}

/// OnCursorMove scaling, ClampMousePos and the mouse distances; then frames of
/// UpdateCamera + OnRender with mouse moves, zoom commands and a moving tee: all
/// bit-identical.
#[test]
fn camera_matches_cpp() {
    let text = golden("camera.txt");
    let mut wrong = Vec::new();
    let mut check = |got: String, want: &str| {
        if got != want && wrong.len() < 20 {
            wrong.push(format!("got  {got}\nwant {want}"));
        }
    };
    let (mut cfg, mut controls, mut camera) =
        (Config::default(), Controls::default(), Camera::default());
    let mut new_sequence = false;
    let (mut mice, mut frames, mut msgs) = (0, 0, 0);
    for line in text.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        match f[0] {
            "config" => {
                cfg = config(&f);
                new_sequence = true;
            }
            "mouse" => {
                let pos = vec2(float(f[1]), float(f[2]));
                let (dx, dy) = (f[3].parse::<f32>().unwrap(), f[4].parse::<f32>().unwrap());
                controls.mouse_pos = pos;
                controls.clamp_mouse_pos(&cfg);
                let clamped = controls.mouse_pos;
                controls.mouse_pos = pos;
                controls.on_cursor_move(dx, dy, &cfg, false);
                let moved = controls.mouse_pos;
                let got = [
                    Controls::min_mouse_distance(&cfg),
                    Controls::max_mouse_distance(&cfg),
                    clamped.x,
                    clamped.y,
                    moved.x,
                    moved.y,
                ]
                .map(bits)
                .join(" ");
                check(format!("{} {got}", f[..5].join(" ")), line);
                mice += 1;
            }
            "msg" => {
                // Cl_ShowDistance and Cl_CameraInfo filled like gameclient.cpp:2361, bodies
                // byte for byte (ids are extended: the UUIDs are checked in M3)
                let (aspect, zoom) = (float(f[1]), float(f[2]));
                let mut d = Packer::default();
                let (x, y) = calc_screen_params(aspect, zoom);
                g::MsgClShowDistance {
                    x: trunc_i32(x),
                    y: trunc_i32(y),
                }
                .pack(&mut d);
                let mut i = Packer::default();
                g::MsgClCameraInfo {
                    zoom: trunc_i32(zoom * 1000.0),
                    deadzone: f[3].parse().unwrap(),
                    follow_factor: f[4].parse().unwrap(),
                }
                .pack(&mut i);
                let bytes = [d.data(), i.data()].concat();
                let got: Vec<String> = bytes.iter().map(|b| format!("{b:02X}")).collect();
                // C++: "<id> <hex bytes> <id> <hex bytes>"; the ids are the longer tokens
                let want: Vec<&str> = line
                    .split(" | ")
                    .nth(1)
                    .unwrap()
                    .split_whitespace()
                    .filter(|t| t.len() == 2)
                    .collect();
                check(got.join(" "), &want.join(" "));
                msgs += 1;
            }
            "frame" => {
                if new_sequence {
                    camera = Camera::default();
                    controls.mouse_pos = vec2(0.0, 0.0);
                    camera.on_reset(&cfg);
                    new_sequence = false;
                }
                let frame = Frame {
                    local_time: float(f[1]),
                    frame_time: float(f[2]),
                    allow_zoom: f[9] == "1",
                };
                let local = vec2(float(f[3]), float(f[4]));
                let (action, has_arg, arg) = (f[5], f[8] == "1", float(f[10]));
                let args = |a: f32| if has_arg { vec![a.to_string()] } else { vec![] };
                let line_of = |cmd: &str| match has_arg {
                    true => format!(
                        "{cmd} {}",
                        args(if cmd == "zoom" { arg + 10.0 } else { arg })[0]
                    ),
                    false => cmd.to_string(),
                };
                match action {
                    "0" | "1" | "2" | "3" | "4" => {
                        let d = |i: usize| f[i].parse::<f32>().unwrap();
                        controls.on_cursor_move(d(6), d(7), &cfg, false);
                    }
                    "5" | "6" | "7" => {
                        let cmd = ["zoom+", "zoom-", "zoom"][action.parse::<usize>().unwrap() - 5];
                        let game = GameState::default();
                        controls.execute(true, &line_of(cmd), game, &mut camera, &frame, &cfg);
                    }
                    _ => {}
                }
                camera.update(controls.mouse_pos, &frame, &cfg);
                camera.on_render(&mut controls, local, &cfg);
                let input_type = match controls.input_type {
                    MouseInputType::Absolute => 0,
                    MouseInputType::Relative => 1,
                    MouseInputType::Automated => 2,
                };
                let c = &camera;
                let got = format!(
                    "{} {} {} {} {} {} {} {} {} {} {} {} {} {}",
                    bits(c.zoom),
                    c.zooming as i32,
                    [
                        c.zoom_smoothing_target,
                        c.user_zoom_target,
                        c.dyncam_speed_bias
                    ]
                    .map(bits)
                    .join(" "),
                    bits(c.dyncam_target_offset.x),
                    bits(c.dyncam_target_offset.y),
                    bits(c.dyncam_current_offset.x),
                    bits(c.dyncam_current_offset.y),
                    bits(c.center.x),
                    bits(c.center.y),
                    bits(controls.mouse_pos.x),
                    bits(controls.mouse_pos.y),
                    input_type,
                    "",
                    ""
                );
                let got = got.trim_end();
                check(format!("{} | {got}", f[..11].join(" ")), line);
                frames += 1;
            }
            _ => panic!("{line}"),
        }
    }
    assert_eq!((msgs, mice, frames), (2000, 5000, 400 * 80));
    assert!(wrong.is_empty(), "differ:\n{}", wrong.join("\n"));
}

/// Our settings after loading `file` (none: defaults), in the C++ saved-config form:
/// our variables (sorted) and the bind lines in C++ order.
fn saved(file: Option<&[u8]>) -> (Vec<String>, Vec<String>) {
    // scripts/cpp_settings.ps1 starts the C++ client with "snd_enable 0" after the file
    let mut files = file.as_slice().to_vec();
    files.push(b"snd_enable 0");
    let s = Settings::load(&files);
    let mut vars = s.config.serialize(false);
    vars.sort();
    (vars, s.binds.serialize())
}

/// The same parts of a config file the C++ client saved.
fn cpp_saved(name: &str) -> (Vec<String>, Vec<String>) {
    let text = golden(&format!("settings/{name}"));
    let ours = |l: &&str| VAR_NAMES.contains(&l.split(' ').next().unwrap());
    let mut vars: Vec<String> = text.lines().filter(ours).map(String::from).collect();
    vars.sort();
    let binds = text
        .lines()
        .filter(|l| l.starts_with("bind "))
        .map(String::from)
        .collect();
    (vars, binds)
}

/// `CBinds::SetDefaults` + `SetDDRaceBinds`: the binds a new C++ client saves.
#[test]
fn default_binds_match_cpp() {
    assert_eq!(saved(None), cpp_saved("defaults.saved.cfg"));
}

/// A settings file with every console-syntax edge we could think of (quotes, escapes,
/// `;`, `#`, modifiers, `&N` key numbers, out-of-range and invalid values, UTF-8 cut
/// by the name length): loaded and saved by the C++ client, loaded by us.
#[test]
fn settings_file_matches_cpp() {
    let corpus = golden("settings/corpus.cfg");
    let (vars, binds) = saved(Some(corpus.as_bytes()));
    let (cpp_vars, cpp_binds) = cpp_saved("corpus.saved.cfg");
    // only the unknown client setting is skipped (binds and our variables are not)
    assert_eq!(Settings::load(&[corpus.as_bytes()]).skipped, 1);
    assert_eq!(vars, cpp_vars);
    let diff: Vec<String> = binds
        .iter()
        .zip(&cpp_binds)
        .filter(|(a, b)| a != b)
        .take(10)
        .map(|(a, b)| {
            format!(
                "ours {a}
cpp  {b}"
            )
        })
        .collect();
    assert!(
        diff.is_empty(),
        "{}",
        diff.join(
            "
"
        )
    );
    assert_eq!(binds.len(), cpp_binds.len());
}

/// M6.8: C++ input rules while a menu is open. `CControls::SnapInput` (controls.cpp:277):
/// the player flags say IN_MENU, the input is reset from the last snapped one (movement,
/// jump released, fire counted as released, hook kept), and the copy sent keeps the
/// previous target. `CBinds::OnInput` (binds.cpp:180): presses go to the menu, releases
/// end active binds without running them.
#[test]
fn menu_input_follows_cpp() {
    use ddnet::protocol::generated as g;
    use ddnet_rs::config::find_key_by_name;

    let mut s = Settings::load(&[]);
    let (mut c, mut cam) = (Controls::default(), Camera::default());
    let cfg = Config::default();
    let frame = Frame {
        local_time: 0.0,
        frame_time: 0.01,
        allow_zoom: true,
    };
    let none = |_: usize| false;
    let (a, space, m1, m2) = (
        find_key_by_name("a"),
        find_key_by_name("space"),
        find_key_by_name("mouse1"),
        find_key_by_name("mouse2"),
    );
    for key in [a, space, m1, m2] {
        for (stroke, line) in s.binds.on_input(key, true, false, false, none) {
            c.execute(stroke, &line, GameState::default(), &mut cam, &frame, &cfg);
        }
    }
    c.mouse_pos = vec2(100.0, -50.0);
    let playing = c.snap_input(
        PlayerState::Playing,
        false,
        false,
        Default::default(),
        &mut Default::default(),
    );
    assert_eq!(
        (playing.direction, playing.jump, playing.fire, playing.hook),
        (-1, 1, 1, 1)
    );
    assert_eq!((playing.target_x, playing.target_y), (100, -50));
    assert_ne!(playing.player_flags & g::PLAYERFLAG_PLAYING, 0);

    // the menu opens: the key releases reach the binds but run nothing
    for key in [a, space, m1, m2] {
        assert!(s.binds.on_input(key, false, true, true, none).is_empty());
    }
    assert!(s.binds.on_input(a, true, false, true, none).is_empty());
    c.mouse_pos = vec2(30.0, 40.0);
    let menu = c.snap_input(
        PlayerState::InMenu,
        false,
        false,
        Default::default(),
        &mut Default::default(),
    );
    assert_eq!(menu.player_flags & g::PLAYERFLAG_PLAYING, 0);
    assert_ne!(menu.player_flags & g::PLAYERFLAG_IN_MENU, 0);
    assert_eq!(
        (menu.direction, menu.jump, menu.fire, menu.hook),
        (0, 0, 2, 1)
    );
    // the target follows the mouse: C++ sends the input after updating it (dummy.txt)
    assert_eq!((menu.target_x, menu.target_y), (30, 40));
    // with the DDRace input bug the input is not reset
    let mut d = Controls::default();
    d.input.jump = 1;
    assert_eq!(
        d.snap_input(
            PlayerState::InMenu,
            false,
            true,
            Default::default(),
            &mut Default::default()
        )
        .jump,
        1
    );
}

/// M6.8: our settings file reloads to the same settings, on top of any C++ file.
#[test]
fn own_settings_file_round_trips() {
    let corpus = golden("settings/corpus.cfg");
    let mut s = Settings::load(&[corpus.as_bytes()]);
    s.binds
        .rebind("+jump", ddnet_rs::config::find_key_by_name("kp_5"));
    s.execute_line("inp_mousesens 200"); // back to the default: must still override
    let text = s.serialize();
    for base in [&b""[..], corpus.as_bytes()] {
        let again = Settings::load(&[base, text.as_bytes()]);
        assert_eq!(again.config, s.config);
        assert_eq!(again.binds.serialize(), s.binds.serialize());
    }
}

fn input(s: &str) -> g::ObjPlayerInput {
    let v: Vec<i32> = s.split(',').map(|x| x.parse().unwrap()).collect();
    g::ObjPlayerInput::from_ints(&v)
}

fn ints(i: &g::ObjPlayerInput) -> String {
    let v: Vec<String> = i.to_ints().iter().map(i32::to_string).collect();
    v.join(",")
}

/// 13.6: `dummy.txt` from `rs_parity dummy` (copies of CControls::SnapInput/ResetInput,
/// CGameClient::OnDummySwap/OnSnapInput and CClient::SendInput): random binds, mouse
/// moves, dummy settings (copy moves, dummy control, hammer, reset on switch) and
/// swaps; at each SendInput both tees' inputs and last inputs, the mouse, the sent input,
/// and the other tee's `m_DummyInput`/`m_HammerInput`/`m_DummyFire` must match.
#[test]
fn dummy_inputs_match_cpp() {
    use ddnet::net::client::DummySnap;
    use ddnet_rs::controls::DummyControl;
    let text = golden("dummy.txt");
    let cfg = Config::default();
    let mut camera = Camera::default();
    let frame = Frame {
        local_time: 0.0,
        frame_time: 0.0,
        allow_zoom: true,
    };
    let mut controls = Controls::default();
    let mut snap = DummySnap::default();
    let (mut dummy, mut last_dummy) = (false, false);
    let mut c: Vec<i32> = vec![0; 17];
    let mut pos = [vec2(0.0, 0.0); 3]; // local character, clients 3 and 5
    let (mut snaps, mut others) = (0, 0);
    for (n, line) in text.lines().enumerate() {
        let f: Vec<&str> = line.split(' ').collect();
        let at = format!("line {}: {line}", n + 1);
        match f[0] {
            "cfg" => {
                c = f[1..18].iter().map(|x| x.parse().unwrap()).collect();
                for (i, p) in pos.iter_mut().enumerate() {
                    *p = vec2(float(f[18 + 2 * i]), float(f[19 + 2 * i]));
                }
            }
            "dummy" => {
                dummy = f[1] == "1";
                if dummy != last_dummy {
                    controls.on_dummy_swap(c[7], &mut snap.input);
                }
                last_dummy = dummy;
            }
            "key" => {
                let (key, value): (usize, i32) = (f[2].parse().unwrap(), f[3].parse().unwrap());
                let names = [
                    "+left",
                    "+right",
                    "+jump",
                    "+hook",
                    "+fire",
                    "+nextweapon",
                    "+prevweapon",
                ];
                let (name, stroke) = match names.get(key) {
                    Some(n) => (n.to_string(), value == 1),
                    None => (format!("+weapon{value}"), true),
                };
                controls.execute(
                    stroke,
                    &name,
                    GameState::default(),
                    &mut camera,
                    &frame,
                    &cfg,
                );
            }
            "mouse" => {
                controls.mouse_pos = vec2(float(f[2]), float(f[3]));
                controls.input_type = [
                    MouseInputType::Absolute,
                    MouseInputType::Relative,
                    MouseInputType::Automated,
                ][f[4].parse::<usize>().unwrap()];
            }
            "snap" => {
                snaps += 1;
                let state = if c[13] != 0 {
                    PlayerState::Chatting
                } else if c[14] != 0 {
                    PlayerState::InMenu
                } else {
                    PlayerState::Playing
                };
                let ctl = DummyControl {
                    copy_moves: c[0] != 0,
                    control: c[1] != 0,
                    jump: c[2],
                    fire: c[3],
                    hook: c[4],
                    spectating_other: c[11] != 0 && c[12] >= 0,
                };
                let sent = controls.snap_input(state, c[15] != 0, c[16] != 0, ctl, &mut snap.input);
                if f[1] != "0" {
                    assert_eq!(ints(&sent), f[2], "sent input, {at}");
                }
                // C++ arrays by tee: [cl_dummy] is our active tee
                let tee = |t: usize| {
                    if (t == 1) == dummy {
                        &*controls
                    } else {
                        &controls.other
                    }
                };
                for t in 0..2 {
                    let k = 3 + 4 * t;
                    let tc = tee(t);
                    assert_eq!(ints(&tc.input), f[k], "tee {t} input, {at}");
                    assert_eq!(ints(&tc.last), f[k + 1], "tee {t} last, {at}");
                    let mouse = (bits(tc.mouse_pos.x), bits(tc.mouse_pos.y));
                    assert_eq!(
                        mouse,
                        (f[k + 2].into(), f[k + 3].into()),
                        "tee {t} mouse, {at}"
                    );
                }
                assert_eq!(ints(&snap.input), f[11], "dummy input, {at}");
            }
            "other" => {
                others += 1;
                let (force, known) = (f[1] == "1", f[2] == "1");
                let pre = DummySnap {
                    input: input(f[3]),
                    hammer_input: input(f[4]),
                    fire: f[5].parse().unwrap(),
                };
                assert_eq!(snap, pre, "state before OnSnapInput, {at}");
                let other_id = c[9 + !dummy as usize];
                let p = pos[if other_id == 3 { 1 } else { 2 }];
                let dir = (pos[0].x - p.x, pos[0].y - p.y);
                let got = if known {
                    snap.snap(force, c[5] != 0, c[6] != 0, dir)
                } else {
                    None
                };
                assert_eq!(got.is_some(), f[7] != "0", "sent or not, {at}");
                if let Some(got) = got {
                    assert_eq!(ints(&got), f[8], "sent, {at}");
                }
                let post = DummySnap {
                    input: input(f[9]),
                    hammer_input: input(f[10]),
                    fire: f[11].parse().unwrap(),
                };
                assert_eq!(snap, post, "state after OnSnapInput, {at}");
            }
            _ => {}
        }
    }
    assert!(snaps > 5000 && others > 5000, "{snaps} {others}");
}
