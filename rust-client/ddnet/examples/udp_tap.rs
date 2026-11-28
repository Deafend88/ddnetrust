//! A UDP tap between local clients and a local server, for capturing what a client
//! sends (e.g. the C++ client's dummy handshake, `scripts/dummy_capture.ps1`). It
//! forwards every datagram and logs one line per datagram:
//! `<ms> <client> <c2s|s2c> <hex>`, where client 0 is the first source address seen,
//! 1 the second, and so on. Loopback only.
//!
//! Usage: udp_tap <listen port> <server port> <out file> <seconds>

use std::fmt::Write as _;
use std::io::{ErrorKind, Write};
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let [_, listen, server, out, secs] = &args[..] else {
        eprintln!("usage: udp_tap <listen port> <server port> <out file> <seconds>");
        std::process::exit(2);
    };
    let port = |s: &str| s.parse::<u16>().expect("port");
    let listen = UdpSocket::bind(("127.0.0.1", port(listen)))?;
    listen.set_nonblocking(true)?;
    let server: SocketAddr = ([127, 0, 0, 1], port(server)).into();
    let end = Instant::now() + Duration::from_secs(secs.parse().expect("seconds"));
    let start = Instant::now();
    let mut log = std::fs::File::create(out)?;
    // one upstream socket per client address, so the server sees each client apart
    let mut clients: Vec<(SocketAddr, UdpSocket)> = Vec::new();
    let mut buf = [0u8; 2048];
    let mut line = |client: usize, dir: &str, data: &[u8]| -> std::io::Result<()> {
        let mut s = format!("{} {client} {dir} ", start.elapsed().as_millis());
        for b in data {
            write!(s, "{b:02x}").unwrap();
        }
        writeln!(log, "{s}")
    };
    while Instant::now() < end {
        let mut idle = true;
        match listen.recv_from(&mut buf) {
            Ok((n, from)) => {
                idle = false;
                let i = match clients.iter().position(|c| c.0 == from) {
                    Some(i) => i,
                    None => {
                        let up = UdpSocket::bind("127.0.0.1:0")?;
                        up.connect(server)?;
                        up.set_nonblocking(true)?;
                        clients.push((from, up));
                        clients.len() - 1
                    }
                };
                line(i, "c2s", &buf[..n])?;
                let _ = clients[i].1.send(&buf[..n]);
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::ConnectionReset) => {}
            Err(e) => return Err(e),
        }
        for (i, (addr, up)) in clients.iter().enumerate() {
            match up.recv(&mut buf) {
                Ok(n) => {
                    idle = false;
                    line(i, "s2c", &buf[..n])?;
                    let _ = listen.send_to(&buf[..n], addr);
                }
                Err(e)
                    if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::ConnectionReset) => {}
                Err(e) => return Err(e),
            }
        }
        if idle {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    Ok(())
}
