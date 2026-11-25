//! End-to-end session against a local `DDNet-Server` built from this repo:
//! handshake, in-band map download, enter game, snapshots, inputs, clean disconnect.
//!
//! The server binary is `$DDNET_SERVER` or `build-rs/DDNet-Server.exe` (CMake target
//! `game-server`). It runs in a temporary directory whose `storage.cfg` excludes the
//! real user directory, bound to 127.0.0.1 with `sv_register 0`.

use std::fs;
use std::net::UdpSocket;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

use ddnet::map::Map;
use ddnet::net::client::{Client, PlayerInfo, State, VERSION_STRING};
use ddnet::net::{self, conn::FREQ};
use ddnet::physics::collision::Collision;
use ddnet::predict::Predictor;
use ddnet::protocol::generated as g;

struct Server {
    child: Child,
    dir: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn server_binary() -> Option<PathBuf> {
    let path = std::env::var_os("DDNET_SERVER")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../build-rs/DDNet-Server.exe")
        });
    path.exists().then_some(path)
}

fn start_server(exe: &PathBuf, map: &str) -> (Server, u16) {
    start_server_with(exe, map, &[])
}

/// `start_server` with `map_cfg` as the map's config (`maps/<map>.cfg`): the server
/// imports it into the map's settings, so it runs after the game type reset the tuning,
/// and clients load it too (`LoadMapSettings`).
fn start_server_with(exe: &PathBuf, map: &str, map_cfg: &[&str]) -> (Server, u16) {
    let port = UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let dir = std::env::temp_dir().join(format!("ddnet-rs-server-{}-{port}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("storage.cfg"),
        "add_path $CURRENTDIR\nadd_path $DATADIR\n",
    )
    .unwrap();
    if !map_cfg.is_empty() {
        fs::create_dir_all(dir.join("maps")).unwrap();
        fs::write(dir.join(format!("maps/{map}.cfg")), map_cfg.join("\n")).unwrap();
    }
    let log = fs::File::create(dir.join("server.log")).unwrap();
    let child = Command::new(exe)
        .current_dir(&dir)
        .args([
            "sv_register 0",
            "bindaddr 127.0.0.1",
            &format!("sv_port {port}"),
            &format!("sv_map {map}"),
            "sv_tee_historian 1",
        ])
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log))
        .spawn()
        .unwrap();
    (Server { child, dir }, port)
}

fn local_x(client: &Client) -> Option<i32> {
    let id = client.local_id()?;
    let item = client
        .snapshot()
        .iter()
        .find(|it| it.type_id == g::NETOBJTYPE_CHARACTER && it.id == id)?;
    Some(g::ObjCharacter::from_ints(&item.data).x)
}

#[test]
fn local_server_session() {
    let Some(exe) = server_binary() else {
        eprintln!("SKIPPED: no DDNet-Server (set DDNET_SERVER or build the game-server target)");
        return;
    };
    let (server, port) = start_server(&exe, "coverage");
    let socket = net::connect_socket(format!("127.0.0.1:{port}").parse().unwrap()).unwrap();
    let start = Instant::now();
    let mut client = Client::new(PlayerInfo::default(), "", net::random_bytes);
    client.connect(net::now(start));

    // run the client until `done` or the deadline
    let run = |client: &mut Client, secs: u64, done: &dyn Fn(&Client) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < deadline && !done(client) {
            net::pump(&socket, client, net::now(start)).unwrap();
            std::thread::sleep(Duration::from_millis(1));
        }
    };

    run(&mut client, 15, &|c| {
        c.state() == State::Online && local_x(c).is_some()
    });
    assert_eq!(
        client.state(),
        State::Online,
        "error: '{}'",
        client.conn.error()
    );
    let map = client.map().expect("map downloaded");
    assert_eq!(map.name, "coverage");
    assert_eq!(
        map.data.len(),
        fs::metadata(exe.parent().unwrap().join("data/maps/coverage.map"))
            .unwrap()
            .len() as usize
    );

    // inputs reach the server: walk right for half a second
    let x0 = local_x(&client).unwrap();
    client.input.direction = 1;
    let walk_until = net::now(start) + FREQ / 2;
    run(&mut client, 5, &|_| net::now(start) > walk_until);
    client.input.direction = 0;
    let x1 = local_x(&client).unwrap();
    assert!(x1 > x0, "character did not move right: {x0} -> {x1}");
    assert!(
        client.pred_tick() > client.game_tick().unwrap(),
        "prediction must run ahead of the server"
    );

    client.disconnect(None, net::now(start));
    net::pump(&socket, &mut client, net::now(start)).unwrap();
    std::thread::sleep(Duration::from_millis(500));

    let log = fs::read_to_string(server.dir.join("server.log")).unwrap();
    assert!(
        log.contains("player has entered the game. ClientId=0"),
        "{log}"
    );
    assert!(log.contains("cid=0 version=20030"), "{log}");
    assert!(log.contains("'nameless tee' has left the game"), "{log}");
    assert!(!log.contains("Timeout"), "{log}");
    // the server records our honest version string (teehistorian)
    let recorded = fs::read_dir(server.dir.join("teehistorian"))
        .unwrap()
        .any(|f| {
            fs::read(f.unwrap().path())
                .unwrap()
                .windows(VERSION_STRING.len())
                .any(|w| w == VERSION_STRING.as_bytes())
        });
    assert!(recorded, "version string not found in teehistorian");
    let dir = server.dir.clone();
    drop(server);
    let _ = fs::remove_dir_all(dir);
}

