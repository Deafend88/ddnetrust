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
use ddnet_rs::controls::{Controls, GameState, MouseInputType};

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
    let s = Settings::load(file.as_slice());
    let mut vars = s.config.serialize();
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
