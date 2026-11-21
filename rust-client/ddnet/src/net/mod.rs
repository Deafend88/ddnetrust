//! Connection and client protocol flow, plus the thin UDP glue around them.

pub mod client;
pub mod conn;

use std::io::{self, ErrorKind};
use std::net::{SocketAddr, UdpSocket};
use std::time::Instant;

use client::Client;

/// Monotonic clock in `conn::FREQ` units. Starts at 1000 s instead of 0 so that
/// "time since 0" checks behave like C++, whose `time_get()` counts from boot.
pub fn now(start: Instant) -> i64 {
    1000 * conn::FREQ + start.elapsed().as_nanos() as i64
}

/// Non-blocking UDP socket connected to `server` (the OS then drops datagrams from
/// other addresses, like C++ `IsPeerAddress`).
pub fn connect_socket(server: SocketAddr) -> io::Result<UdpSocket> {
    let bind: SocketAddr = if server.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    }
    .parse()
    .unwrap();
    let socket = UdpSocket::bind(bind)?;
    socket.connect(server)?;
    socket.set_nonblocking(true)?;
    Ok(socket)
}

/// Receives everything pending, runs one client frame, sends what it produced.
pub fn pump(socket: &UdpSocket, client: &mut Client, now: i64) -> io::Result<()> {
    let mut incoming = Vec::new();
    let mut buf = [0u8; 2048];
    loop {
        match socket.recv(&mut buf) {
            Ok(n) => incoming.push(buf[..n].to_vec()),
            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
            // Windows reports ICMP port unreachable as a reset on the next recv; C++
            // disables that (SIO_UDP_CONNRESET), so it is not an error here either.
            Err(e) if e.kind() == ErrorKind::ConnectionReset => continue,
            Err(e) => return Err(e),
        }
    }
    client.frame(incoming, now);
    for datagram in client.conn.outbox.drain(..) {
        if let Err(e) = socket.send(&datagram)
            && e.kind() != ErrorKind::ConnectionReset
        {
            return Err(e);
        }
    }
    Ok(())
}

/// 16 unpredictable bytes from std's OS-seeded SipHash keys (`RandomState`).
/// ponytail: no extra dependency; switch to `getrandom` if a CSPRNG is ever required.
pub fn random_bytes() -> [u8; 16] {
    use std::hash::{BuildHasher, RandomState};
    let a = RandomState::new().hash_one(0u8);
    let b = RandomState::new().hash_one(1u8);
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&a.to_le_bytes());
    out[8..].copy_from_slice(&b.to_le_bytes());
    out
}