/// M5: prediction from snapshots against the server. Alone on the map with scripted
/// walking, jumping and hooking, every snapshot of the local character must equal what
/// was predicted for its tick, and re-predicting a tick must never change it.
#[test]
fn prediction_matches_server() {
    for map_name in ["dm1", "coverage"] {
        let Some((pred, _, weak_col)) = check_prediction(map_name, &[], None) else {
            return;
        };
        // the collision lives exactly as long as the predictor and its world copies
        drop(pred);
        assert!(weak_col.upgrade().is_none(), "collision still alive");
    }
}

/// 8.1: the same with changed tuning (global and per tune zone, so `Sv_TuneParams` and
/// `UpdateLocalTuning` must be right) and, on coverage, after joining DDRace team 7 by
/// chat (so `Sv_TeamsState` must be right). The tuning keeps the scripted path off
/// death tiles: the server turns those into a freeze in practice mode, which the C++
/// client prediction does not model either.
#[test]
fn prediction_matches_server_with_tuning_and_teams() {
    let zones: Vec<String> = (1..=40)
        .flat_map(|z| {
            [
                format!("tune_zone {z} gravity {}", 0.3 + (z % 5) as f32 * 0.1),
                format!("tune_zone {z} ground_control_speed {}", 8 + z % 7),
                format!("tune_zone {z} air_jump_impulse {}", 9 + z % 4),
            ]
        })
        .collect();
    let mut map_cfg = vec![
        "tune gravity 0.47",
        "tune ground_control_speed 11",
        "tune hook_length 300",
    ];
    map_cfg.extend(zones.iter().map(String::as_str));
    for (map_name, chat) in [("dm1", None), ("coverage", Some("/team 7"))] {
        let Some((mut pred, local, _)) = check_prediction(map_name, &map_cfg, chat) else {
            return;
        };
        let team = if chat.is_some() { 7 } else { 0 };
        assert_eq!(pred.teams_mut().team(local), team, "{map_name}: team");
        assert_eq!(
            pred.received_tuning()
                .get(ddnet::physics::tuning::Param::GroundControlSpeed),
            11.0,
            "{map_name}: server tuning not received"
        );
    }
}

