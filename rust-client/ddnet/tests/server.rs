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
use std::time::{Duration, Instant};

use ddnet::net::client::{Client, PlayerInfo, State, VERSION_STRING};
use ddnet::net::{self, conn::FREQ};
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

fn start_server(exe: &PathBuf) -> (Server, u16) {
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
    let log = fs::File::create(dir.join("server.log")).unwrap();
    let child = Command::new(exe)
        .current_dir(&dir)
        .args([
            "sv_register 0",
            "bindaddr 127.0.0.1",
            &format!("sv_port {port}"),
            "sv_map coverage",
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
    let (server, port) = start_server(&exe);
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
