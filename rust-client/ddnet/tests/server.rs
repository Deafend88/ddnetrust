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
    start_server_with(exe, map, &[], &[])
}

/// `start_server` with `map_cfg` as the map's config (`maps/<map>.cfg`): the server
/// imports it into the map's settings, so it runs after the game type reset the tuning,
/// and clients load it too (`LoadMapSettings`).
/// `args` are extra server command-line settings.
fn start_server_with(exe: &PathBuf, map: &str, map_cfg: &[&str], args: &[&str]) -> (Server, u16) {
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
        .args(args)
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
        client.conn().error()
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
        let Some((pred, _, weak_col)) = check_prediction(map_name, &[], None, false) else {
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
        let Some((mut pred, local, _)) = check_prediction(map_name, &map_cfg, chat, false) else {
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
    antiping: bool,
) -> Option<(Predictor, i32, std::rc::Weak<Collision>)> {
    let Some(exe) = server_binary() else {
        eprintln!("SKIPPED: no DDNet-Server (set DDNET_SERVER or build the game-server target)");
        return None;
    };
    // antiping: the rcon test command `weapons` gives every weapon
    let args: &[&str] = if antiping {
        &["sv_test_cmds 1", "sv_rcon_password test"]
    } else {
        &[]
    };
    let (server, port) = start_server_with(&exe, map_name, map_cfg, args);
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
    pred.config.antiping = antiping;
    let entered = net::now(start);
    if let Some(line) = chat {
        client.say(0, line, entered);
    }
    let mut weapons_sent = 0;
    let mut zones = std::collections::BTreeSet::new();
    // antiping: our projectiles and lasers in snapshots, and how many of them the
    // prediction world holds (C++ FindMatch), per weapon
    let (mut own, mut found) = ([0u32; 5], [0u32; 5]);
    while net::now(start) - entered < 10 * FREQ {
        let elapsed = net::now(start) - entered;
        // the rcon test command `weapons` once the tee has spawned, and every second
        // (the map takes them away again)
        if antiping && elapsed > FREQ && elapsed / FREQ > weapons_sent {
            if weapons_sent == 0 {
                client.rcon_auth("test", net::now(start));
            }
            client.rcon("weapons", net::now(start));
            weapons_sent = elapsed / FREQ;
        }
        net::scripted_input(&mut client.input, elapsed);
        if antiping {
            fire_script(&mut client.input, elapsed, map_name == "coverage");
        }
        net::pump(&socket, &mut client, net::now(start)).unwrap();
        let switched = !client.snapshot_switches.is_empty();
        pred.update(&mut client, net::now(start));
        zones.insert(pred.tune_state().1);
        if antiping && switched {
            count_own_entities(&client, &pred, &mut own, &mut found);
        }

        std::thread::sleep(Duration::from_millis(1));
    }
    let s = &pred.stats;
    eprintln!(
        "{map_name} ({} map config lines, chat {chat:?}): {s:?}, tune zones {zones:?}, own entities {own:?}, in the prediction world {found:?}",
        map_cfg.len()
    );
    if antiping {
        // gun, shotgun, grenade and laser all fired
        assert!(own[1..].iter().all(|&n| n > 0), "{map_name}: {own:?}");
        assert_eq!(own, found, "{map_name}");
    }
    assert!(s.compared > 100, "{map_name}: {s:?}");
    assert_eq!(s.mismatched, 0, "{map_name}: {s:?}");
    assert_eq!(s.prediction_errors, 0, "{map_name}: {s:?}");
    let local = client.local_id().unwrap_or(-1);
    let dir = server.dir.clone();
    drop(server);
    let _ = fs::remove_dir_all(dir);
    Some((pred, local, weak_col))
}

/// Fire on top of the scripted movement: gun, shotgun, grenade and laser in turn
/// (2 s each), fire toggled every 150 ms; with `down` aiming down half the time
/// (blasts next to the tee).
fn fire_script(input: &mut g::ObjPlayerInput, elapsed: i64, down: bool) {
    let weapon = 1 + (elapsed / (2 * FREQ)) % 4;
    input.wanted_weapon = weapon as i32 + 1;
    let presses = (elapsed / (FREQ * 15 / 100)) as i32;
    input.fire = presses & ddnet::physics::core::INPUT_STATE_MASK;
    if down && (elapsed / (FREQ / 2)) % 2 == 0 {
        input.target_y = 150;
    }
}

/// Counts the current snapshot's projectiles and lasers owned by the local player (by
/// weapon) and those the prediction world holds (C++ `FindMatch`).
fn count_own_entities(client: &Client, pred: &Predictor, own: &mut [u32; 5], found: &mut [u32; 5]) {
    use ddnet::items::{extract_laser, extract_projectile};
    let Some(local) = client.local_id() else {
        return;
    };
    for it in client.snapshot() {
        let weapon = if it.type_id == g::NETOBJTYPE_DDNETPROJECTILE {
            let d = extract_projectile(it.type_id, &it.data, None, None);
            (d.owner == local).then_some(d.type_)
        } else if it.type_id == g::NETOBJTYPE_DDNETLASER {
            let d = extract_laser(it.type_id, &it.data, None, None);
            let laser = d.type_ == g::LASERTYPE_RIFLE || d.type_ == g::LASERTYPE_SHOTGUN;
            let weapon = if d.type_ == g::LASERTYPE_SHOTGUN {
                g::WEAPON_SHOTGUN
            } else {
                g::WEAPON_LASER
            };
            (laser && d.owner == local).then_some(weapon)
        } else {
            None
        };
        if let Some(w) = weapon
            .and_then(|w| usize::try_from(w).ok())
            .filter(|&w| w < 5)
        {
            own[w] += 1;
            let mut data = it.data.clone();
            data.resize(g::ObjDDNetLaser::NUM_INTS, 0);
            found[w] += pred.world().find_match(it.id, it.type_id, &data).is_some() as u32;
        }
    }
}

/// 8.5: antiping on (`cl_antiping 1`: weapons, other players) while firing every
/// weapon: the local tee still matches every snapshot, and every own projectile and
/// laser of a snapshot is in the prediction world.
///
/// Explosions push nobody here (`explosion_strength 0`): C++ predicts a grenade from the
/// snapshot, whose position is rounded to 1/100 (projectile.cpp:355), so a blast that
/// reaches the tee can be one velocity step (1/256) off in C++ too; and the knockback made
/// the path timing-dependent, sometimes into a dm1 death tile, which freezes the tee in
/// the test commands' practice mode before it fires grenades and lasers. The force itself
/// is checked against C++ at full strength by `rs_parity antiping`.
#[test]
fn prediction_matches_server_with_antiping() {
    for map_name in ["dm1", "coverage"] {
        if check_prediction(map_name, &["tune explosion_strength 0"], None, true).is_none() {
            return;
        }
    }
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
        pred.update(&mut a, net::now(start));
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
        pred.update(&mut a, net::now(start));
        let rendered = pred.rendered(&a.timing(net::now(start)));
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

/// 8.6: `cl_antiping_smooth` on a second tee that changes direction and jumps at random
/// every 50-300 ms (its preinputs come late or not at all, so it mispredicts). Logs the
/// largest per-frame step of its drawn position with smoothing (`GetSmoothPos`) and
/// without (between its predicted cores), and how many smoothing windows started.
#[test]
fn antiping_smoothing_steps() {
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
    let mut pred = Predictor::new(Rc::new(Collision::new(&map)), &map);
    pred.config.antiping = true;
    pred.config.antiping_smooth = true;
    let idb = b.local_id().unwrap() as usize;
    let entered = net::now(start);
    let mut seed = 12345u32;
    let mut next_change = entered;
    // (last smooth pos, last raw pos, max smooth step, max raw step, frames, windows)
    let (mut last, mut max, mut frames, mut windows) = (None, (0f32, 0f32), 0u32, 0u32);
    while net::now(start) - entered < 10 * FREQ {
        let now = net::now(start);
        net::scripted_input(&mut a.input, now - entered);
        if now >= next_change {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            b.input.direction = (seed >> 16) as i32 % 3 - 1;
            b.input.jump = (seed >> 20).is_multiple_of(4) as i32;
            next_change = now + FREQ / 20 + (seed >> 8) as i64 % (FREQ / 4);
        }
        net::pump(&sa, &mut a, now).unwrap();
        net::pump(&sb, &mut b, now).unwrap();
        let before = pred.smooth_start[idb];
        pred.update(&mut a, now);
        windows += (pred.smooth_start[idb] != before) as u32;
        let in_world = (pred.predicted_world.as_ref()).is_some_and(|w| w.character(idb).is_some());
        if !in_world {
            last = None;
            continue;
        }
        let t = a.timing(now);
        let smooth = pred.smooth_pos(idb, &t);
        let raw = ddnet::physics::vmath::mix(
            pred.prev_predicted[idb].pos,
            pred.predicted[idb].pos,
            t.pred_intra_tick,
        );
        if let Some((ls, lr)) = last {
            let step = |p: ddnet::physics::vmath::Vec2, q: ddnet::physics::vmath::Vec2| {
                (p.x - q.x).hypot(p.y - q.y)
            };
            max = (max.0.max(step(smooth, ls)), max.1.max(step(raw, lr)));
            frames += 1;
        }
        last = Some((smooth, raw));
        std::thread::sleep(Duration::from_millis(1));
    }
    eprintln!(
        "other tee, largest per-frame step: smoothing on {:.2} px, off {:.2} px; {frames} frames, {windows} smoothing windows",
        max.0, max.1
    );
    assert!(frames > 1000, "{frames}");
    assert!(max.0.is_finite() && max.1.is_finite());
    drop(server);
}

/// 13.5: the Esc menu's messages reach the server like C++'s (`SendSwitchTeam`,
/// `SendKill`, `say /pause`), and the current server's info arrives (`ServerInfo()`).
#[test]
fn esc_menu_messages_reach_server() {
    let Some(exe) = server_binary() else {
        eprintln!("SKIPPED: no DDNet-Server (set DDNET_SERVER or build the game-server target)");
        return;
    };
    let (server, port) = start_server_with(&exe, "coverage", &[], &["sv_name \"  rs   test \""]);
    let socket = net::connect_socket(format!("127.0.0.1:{port}").parse().unwrap()).unwrap();
    let start = Instant::now();
    let mut client = Client::new(PlayerInfo::default(), "", net::random_bytes);
    client.connect(net::now(start));
    let run = |client: &mut Client, secs: u64, done: &dyn Fn(&Client) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < deadline && !done(client) {
            net::pump(&socket, client, net::now(start)).unwrap();
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    run(&mut client, 15, &|c| {
        local_x(c).is_some() && c.server_info().is_some()
    });
    let info = client.server_info().expect("server info");
    // the name's spaces cleaned like C++ str_clean_whitespaces
    assert_eq!(
        (info.name.as_str(), info.map.as_str()),
        ("rs test", "coverage")
    );
    assert!(info.version.contains("0.6"), "{info:?}");
    let log = || fs::read_to_string(server.dir.join("server.log")).unwrap();
    // the local player's team and DDNet player flags in the current snapshot
    let state = |c: &Client| {
        let id = c.local_id()?;
        let item = |t| {
            c.snapshot()
                .iter()
                .find(|it| it.type_id == t && it.id == id)
        };
        let team = g::ObjPlayerInfo::from_ints(&item(g::NETOBJTYPE_PLAYERINFO)?.data).team;
        let flags = item(g::NETOBJTYPE_DDNETPLAYER).map_or(0, |it| it.data[0]);
        Some((team, flags))
    };
    type Step<'a> = (&'a dyn Fn(&mut Client, i64), &'a dyn Fn(&Client) -> bool);
    let steps: [Step; 4] = [
        (
            &|c, now| c.send_switch_team(g::TEAM_SPECTATORS, now),
            &|c| state(c).is_some_and(|s| s.0 == g::TEAM_SPECTATORS),
        ),
        (&|c, now| c.send_switch_team(g::TEAM_RED, now), &|c| {
            state(c).is_some_and(|s| s.0 == g::TEAM_RED)
        }),
        (
            &|c, now| c.send_kill(now),
            // C++ WEAPON_SELF is -2
            &|_| log().contains("victim='0:nameless tee' weapon=-2"),
        ),
        (&|c, now| c.say(0, "/pause", now), &|c| {
            state(c).is_some_and(|s| s.1 & g::EXPLAYERFLAG_PAUSED != 0)
        }),
    ];
    for (n, (send, done)) in steps.into_iter().enumerate() {
        // the server rate-limits team changes and kills (sv_team_change_delay etc.)
        run(&mut client, 4, &|_| false);
        send(&mut client, net::now(start));
        run(&mut client, 3, done);
        assert!(
            done(&client),
            "step {n}: {:?}
{}",
            state(&client),
            log()
        );
    }
    drop(server);
}

/// 13.6: the dummy joins as a second player on its own connection, becomes the active
/// tee (C++ sets `cl_dummy 1` on its `CON_READY`), walks while the main tee stands, and
/// after swapping back the main tee walks; with `cl_dummy_hammer` the other tee hammers.
#[test]
fn dummy_joins_walks_and_swaps() {
    let Some(exe) = server_binary() else {
        eprintln!("SKIPPED: no DDNet-Server (set DDNET_SERVER or build the game-server target)");
        return;
    };
    // both tees in every snapshot, wherever they spawn
    let (server, port) = start_server_with(&exe, "dm1", &[], &["sv_show_all_default 1"]);
    let addr = format!("127.0.0.1:{port}").parse().unwrap();
    let sockets = [
        net::connect_socket(addr).unwrap(),
        net::connect_socket(addr).unwrap(),
    ];
    let start = Instant::now();
    let player = PlayerInfo {
        name: "main".into(),
        ..PlayerInfo::default()
    };
    let mut client = Client::new(player, "", net::random_bytes);
    client.connect(net::now(start));
    let run = |client: &mut Client, ms: u64, done: &dyn Fn(&Client) -> bool| {
        let deadline = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < deadline && !done(client) {
            net::pump_both(&[&sockets[0], &sockets[1]], client, net::now(start)).unwrap();
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    run(&mut client, 15_000, &|c| local_x(c).is_some());
    assert_eq!(client.dummy_connect(net::now(start)), Ok(()));
    assert_eq!(
        client.dummy_connect(net::now(start)),
        Err("Dummy is already connecting.")
    );
    run(&mut client, 10_000, &|c| {
        c.dummy() && c.local_ids().iter().all(Option::is_some) && local_x(c).is_some()
    });
    assert!(
        client.dummy_connected() && client.dummy(),
        "{:?}",
        client.dummy_error
    );
    let [main, dummy] = client.local_ids().map(Option::unwrap);
    assert_ne!(main, dummy);
    assert_eq!(client.local_id(), Some(dummy));
    let log = fs::read_to_string(server.dir.join("server.log")).unwrap();
    assert!(log.contains("'[D] main'"), "{log}");
    let x = |c: &Client, id| character(c, id).map(|ch| ch.x);
    // both tees are in the dummy's snapshot
    run(&mut client, 1_000, &|_| false);
    let (m0, d0) = (x(&client, main).unwrap(), x(&client, dummy).unwrap());
    client.input.direction = 1;
    run(&mut client, 1_000, &|_| false);
    client.input.direction = 0;
    // until both cores rest (a tee may have walked off a ledge; the server resends a
    // dead-reckoned core within 3 s)
    let still = |c: &Client| {
        [main, dummy]
            .iter()
            .all(|&id| character(c, id).is_some_and(|ch| ch.vel_x.abs() < 128 && ch.vel_y == 0))
    };
    run(&mut client, 4_500, &still);
    assert!(still(&client));
    let (m1, d1) = (x(&client, main).unwrap(), x(&client, dummy).unwrap());
    assert!(
        d1 > d0 + 100 && m1 == m0,
        "dummy {d0}->{d1}, main {m0}->{m1}"
    );
    // swap back: the main tee is active again
    client.set_dummy(false);
    run(&mut client, 300, &|c| c.local_id() == Some(main));
    assert_eq!(client.local_id(), Some(main));
    client.input.direction = -1;
    run(&mut client, 1_000, &|_| false);
    client.input.direction = 0;
    run(&mut client, 4_500, &still);
    assert!(still(&client));
    let (m2, d2) = (x(&client, main).unwrap(), x(&client, dummy).unwrap());
    assert!(
        m2 < m1 - 100 && d2 == d1,
        "main {m1}->{m2}, dummy {d1}->{d2}"
    );
    // the dummy hammers the main tee (C++ OnSnapInput with cl_dummy_hammer)
    client.dummy_hammer = true;
    run(&mut client, 2_000, &|_| false);
    let hammer = |c: &Client| character(c, dummy).map(|ch| (ch.weapon, ch.attack_tick));
    assert_eq!(hammer(&client).map(|h| h.0), Some(g::WEAPON_HAMMER));
    let t0 = hammer(&client).unwrap().1;
    run(&mut client, 1_000, &|c| {
        hammer(c).is_some_and(|h| h.1 != t0)
    });
    assert_ne!(hammer(&client).unwrap().1, t0, "the dummy keeps hammering");
    client.dummy_hammer = false;
    // disconnecting the dummy leaves the main tee playing
    client.dummy_disconnect(None, net::now(start));
    run(&mut client, 1_000, &|_| false);
    assert!(!client.dummy_connected() && !client.dummy());
    assert_eq!(client.state(), State::Online);
    assert!(character(&client, dummy).is_none(), "the dummy left");
    drop(server);
}

/// The chunks a client sent in one datagram, one string per chunk ("V" for vital):
/// the message name and its payload, the payload left out where it depends on the
/// client version, the moment or random ids. `None` for connless datagrams.
fn describe_sent(datagram: &[u8]) -> Option<Vec<String>> {
    use ddnet::protocol::packer::Unpacker;
    use ddnet::protocol::packet::{
        self, CHUNKFLAG_VITAL, ChunkHeader, PACKETFLAG_CONNLESS, PACKETFLAG_CONTROL,
    };
    let p = packet::decode(datagram)?;
    if p.flags & PACKETFLAG_CONNLESS != 0 {
        return None;
    }
    if p.flags & PACKETFLAG_CONTROL != 0 {
        return Some(vec![format!("ctrl {}", p.data[0])]);
    }
    const SYS: [&str; 30] = [
        "?",
        "INFO",
        "MAP_CHANGE",
        "MAP_DATA",
        "CON_READY",
        "SNAP",
        "SNAPEMPTY",
        "SNAPSINGLE",
        "SNAPSMALL",
        "INPUTTIMING",
        "RCON_AUTH_STATUS",
        "RCON_LINE",
        "AUTH_CHALLENGE",
        "AUTH_RESULT",
        "READY",
        "ENTERGAME",
        "INPUT",
        "RCON_CMD",
        "RCON_AUTH",
        "REQUEST_MAP_DATA",
        "AUTH_START",
        "AUTH_RESPONSE",
        "PING",
        "PING_REPLY",
        "ERROR",
        "RCON_CMD_ADD",
        "RCON_CMD_REM",
        "?",
        "?",
        "?",
    ];
    let mut out = Vec::new();
    let mut data = &p.data[..];
    for _ in 0..p.num_chunks {
        let (h, len) = ChunkHeader::unpack(data)?;
        let body = data.get(len..len + h.size)?;
        data = &data[len + h.size..];
        let mut u = Unpacker::new(body);
        let msg = u.get_int();
        let (id, sys) = (msg >> 1, msg & 1 != 0);
        let name = if id == 0 {
            let uuid: [u8; 16] = u.get_raw(16)?.try_into().ok()?;
            (g::SYS_EX_MSGS.iter().find(|m| m.2 == uuid).map(|m| m.1))
                .or_else(|| g::MSG_TYPES.iter().find(|m| m.2 == Some(uuid)).map(|m| m.1))
                .unwrap_or("?ex")
                .to_string()
        } else if sys {
            SYS.get(id as usize).unwrap_or(&"?").to_string()
        } else {
            (g::MSG_TYPES.iter().find(|m| m.0 == id && m.2.is_none()))
                .map_or(format!("game{id}"), |m| m.1.to_string())
        };
        // the message id's length: the varint, plus the UUID of extended messages
        let mut pk = ddnet::protocol::packer::Packer::default();
        pk.add_int(msg);
        let rest = body.get(pk.data().len() + if id == 0 { 16 } else { 0 }..)?;
        let payload = match name.as_str() {
            // version and random ids; ticks
            "clientver@ddnet.tw" | "pong@ddnet.tw" | "ping@ddnet.tw" | "Cl_IsDDNetLegacy" => {
                String::new()
            }
            // the input without its ack and prediction ticks
            "INPUT" => {
                let mut u = Unpacker::new(rest);
                let (_, _, size) = (u.get_int(), u.get_int(), u.get_int());
                let ints: Vec<String> = (0..size / 4).map(|_| u.get_int().to_string()).collect();
                ints.join(",")
            }
            _ => rest.iter().map(|b| format!("{b:02x}")).collect(),
        };
        let vital = if h.flags & CHUNKFLAG_VITAL != 0 {
            "V "
        } else {
            ""
        };
        out.push(format!("{vital}{name} {payload}").trim_end().to_string());
    }
    Some(out)
}

/// The dummy connection's datagrams in a `udp_tap` log (client 1, sent to the server),
/// described.
fn tap_dummy_sent(log: &str) -> Vec<Vec<String>> {
    log.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(' ').collect();
            (f[1] == "1" && f[2] == "c2s").then(|| {
                (0..f[3].len() / 2)
                    .map(|i| u8::from_str_radix(&f[3][2 * i..2 * i + 2], 16).unwrap())
                    .collect::<Vec<u8>>()
            })
        })
        .filter_map(|d| describe_sent(&d))
        .collect()
}

/// 13.6: our dummy connection sends what the C++ client's sends, chunk for chunk and
/// datagram for datagram, from the connect up to its first input (`dummy_cpp.tap`:
/// the C++ client's dummy through `udp_tap`, `scripts/dummy_capture.ps1`).
#[test]
fn dummy_handshake_matches_cpp() {
    let Some(exe) = server_binary() else {
        eprintln!("SKIPPED: no DDNet-Server (set DDNET_SERVER or build the game-server target)");
        return;
    };
    let golden = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/golden/dummy_cpp.tap");
    let cpp = tap_dummy_sent(&fs::read_to_string(golden).unwrap());
    let (server, port) = start_server(&exe, "dm1");
    let addr = format!("127.0.0.1:{port}").parse().unwrap();
    let sockets = [
        net::connect_socket(addr).unwrap(),
        net::connect_socket(addr).unwrap(),
    ];
    let socks = [&sockets[0], &sockets[1]];
    let start = Instant::now();
    let player = PlayerInfo {
        name: "main".into(),
        ..PlayerInfo::default()
    };
    let mut client = Client::new(player, "", net::random_bytes);
    // the C++ client's window: 800x600, default zoom
    client.camera.aspect = 800.0 / 600.0;
    client.connect(net::now(start));
    let mut sent = Vec::new();
    let frame = |client: &mut Client, sent: &mut Vec<Vec<String>>| {
        let incoming = net::recv_all(&socks).unwrap();
        client.frame(incoming, net::now(start));
        sent.extend(client.outbox(1).iter().filter_map(|d| describe_sent(d)));
        net::send_all(&socks, client).unwrap();
        std::thread::sleep(Duration::from_millis(1));
    };
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline && local_x(&client).is_none() {
        frame(&mut client, &mut sent);
    }
    client.dummy_connect(net::now(start)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let has_input = |s: &[Vec<String>]| s.iter().any(|p| p.iter().any(|c| c.starts_with("INPUT")));
    while Instant::now() < deadline && !has_input(&sent) {
        frame(&mut client, &mut sent);
    }
    drop(server);
    // one line per datagram
    let upto = |s: &[Vec<String>]| {
        let n = s
            .iter()
            .position(|p| p.iter().any(|c| c.starts_with("INPUT")));
        let lines: Vec<String> = s[..n.map_or(s.len(), |n| n + 1)]
            .iter()
            .map(|p| p.join(" | "))
            .collect();
        lines.join(
            "
",
        )
    };
    assert!(has_input(&cpp) && has_input(&sent));
    assert_eq!(upto(&sent), upto(&cpp));
}

/// 13.6: prediction with the dummy: both tees walk the scripted path (the other tee's
/// input mirrored), 5 s with the dummy active, then swapped back to the main tee for
/// 5 s. Every snapshot of the active tee must match its prediction.
#[test]
fn prediction_matches_server_with_dummy() {
    let Some(exe) = server_binary() else {
        eprintln!("SKIPPED: no DDNet-Server (set DDNET_SERVER or build the game-server target)");
        return;
    };
    let (server, port) = start_server_with(&exe, "dm1", &[], &["sv_show_all_default 1"]);
    let addr = format!("127.0.0.1:{port}").parse().unwrap();
    let sockets = [
        net::connect_socket(addr).unwrap(),
        net::connect_socket(addr).unwrap(),
    ];
    let socks = [&sockets[0], &sockets[1]];
    let start = Instant::now();
    let mut client = Client::new(PlayerInfo::default(), "", net::random_bytes);
    client.connect(net::now(start));
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline && local_x(&client).is_none() {
        net::pump_both(&socks, &mut client, net::now(start)).unwrap();
        std::thread::sleep(Duration::from_millis(1));
    }
    client.dummy_connect(net::now(start)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline
        && !(client.dummy()
            && client.local_ids().iter().all(Option::is_some)
            && local_x(&client).is_some())
    {
        net::pump_both(&socks, &mut client, net::now(start)).unwrap();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(client.dummy(), "dummy did not connect");
    let map = Map::load(client.map().unwrap().data.clone()).unwrap();
    let mut pred = Predictor::new(Rc::new(Collision::new(&map)), &map);
    let entered = net::now(start);
    let mut halves = Vec::new();
    for active_dummy in [true, false] {
        client.set_dummy(active_dummy);
        let before = pred.stats.clone();
        let from = net::now(start);
        while net::now(start) - from < 5 * FREQ {
            let elapsed = net::now(start) - entered;
            net::scripted_input(&mut client.input, elapsed);
            // the other tee walks the other way
            net::scripted_input(&mut client.dummy_snap.input, elapsed + 2 * FREQ);
            let incoming = net::recv_all(&socks).unwrap();
            client.pump_network(incoming, net::now(start));
            if client.swapping() {
                pred.is_dummy_swapping = true;
            }
            client.update(net::now(start));
            net::send_all(&socks, &mut client).unwrap();
            pred.update(&mut client, net::now(start));
            std::thread::sleep(Duration::from_millis(1));
        }
        let s = &pred.stats;
        halves.push((
            s.compared - before.compared,
            s.mismatched - before.mismatched,
            s.prediction_errors - before.prediction_errors,
        ));
        assert!(
            pred.predicted_dummy_id().is_some(),
            "the other tee is predicted"
        );
    }
    eprintln!("dummy active / main active: (compared, mismatched, errors) {halves:?}");
    for &(compared, mismatched, errors) in &halves {
        assert!(compared > 100, "{halves:?}");
        assert_eq!((mismatched, errors), (0, 0), "{halves:?}");
    }
    drop(server);
}

/// 13.7: votes like C++ `CVoting`: the server's option list arrives, one player calls an
/// option vote (`Cl_CallVote`), the other sees it (`Sv_VoteSet`) and votes yes
/// (`Cl_Vote`): the vote passes.
#[test]
fn votes_reach_server() {
    let Some(exe) = server_binary() else {
        eprintln!("SKIPPED: no DDNet-Server (set DDNET_SERVER or build the game-server target)");
        return;
    };
    let args = ["add_vote \"Say hi\" \"say hi\"", "sv_vote_spectate 0"];
    let (server, port) = start_server_with(&exe, "dm1", &[], &args);
    let addr = format!("127.0.0.1:{port}").parse().unwrap();
    let sockets = [
        net::connect_socket(addr).unwrap(),
        net::connect_socket(addr).unwrap(),
    ];
    let start = Instant::now();
    let mut clients = [0, 1].map(|i| {
        let name = format!("voter{i}");
        let mut c = Client::new(
            PlayerInfo {
                name,
                ..PlayerInfo::default()
            },
            "",
            net::random_bytes,
        );
        c.connect(net::now(start));
        c
    });
    let mut log: [Vec<g::GameMsg>; 2] = Default::default();
    let run = |clients: &mut [Client; 2], log: &mut [Vec<g::GameMsg>; 2], ms: u64| {
        let deadline = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < deadline {
            for i in 0..2 {
                net::pump(&sockets[i], &mut clients[i], net::now(start)).unwrap();
                log[i].append(&mut clients[i].messages);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    // the server's data autoexec adds the map votes first; ours comes last
    let has_option = |log: &[g::GameMsg]| {
        log.iter().any(|m| match m {
            g::GameMsg::SvVoteOptionListAdd(l) => {
                let all = [
                    &l.description0,
                    &l.description1,
                    &l.description2,
                    &l.description3,
                    &l.description4,
                    &l.description5,
                    &l.description6,
                    &l.description7,
                    &l.description8,
                    &l.description9,
                    &l.description10,
                    &l.description11,
                    &l.description12,
                    &l.description13,
                    &l.description14,
                ];
                all[..l.num_options as usize].iter().any(|d| *d == "Say hi")
            }
            g::GameMsg::SvVoteOptionAdd(o) => o.description == "Say hi",
            _ => false,
        })
    };
    for _ in 0..30 {
        run(&mut clients, &mut log, 500);
        if has_option(&log[0]) {
            break;
        }
    }
    assert!(clients.iter().all(|c| local_x(c).is_some()), "both joined");
    assert!(has_option(&log[0]), "the option list arrived");
    clients[0].call_vote("option", "Say hi", "", net::now(start));
    run(&mut clients, &mut log, 1000);
    let set = log[1].iter().any(
        |m| matches!(m, g::GameMsg::SvVoteSet(s) if s.timeout > 0 && s.description == "Say hi"),
    );
    assert!(set, "the other player sees the vote");
    clients[1].vote(1, net::now(start));
    run(&mut clients, &mut log, 1500);
    let passed = log[1]
        .iter()
        .any(|m| matches!(m, g::GameMsg::SvChat(c) if c.message == "Vote passed"));
    assert!(
        passed,
        "{}",
        fs::read_to_string(server.dir.join("server.log")).unwrap()
    );
    drop(server);
}
