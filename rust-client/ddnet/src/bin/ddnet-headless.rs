//! Headless DDNet client: connects, downloads the map, enters the game, optionally
//! walks right or runs a movement script, then disconnects cleanly.
//!
//! Usage: ddnet-headless <server ip:port> [--name NAME] [--seconds N] [--walk SECONDS] [--script]
//!        [--fire WEAPON] [--aim X,Y]
//!
//! `--script` repeats a 4 second loop: walk right with a jump and a hook up-right, then
//! the same to the left. `--fire` holds that weapon (0 hammer, 1 gun, ...) and shoots it
//! up-right five times a second. `--aim` aims at X,Y relative to the tee.
//! The camera messages report a C++ client with default settings in a 16:9 window.
//!
//! Only loopback servers are accepted for now: testing against public servers has
//! not been approved yet.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use ddnet::map::Map;
use ddnet::net::client::{Client, PlayerInfo, State, VERSION_STRING};
use ddnet::net::{self, conn::FREQ};
use ddnet::physics::collision::Collision;
use ddnet::predict::Predictor;
use ddnet::protocol::generated::{self as g, GameMsg};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(server) = args.next().and_then(|a| a.parse::<SocketAddr>().ok()) else {
        eprintln!(
            "usage: ddnet-headless <server ip:port> [--name NAME] [--seconds N] [--walk SECONDS] [--script] [--fire WEAPON] [--aim X,Y]"
        );
        std::process::exit(2);
    };
    if !server.ip().is_loopback() {
        eprintln!("refusing {server}: only local (loopback) servers are allowed for now");
        std::process::exit(2);
    }
    let mut player = PlayerInfo::default();
    let (mut seconds, mut walk, mut script, mut fire) = (10i64, 0i64, false, None::<i32>);
    let mut aim = None::<(i32, i32)>;
    while let Some(flag) = args.next() {
        let mut value = || args.next().unwrap_or_default();
        match flag.as_str() {
            "--name" => player.name = value(),
            "--seconds" => seconds = value().parse().expect("--seconds N"),
            "--walk" => walk = value().parse().expect("--walk SECONDS"),
            "--script" => script = true,
            "--fire" => fire = Some(value().parse().expect("--fire WEAPON")),
            "--aim" => {
                let v = value();
                let (x, y) = v.split_once(',').expect("--aim X,Y");
                aim = Some((x.parse().expect("--aim X,Y"), y.parse().expect("--aim X,Y")));
            }
            _ => panic!("unknown flag {flag}"),
        }
    }

    println!("{VERSION_STRING} connecting to {server}");
    let socket = net::connect_socket(server).expect("socket");
    let start = Instant::now();
    let mut client = Client::new(player, "", net::random_bytes);
    client.connect(net::now(start));

    // the map's collision, and prediction from snapshots once the game is entered
    let collision = std::cell::OnceCell::new();
    let mut prediction: Option<Predictor> = None;
    let mut last_state = client.state();
    let mut entered_at = None;
    let mut next_report = 0;
    loop {
        let now = net::now(start);
        if let Err(e) = net::pump(&socket, &mut client, now) {
            eprintln!("network error: {e}");
            break;
        }
        if client.state() != last_state {
            println!("state: {:?}", client.state());
            if client.state() == State::Online {
                entered_at = Some(now);
                let col = client
                    .map()
                    .and_then(|m| Map::load(m.data.clone()).ok())
                    .map(|m| collision.get_or_init(|| Collision::new(&m)));
                prediction = col.map(Predictor::new);
                if let Some(map) = client.map() {
                    println!(
                        "map: {} crc={:08x} size={}",
                        map.name,
                        map.crc,
                        map.data.len()
                    );
                }
            }
            if client.state() == State::Offline {
                println!("disconnected: '{}'", client.conn.error());
                return;
            }
            last_state = client.state();
        }
        for msg in client.messages.drain(..) {
            match msg {
                GameMsg::SvChat(m) => println!("chat: {}", m.message),
                GameMsg::SvBroadcast(m) => println!("broadcast: {}", m.message),
                GameMsg::SvMotd(m) if !m.message.is_empty() => println!("motd: {}", m.message),
                _ => {}
            }
        }
        if let Some(p) = &mut prediction {
            p.update(&mut client);
        }
        if let Some(entered) = entered_at {
            let elapsed = now - entered;
            if script {
                net::scripted_input(&mut client.input, elapsed);
            } else {
                client.input.direction = (elapsed < walk * FREQ) as i32;
            }
            if let Some(weapon) = fire {
                // C++ CControls: the fire counter counts presses and releases
                client.input.wanted_weapon = weapon + 1;
                client.input.fire = (elapsed / (FREQ / 10)) as i32;
                (client.input.target_x, client.input.target_y) = (100, -60);
            }
            if let Some(target) = aim {
                (client.input.target_x, client.input.target_y) = target;
            }
            if elapsed >= next_report * FREQ {
                if let Some(c) = local_character(&client) {
                    print!(
                        "tick {} pos ({}, {})",
                        client.game_tick().unwrap_or(0),
                        c.x,
                        c.y
                    );
                    if let Some(s) = prediction.as_ref().map(|p| &p.stats) {
                        print!(
                            " | prediction: {} snapshots checked, {} mispredicted, {} prediction errors",
                            s.compared, s.mismatched, s.prediction_errors
                        );
                    }
                    println!();
                }
                next_report += 1;
            }
            if elapsed >= seconds * FREQ {
                client.disconnect(None, now);
                let _ = net::pump(&socket, &mut client, now); // send the close packet
                println!("disconnected cleanly");
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn local_character(client: &Client) -> Option<g::ObjCharacter> {
    let id = client.local_id()?;
    client
        .snapshot()
        .iter()
        .find(|it| it.type_id == g::NETOBJTYPE_CHARACTER && it.id == id)
        .map(|it| g::ObjCharacter::from_ints(&it.data))
}