/// Runs the scripted movement on `map_name` for 10 s (after sending `chat`, if any) and
/// checks every snapshot of the local character against its prediction. Returns the predictor, the local id and the
/// collision (weak); `None` without a server binary.
fn check_prediction(
    map_name: &str,
    map_cfg: &[&str],
    chat: Option<&str>,
) -> Option<(Predictor, i32, std::rc::Weak<Collision>)> {
    let Some(exe) = server_binary() else {
        eprintln!("SKIPPED: no DDNet-Server (set DDNET_SERVER or build the game-server target)");
        return None;
    };
    let (server, port) = start_server_with(&exe, map_name, map_cfg);
    let socket = net::connect_socket(format!("127.0.0.1:{port}").parse().unwrap()).unwrap();
    let start = Instant::now();
    let mut client = Client::new(PlayerInfo::default(), "", net::random_bytes);
    client.connect(net::now(start));
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline
        && !(client.state() == State::Online && local_x(&client).is_some())
    {
        net::pump(&socket, &mut client, net::now(start)).unwrap();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(client.state(), State::Online, "{map_name}");

    let map = Map::load(client.map().unwrap().data.clone()).unwrap();
    let col = Collision::new(&map);
    let col = Rc::new(col);
    let weak_col = Rc::downgrade(&col);
    let mut pred = Predictor::new(col, &map);
    let entered = net::now(start);
    if let Some(line) = chat {
        client.say(0, line, entered);
    }
    let mut zones = std::collections::BTreeSet::new();
    while net::now(start) - entered < 10 * FREQ {
        net::scripted_input(&mut client.input, net::now(start) - entered);
        net::pump(&socket, &mut client, net::now(start)).unwrap();
        pred.update(&mut client);
        zones.insert(pred.tune_state().1);
        std::thread::sleep(Duration::from_millis(1));
    }
    let s = &pred.stats;
    eprintln!(
        "{map_name} ({} map config lines, chat {chat:?}): {s:?}, tune zones {zones:?}",
        map_cfg.len()
    );
    assert!(s.compared > 100, "{map_name}: {s:?}");
    assert_eq!(s.mismatched, 0, "{map_name}: {s:?}");
    assert_eq!(s.prediction_errors, 0, "{map_name}: {s:?}");
    let local = client.local_id().unwrap_or(-1);
    let dir = server.dir.clone();
    drop(server);
    let _ = fs::remove_dir_all(dir);
    Some((pred, local, weak_col))
}

fn character(client: &Client, id: i32) -> Option<g::ObjCharacter> {
    let item = client
        .snapshot()
        .iter()
        .find(|it| it.type_id == g::NETOBJTYPE_CHARACTER && it.id == id)?;
    Some(g::ObjCharacter::from_ints(&item.data))
}

/// M5: mispredictions when another player interferes, and recovery afterwards. A
/// second client walks to the first one and hooks it for 8 s, then leaves; the first
/// client's prediction cannot know the other's inputs, so some ticks mispredict, and
/// once it is alone again every snapshot must match again.
#[test]
fn prediction_recovers_from_interference() {
    let Some(exe) = server_binary() else {
        eprintln!("SKIPPED: no DDNet-Server (set DDNET_SERVER or build the game-server target)");
        return;
    };
    let (server, port) = start_server(&exe, "dm1");
    let addr = format!("127.0.0.1:{port}").parse().unwrap();
    let start = Instant::now();
    let (sa, sb) = (
        net::connect_socket(addr).unwrap(),
        net::connect_socket(addr).unwrap(),
    );
    let mut a = Client::new(PlayerInfo::default(), "", net::random_bytes);
    let mut b = Client::new(
        PlayerInfo {
            name: "other".into(),
            ..PlayerInfo::default()
        },
        "",
        net::random_bytes,
    );
    a.connect(net::now(start));
    b.connect(net::now(start));
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline && !(local_x(&a).is_some() && local_x(&b).is_some()) {
        net::pump(&sa, &mut a, net::now(start)).unwrap();
        net::pump(&sb, &mut b, net::now(start)).unwrap();
        std::thread::sleep(Duration::from_millis(1));
    }
    let map = Map::load(a.map().unwrap().data.clone()).unwrap();
    let col = Collision::new(&map);
    let mut pred = Predictor::new(Rc::new(col), &map);
    let entered = net::now(start);
    let mut during = None;
    let (mut closest, mut hooked) = (i32::MAX, false);
    while net::now(start) - entered < 14 * FREQ {
        let now = net::now(start);
        net::scripted_input(&mut a.input, now - entered);
        if now - entered < 8 * FREQ {
            // walk towards the first client and hook at it
            if let (Some(ca), Some(cb)) = (
                character(&b, a.local_id().unwrap()),
                character(&b, b.local_id().unwrap()),
            ) {
                b.input.direction = (ca.x - cb.x).signum();
                b.input.target_x = ca.x - cb.x;
                b.input.target_y = ca.y - cb.y;
                b.input.hook = ((now - entered) / (FREQ / 2) % 2) as i32;
                b.input.jump = (ca.y < cb.y - 32) as i32;
            }
            if let (Some(ca), Some(cb)) = (
                character(&a, a.local_id().unwrap()),
                character(&a, b.local_id().unwrap()),
            ) {
                let d = (((ca.x - cb.x).pow(2) + (ca.y - cb.y).pow(2)) as f64).sqrt() as i32;
                closest = closest.min(d);
                hooked |= cb.hooked_player == a.local_id().unwrap();
            }
            net::pump(&sb, &mut b, now).unwrap();
        } else if during.is_none() {
            b.disconnect(None, now);
            net::pump(&sb, &mut b, now).unwrap();
            during = Some(pred.stats.clone());
        }
        net::pump(&sa, &mut a, now).unwrap();
        pred.update(&mut a);
        std::thread::sleep(Duration::from_millis(1));
    }
    let (during, end) = (during.unwrap(), pred.stats.clone());
    eprintln!("closest {closest} px, hooked {hooked}");
    eprintln!("with the other player (8 s): {during:?}");
    eprintln!(
        "alone again (6 s): compared {}, mismatched {}, prediction errors {}",
        end.compared - during.compared,
        end.mismatched - during.mismatched,
        end.prediction_errors - during.prediction_errors
    );
    // the first snapshots after the other player left still carry its last effects
    assert!(end.compared - during.compared > 100, "{end:?}");
    assert!(
        end.mismatched - during.mismatched <= 2,
        "no recovery: {during:?} -> {end:?}"
    );
    drop(server);
}

/// M6.5: what the renderer would draw each frame (`Predictor::rendered`) moves smoothly
/// across tick boundaries: two scripted tees on dm1, the local one drawn between its
/// predicted ticks, the other one between snapshots. Frames come every ~1 ms (20 per
/// tick), so a tee walking or swinging on its hook moves a few pixels per frame, while
/// drawing the current tick's position instead jumps by a whole tick's movement.
/// 10 s: since the camera messages (M6.7) the server clips to our 16:9 view like for the
/// C++ client (radius 1429/2 + 320 px instead of its 1200 px default), so the other tee,
/// often ~1250 px away, is in our snapshots less than half of the time.
/// Rendering runs a tick behind the newest snapshot and snapshots come every 2 ticks, so
/// the other tee is often extrapolated (`IntraGameTick` > 1, unclamped as in C++
/// CClient::Update; it reaches ~1.5 before each switch). That is continuous, but at each
/// switch the tee snaps onto the new interval: those switch frames (about one per
/// snapshot, under 5% of frames) may step up to one snapshot's movement; every other
/// frame must stay under half of it.
#[test]
fn rendered_positions_are_smooth() {
    let Some(exe) = server_binary() else {
        eprintln!("SKIPPED: no DDNet-Server (set DDNET_SERVER or build the game-server target)");
        return;
    };
    let (server, port) = start_server(&exe, "dm1");
    let addr = format!("127.0.0.1:{port}").parse().unwrap();
    let start = Instant::now();
    let (sa, sb) = (
        net::connect_socket(addr).unwrap(),
        net::connect_socket(addr).unwrap(),
    );
    let mut a = Client::new(PlayerInfo::default(), "", net::random_bytes);
    let mut b = Client::new(PlayerInfo::default(), "", net::random_bytes);
    a.connect(net::now(start));
    b.connect(net::now(start));
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline && !(local_x(&a).is_some() && local_x(&b).is_some()) {
        net::pump(&sa, &mut a, net::now(start)).unwrap();
        net::pump(&sb, &mut b, net::now(start)).unwrap();
        std::thread::sleep(Duration::from_millis(1));
    }
    let map = Map::load(a.map().unwrap().data.clone()).unwrap();
    let col = Collision::new(&map);
    let mut pred = Predictor::new(Rc::new(col), &map);
    let (ida, idb) = (a.local_id().unwrap(), b.local_id().unwrap());
    let entered = net::now(start);
    // per tee: last rendered and last snapshot position, largest step of each, frames
    let mut track = [(None, None, 0f32, 0f32, 0u32); 2];
    // the previous frame's IntraGameTick and snapshot tick; late switch frames per tee
    let (mut last_intra, mut last_tick, mut late) = (0f32, None, [0u32; 2]);
    while net::now(start) - entered < 10 * FREQ {
        let now = net::now(start);
        net::scripted_input(&mut a.input, now - entered);
        net::scripted_input(&mut b.input, now - entered + FREQ);
        net::pump(&sa, &mut a, now).unwrap();
        net::pump(&sb, &mut b, now).unwrap();
        pred.update(&mut a);
        let rendered = pred.rendered(&a);
        // a tee that left the snapshot (out of range) starts over when it returns
        for (k, id) in [ida, idb].into_iter().enumerate() {
            if !rendered.iter().any(|r| r.id == id) {
                (track[k].0, track[k].1) = (None, None);
            }
        }
        for r in rendered {
            let k = if r.id == ida {
                0
            } else if r.id == idb {
                1
            } else {
                continue;
            };
            assert_eq!(r.predicted, r.id == ida);
            let (last, last_snap, max, max_snap, frames) = &mut track[k];
            let snap = (r.cur.x as f32, r.cur.y as f32);
            let step = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).hypot(a.1 - b.1);
            let late_switch = !r.predicted && last_intra > 1.0 && a.game_tick() != last_tick;
            if let (Some(l), Some(ls)) = (*last, *last_snap) {
                let d = step(l, (r.pos.x, r.pos.y));
                if late_switch {
                    late[k] += 1;
                    assert!(d <= step(ls, snap).max(*max_snap), "late switch step {d}");
                } else {
                    *max = max.max(d);
                }
                *max_snap = max_snap.max(step(ls, snap));
                *frames += 1;
            }
            (*last, *last_snap) = (Some((r.pos.x, r.pos.y)), Some(snap));
        }
        (last_intra, last_tick) = (a.intra_tick, a.game_tick());
        std::thread::sleep(Duration::from_millis(1));
    }
    eprintln!(
        "local (predicted) and other: (max step, max step without interpolation, frames) = {:?}, late switches: {late:?}",
        track.map(|t| (t.2, t.3, t.4))
    );
    for (k, (max, max_snap, frames)) in track.map(|t| (t.2, t.3, t.4)).into_iter().enumerate() {
        assert!(frames > 2000, "{frames}");
        assert!(
            late[k] * 20 < frames,
            "{} of {frames} frames are late switches",
            late[k]
        );
        // the test can tell: without interpolation steps are whole ticks
        assert!(max_snap >= 10.0, "{max_snap}");
        assert!(max <= max_snap / 2.0, "{max} vs {max_snap}");
    }
    drop(server);
}

