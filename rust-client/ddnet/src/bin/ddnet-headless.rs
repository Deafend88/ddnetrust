//! Headless DDNet client: connects, downloads the map, enters the game, optionally
//! walks right or runs a movement script, then disconnects cleanly.
//!
//! Usage: ddnet-headless <server ip:port> [--name NAME] [--seconds N] [--walk SECONDS] [--script]
//!
//! `--script` repeats a 4 second loop: walk right with a jump and a hook up-right, then
//! the same to the left.
//!
//! Only loopback servers are accepted for now: testing against public servers has
//! not been approved yet.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use ddnet::net::client::{Client, PlayerInfo, State, VERSION_STRING};
use ddnet::net::{self, conn::FREQ};
use ddnet::protocol::generated::{self as g, GameMsg};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(server) = args.next().and_then(|a| a.parse::<SocketAddr>().ok()) else {
        eprintln!(
            "usage: ddnet-headless <server ip:port> [--name NAME] [--seconds N] [--walk SECONDS] [--script]"
        );
        std::process::exit(2);
    };
    if !server.ip().is_loopback() {
        eprintln!("refusing {server}: only local (loopback) servers are allowed for now");
        std::process::exit(2);
    }
    let mut player = PlayerInfo::default();
    let (mut seconds, mut walk, mut script) = (10i64, 0i64, false);
    while let Some(flag) = args.next() {
        let mut value = || args.next().unwrap_or_default();
        match flag.as_str() {
            "--name" => player.name = value(),
            "--seconds" => seconds = value().parse().expect("--seconds N"),
            "--walk" => walk = value().parse().expect("--walk SECONDS"),
            "--script" => script = true,
            _ => panic!("unknown flag {flag}"),
        }
    }

    println!("{VERSION_STRING} connecting to {server}");
    let socket = net::connect_socket(server).expect("socket");
    let start = Instant::now();
    let mut client = Client::new(player, "", net::random_bytes);
    client.connect(net::now(start));

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
        if let Some(entered) = entered_at {
            let elapsed = now - entered;
            if script {
                scripted_input(&mut client.input, elapsed);
            } else {
                client.input.direction = (elapsed < walk * FREQ) as i32;
            }
            if elapsed >= next_report * FREQ {
                if let Some(c) = local_character(&client) {
                    println!(
                        "tick {} pos ({}, {})",
                        client.game_tick().unwrap_or(0),
                        c.x,
                        c.y
                    );
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

/// The `--script` loop, in tenths of a second: each 2 s half walks one way, jumps at
/// 0.5 s and hooks up ahead from 1.0 to 1.6 s.
fn scripted_input(input: &mut g::ObjPlayerInput, elapsed: i64) {
    let phase = elapsed * 10 / FREQ % 40;
    let dir = if phase < 20 { 1 } else { -1 };
    let t = phase % 20;
    input.direction = dir;
    input.jump = (5..7).contains(&t) as i32;
    input.hook = (10..16).contains(&t) as i32;
    input.target_x = dir * 100;
    input.target_y = -150;
}

fn local_character(client: &Client) -> Option<g::ObjCharacter> {
    let id = client.local_id()?;
    client
        .snapshot()
        .iter()
        .find(|it| it.type_id == g::NETOBJTYPE_CHARACTER && it.id == id)
        .map(|it| g::ObjCharacter::from_ints(&it.data))
}