/// M6.7: the server takes our `Cl_ShowDistance` and `Cl_CameraInfo` and clips by them.
/// C++: `CPlayer::UpdateNetworkClipRadius` (player.cpp:1124; with camera info the radius
/// is ShowDistance / 2 + 320, the dyncam offset is 0 with default settings) and
/// `CCharacter::IsSnappingCharacterInView` (a square of the larger radius around the
/// tee-to-hook line; the hook is idle here). Two tees stand at their spawns while one
/// steps its zoom from 0.4 to 2.5: the other tee must be in its snapshots exactly when
/// the rule says so, and must appear or vanish on the way. Without the show distance
/// the radius would not follow the zoom; without the camera info it would be the whole
/// show distance. The teehistorian holds the exact bytes we sent.
#[test]
fn camera_messages_set_network_clipping() {
    use ddnet::net::client::calc_screen_params;
    use ddnet::physics::vmath::trunc_i32;
    use ddnet::protocol::packer::Packer;

    let Some(exe) = server_binary() else {
        eprintln!("SKIPPED: no DDNet-Server (set DDNET_SERVER or build the game-server target)");
        return;
    };
    let (server, port) = start_server(&exe, "dm1");
    let addr = format!("127.0.0.1:{port}").parse().unwrap();
    let start = Instant::now();
    let (sa, sb) = (
        net::connect_socket(addr).unwrap(),
        net::connect_socket(addr).unwrap(),
    );
    let mut a = Client::new(PlayerInfo::default(), "", net::random_bytes);
    let mut b = Client::new(PlayerInfo::default(), "", net::random_bytes);
    a.camera.zoom = 0.4;
    a.connect(net::now(start));
    b.connect(net::now(start));
    let run = |a: &mut Client, b: &mut Client, secs: f32| {
        let until = Instant::now() + Duration::from_secs_f32(secs);
        while Instant::now() < until {
            net::pump(&sa, a, net::now(start)).unwrap();
            net::pump(&sb, b, net::now(start)).unwrap();
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline && (local_x(&a).is_none() || local_x(&b).is_none()) {
        run(&mut a, &mut b, 0.01);
    }
    let pos = |c: &Client, id: i32| {
        c.snapshot()
            .iter()
            .find(|it| it.type_id == g::NETOBJTYPE_CHARACTER && it.id == id)
            .map(|it| g::ObjCharacter::from_ints(&it.data))
    };
    let (ida, idb) = (a.local_id().unwrap(), b.local_id().unwrap());
    // land at the spawns
    let still = |c: &Client, id: i32| pos(c, id).is_some_and(|p| (p.vel_x, p.vel_y) == (0, 0));
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && !(still(&a, ida) && still(&b, idb)) {
        run(&mut a, &mut b, 0.05);
    }
    let (mut sent, mut seen) = (Vec::new(), Vec::new());
    for step in 0..=21 {
        a.camera.zoom = 0.4 + step as f32 * 0.1;
        sent.push(a.camera.zoom);
        run(&mut a, &mut b, 0.4); // sent with the next snapshot, then a few ticks
        let (pa, pb) = (pos(&a, ida).unwrap(), pos(&b, idb).unwrap());
        assert_eq!(
            (pa.vel_x, pa.vel_y, pb.hook_state, pb.vel_x, pb.vel_y),
            (0, 0, 0, 0, 0),
            "both must stand still"
        );
        let (w, h) = calc_screen_params(16.0 / 9.0, a.camera.zoom);
        let radius = (trunc_i32(w) as f32 / 2.0 + 320.0).max(trunc_i32(h) as f32 / 2.0 + 320.0);
        let d = (pa.x - pb.x).abs().max((pa.y - pb.y).abs()) as f32;
        let visible = pos(&a, idb).is_some();
        if (d - radius).abs() >= 2.0 {
            assert_eq!(
                visible,
                d <= radius,
                "zoom {}: distance {d}, radius {radius}",
                a.camera.zoom
            );
        }
        seen.push(visible);
    }
    eprintln!("visible per zoom step: {seen:?}");
    assert!(
        seen.contains(&true) && seen.contains(&false),
        "the zoom steps must cross the distance"
    );

    // C++ CGameContext::OnMessage records the raw message: ex id 0, UUID, body
    let uuid = |id: i32| g::MSG_TYPES.iter().find(|m| m.0 == id).unwrap().2.unwrap();
    let msg = |id: i32, body: &dyn Fn(&mut Packer)| {
        let mut p = Packer::default();
        p.add_int(0);
        p.add_raw(&uuid(id));
        body(&mut p);
        p.data().to_vec()
    };
    drop((a, b));
    std::thread::sleep(Duration::from_millis(200));
    let recorded: Vec<u8> = fs::read_dir(server.dir.join("teehistorian"))
        .unwrap()
        .flat_map(|f| fs::read(f.unwrap().path()).unwrap())
        .collect();
    for zoom in sent {
        let (x, y) = calc_screen_params(16.0 / 9.0, zoom);
        let show = msg(g::MsgClShowDistance::ID, &|p| {
            g::MsgClShowDistance {
                x: trunc_i32(x),
                y: trunc_i32(y),
            }
            .pack(p)
        });
        let info = msg(g::MsgClCameraInfo::ID, &|p| {
            g::MsgClCameraInfo {
                zoom: trunc_i32(zoom * 1000.0),
                deadzone: 0,
                follow_factor: 0,
            }
            .pack(p)
        });
        for e in [show, info] {
            assert!(
                recorded.windows(e.len()).any(|w| w == &e[..]),
                "zoom {zoom}: not in teehistorian: {e:02x?}"
            );
        }
    }
    drop(server);
}
