//! Client side of a connection, as a sans-IO state machine: it takes received
//! packets and the current time, and leaves datagrams to send in `outbox`.
//! C++: `CNetConnection` (`network_conn.cpp`), `CNetClient::Update` / `Send`
//! (`network_client.cpp`) and `CPacketChunkUnpacker` (`network.cpp:43`).

use std::collections::VecDeque;

use crate::protocol::packer::{sanitize_cc, utf8_check, utf8_fix_truncation};
use crate::protocol::packet::{
    self, CHUNKFLAG_VITAL, ChunkHeader, MAX_PACKET_CHUNKS, MAX_PAYLOAD, MAX_SEQUENCE,
    PACKETFLAG_CONTROL, PACKETFLAG_RESEND, Packet, is_seq_in_backroom,
};

/// Clock units per second; `now` values are in nanoseconds like C++ `time_get()`.
pub const FREQ: i64 = 1_000_000_000;
/// C++ config defaults (`config_variables.h`): `conn_timeout`, `conn_timeout_protection`,
/// `conn_resend_requests_per_second`.
const CONN_TIMEOUT: i64 = 100;
const CONN_TIMEOUT_PROTECTION: i64 = 1000;
const RESEND_REQUESTS_PER_SECOND: i64 = 10;

pub const CHUNKFLAG_RESEND: i32 = 2;
pub const CTRLMSG_KEEPALIVE: u8 = 0;
pub const CTRLMSG_CONNECT: u8 = 1;
pub const CTRLMSG_CONNECTACCEPT: u8 = 2;
pub const CTRLMSG_ACCEPT: u8 = 3;
pub const CTRLMSG_CLOSE: u8 = 4;
pub const SECURITY_TOKEN_UNKNOWN: i32 = -1;
pub const SECURITY_TOKEN_MAGIC: [u8; 4] = *b"TKEN";
const MAX_CHUNK_HEADER_SIZE: usize = 3;

/// C++: `CNetConnection::EState` (the 0.7-only `WANT_TOKEN` is never entered).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Offline = 0,
    Connect = 2,
    Online = 4,
    Error = 5,
}

/// A received chunk. C++: `CNetChunk`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub flags: i32,
    pub data: Vec<u8>,
}

/// C++: `CNetChunkResend`.
struct Resend {
    flags: i32,
    data: Vec<u8>,
    sequence: i32,
    first_send: i64,
    last_send: i64,
}

/// C++: `CNetConnection` + `CNetClient`.
pub struct Connection {
    state: State,
    sequence: i32,
    ack: i32,
    peer_ack: i32,
    remote_closed: bool,
    timeout_situation: bool,
    security_token: i32,
    resend: VecDeque<Resend>,
    resend_bytes: usize,
    last_send: i64,
    last_recv: i64,
    last_resend: i64,
    resend_requested: bool,
    // C++: `m_Construct`
    construct_flags: i32,
    construct_num_chunks: i32,
    construct_data: Vec<u8>,
    error: String,
    /// Datagrams to send to the server, oldest first.
    pub outbox: Vec<Vec<u8>>,
}

impl Default for Connection {
    fn default() -> Self {
        Self {
            state: State::Offline,
            sequence: 0,
            ack: 0,
            peer_ack: 0,
            remote_closed: false,
            timeout_situation: false,
            security_token: SECURITY_TOKEN_UNKNOWN,
            resend: VecDeque::new(),
            resend_bytes: 0,
            last_send: 0,
            last_recv: 0,
            last_resend: 0,
            resend_requested: false,
            construct_flags: 0,
            construct_num_chunks: 0,
            construct_data: Vec::new(),
            error: String::new(),
            outbox: Vec::new(),
        }
    }
}

impl Connection {
    pub fn state(&self) -> State {
        self.state
    }

    /// Sequence of the last in-order vital chunk received. C++: `AckSequence`.
    pub fn ack(&self) -> i32 {
        self.ack
    }

    pub fn error(&self) -> &str {
        &self.error
    }

    /// C++: `CNetConnection::Reset` (keeps the error string and the outbox).
    fn reset(&mut self) {
        let error = std::mem::take(&mut self.error);
        let outbox = std::mem::take(&mut self.outbox);
        *self = Self {
            error,
            outbox,
            ..Self::default()
        };
    }

    /// C++: `CNetConnection::Connect` (network_conn.cpp:247).
    pub fn connect(&mut self, now: i64) {
        if self.state != State::Offline {
            return;
        }
        self.reset();
        self.error.clear();
        self.state = State::Connect;
        self.send_connect(now);
    }

    fn send_connect(&mut self, now: i64) {
        self.send_control(CTRLMSG_CONNECT, &SECURITY_TOKEN_MAGIC, now);
    }

    /// C++: `CNetConnection::SendControl` -> `CNetBase::SendControlMsg`.
    fn send_control(&mut self, msg: u8, extra: &[u8], now: i64) {
        self.last_send = now;
        let mut data = vec![msg];
        data.extend_from_slice(extra);
        self.outbox.push(packet::encode(
            PACKETFLAG_CONTROL,
            self.ack,
            0,
            &data,
            self.security_token,
        ));
    }

    /// C++: `CNetConnection::Disconnect` (network_conn.cpp:311) called with a reason
    /// other than its own error string.
    pub fn disconnect(&mut self, reason: Option<&str>, now: i64) {
        if self.state == State::Offline {
            return;
        }
        if !self.remote_closed {
            self.send_close(reason, now);
            self.error = reason.unwrap_or_default().to_owned();
        }
        self.reset();
    }

    fn send_close(&mut self, reason: Option<&str>, now: i64) {
        if self.timeout_situation {
            return;
        }
        match reason {
            Some(reason) => {
                let mut extra = reason.as_bytes().to_vec();
                extra.push(0);
                self.send_control(CTRLMSG_CLOSE, &extra, now);
            }
            None => self.send_control(CTRLMSG_CLOSE, &[], now),
        }
    }

    /// C++: `CNetClient::Send` for a connection-oriented chunk.
    pub fn send(&mut self, data: &[u8], vital: bool, flush: bool, now: i64) {
        self.queue_chunk(if vital { CHUNKFLAG_VITAL } else { 0 }, data, now);
        if flush {
            self.flush(now);
        }
    }

    /// C++: `CNetConnection::QueueChunk`. Returns 0, or -1 if offline or the resend
    /// buffer is full (the chunk is still sent once in that case).
    pub fn queue_chunk(&mut self, flags: i32, data: &[u8], now: i64) -> i32 {
        if flags & CHUNKFLAG_VITAL != 0 {
            self.sequence = (self.sequence + 1) % MAX_SEQUENCE;
        }
        self.queue_chunk_ex(flags, data, self.sequence, now)
    }

    /// C++: `CNetConnection::QueueChunkEx` (network_conn.cpp:143).
    fn queue_chunk_ex(&mut self, flags: i32, data: &[u8], sequence: i32, now: i64) -> i32 {
        if matches!(self.state, State::Offline | State::Error) {
            return -1;
        }
        // flush if the chunk would not fit next to the security token
        if self.construct_data.len() + data.len() + MAX_CHUNK_HEADER_SIZE > MAX_PAYLOAD - 4
            || self.construct_num_chunks == MAX_PACKET_CHUNKS
        {
            self.flush(now);
        }
        let header = ChunkHeader {
            flags,
            size: data.len(),
            sequence,
        };
        header.pack(&mut self.construct_data);
        self.construct_data.extend_from_slice(data);
        self.construct_num_chunks += 1;

        if flags & CHUNKFLAG_VITAL != 0 && flags & CHUNKFLAG_RESEND == 0 {
            // ponytail: C++ keeps resends in a 32 KiB CStaticRingBuffer; this is a byte
            // budget with the same per-entry rounding but no fragmentation. Only differs
            // with ~32 KiB unacked, which times out first in practice.
            let wanted = resend_cost(data.len());
            if self.resend_bytes + wanted > 32 * 1024 {
                return -1; // out of buffer, hope nobody asks for a resend
            }
            self.resend_bytes += wanted;
            self.resend.push_back(Resend {
                flags,
                data: data.to_vec(),
                sequence,
                first_send: now,
                last_send: now,
            });
        }
        0
    }

    /// C++: `CNetConnection::Flush` (network_conn.cpp:121). Returns the number of
    /// chunks sent.
    pub fn flush(&mut self, now: i64) -> i32 {
        let num_chunks = self.construct_num_chunks;
        if num_chunks == 0 && self.construct_flags & PACKETFLAG_RESEND == 0 {
            return 0;
        }
        self.outbox.push(packet::encode(
            self.construct_flags,
            self.ack,
            num_chunks,
            &self.construct_data,
            self.security_token,
        ));
        self.last_send = now;
        self.construct_flags = 0;
        self.construct_num_chunks = 0;
        self.construct_data.clear();
        num_chunks
    }

    /// C++: `CNetConnection::AckChunks`.
    fn ack_chunks(&mut self, ack: i32) {
        while let Some(front) = self.resend.front() {
            if !is_seq_in_backroom(front.sequence, ack) {
                break;
            }
            let wanted = resend_cost(front.data.len());
            self.resend_bytes -= wanted;
            self.resend.pop_front();
        }
    }

    fn resend_chunk(&mut self, index: usize, now: i64) {
        let (flags, data, sequence) = {
            let r = &self.resend[index];
            (r.flags | CHUNKFLAG_RESEND, r.data.clone(), r.sequence)
        };
        self.queue_chunk_ex(flags, &data, sequence, now);
        self.resend[index].last_send = now;
    }

    /// C++: `CNetConnection::AnswerResendRequest`: at most one full resend per
    /// `1 / RESEND_REQUESTS_PER_SECOND`.
    fn answer_resend_request(&mut self, now: i64) {
        if !self.resend_requested || now - self.last_resend < FREQ / RESEND_REQUESTS_PER_SECOND {
            return;
        }
        self.resend_requested = false;
        self.last_resend = now;
        for i in 0..self.resend.len() {
            self.resend_chunk(i, now);
        }
    }

    /// Processes a decoded packet from the server; `true` means its chunks should be
    /// read with `next_chunk`. Strips the security token from `packet.data`.
    /// C++: `CNetConnection::Feed` (network_conn.cpp:356), 0.6 path.
    pub fn feed(&mut self, packet: &mut Packet, now: i64) -> bool {
        if !matches!(self.state, State::Connect | State::Online) {
            return false;
        }
        // C++ reads the control byte from the buffer even after stripping the token
        // left the data empty (it is then the token's first byte), so take it now.
        let first_byte = packet.data.first().copied().unwrap_or(0);
        if self.security_token != SECURITY_TOKEN_UNKNOWN
            && self.security_token != packet::SECURITY_TOKEN_UNSUPPORTED
        {
            // supposed to have a valid token in this packet, check it
            let Some(split) = packet.data.len().checked_sub(4) else {
                return false;
            };
            let token = i32::from_be_bytes(packet.data[split..].try_into().unwrap());
            packet.data.truncate(split);
            if token != self.security_token {
                return false;
            }
        }

        // the ack must lie between our latest acked and our latest sent sequence
        if self.sequence >= self.peer_ack {
            if packet.ack < self.peer_ack || packet.ack > self.sequence {
                return false;
            }
        } else if packet.ack < self.peer_ack && packet.ack > self.sequence {
            return false;
        }
        self.peer_ack = packet.ack;

        if packet.flags & PACKETFLAG_RESEND != 0 {
            self.resend_requested = true;
        }
        self.answer_resend_request(now);

        if packet.flags & PACKETFLAG_CONTROL != 0 {
            let msg = first_byte;
            if msg == CTRLMSG_CLOSE {
                self.state = State::Error;
                self.remote_closed = true;
                self.error = close_reason(&packet.data);
                return false;
            }
            if self.state == State::Connect && msg == CTRLMSG_CONNECTACCEPT {
                let d = &packet.data;
                self.security_token = if self.security_token == SECURITY_TOKEN_UNKNOWN
                    && d.len() >= 1 + 4 + 4
                    && d[1..5] == SECURITY_TOKEN_MAGIC
                {
                    i32::from_be_bytes(d[5..9].try_into().unwrap())
                } else {
                    packet::SECURITY_TOKEN_UNSUPPORTED
                };
                self.send_control(CTRLMSG_ACCEPT, &[], now);
                self.last_recv = now;
                self.state = State::Online;
            }
        }

        if self.state == State::Online {
            self.last_recv = now;
            self.ack_chunks(packet.ack);
        }
        true
    }

    /// Next chunk of a packet that `feed` accepted, handling vital sequence numbers:
    /// duplicates are skipped, gaps request a resend. Call until `None`.
    /// C++: `CPacketChunkUnpacker::UnpackNextChunk` (network.cpp:57).
    pub fn next_chunk(&mut self, unpacker: &mut ChunkUnpacker) -> Option<Chunk> {
        loop {
            if unpacker.index >= unpacker.packet.num_chunks {
                return None;
            }
            let data = &unpacker.packet.data;
            let (header, header_len) = ChunkHeader::unpack(data.get(unpacker.offset..)?)?;
            unpacker.index += 1;
            let start = unpacker.offset + header_len;
            let chunk = data.get(start..start + header.size)?;
            unpacker.offset = start + header.size;

            if header.flags & CHUNKFLAG_VITAL != 0 {
                if header.sequence == (self.ack + 1) % MAX_SEQUENCE {
                    self.ack = header.sequence; // in sequence
                } else {
                    if !is_seq_in_backroom(header.sequence, self.ack) {
                        // out of sequence, request resend
                        self.construct_flags |= PACKETFLAG_RESEND;
                    }
                    continue;
                }
            }
            return Some(Chunk {
                flags: header.flags,
                data: chunk.to_vec(),
            });
        }
    }

    /// Timers: timeouts, resends, flush after 500 ms, keepalive after 1 s, connect
    /// retries every 500 ms; drops the connection on error.
    /// C++: `CNetClient::Update` -> `CNetConnection::Update` (network_conn.cpp:504).
    pub fn update(&mut self, now: i64) {
        if self.state == State::Error
            && self.timeout_situation
            && now - self.last_recv > FREQ * CONN_TIMEOUT_PROTECTION
        {
            self.timeout_situation = false;
            self.error = "Timeout Protection over".into();
        }
        if matches!(self.state, State::Offline | State::Error) {
            return self.drop_on_error(now);
        }
        self.timeout_situation = false;
        self.answer_resend_request(now);

        if self.state != State::Connect && now - self.last_recv > FREQ * CONN_TIMEOUT {
            self.state = State::Error;
            self.error = "Timeout".into();
            self.timeout_situation = true;
        }

        if let Some(front) = self.resend.front() {
            if now - front.first_send > FREQ * CONN_TIMEOUT {
                self.state = State::Error;
                self.error = format!("Too weak connection (not acked for {CONN_TIMEOUT} seconds)");
                self.timeout_situation = true;
            } else if now - front.last_send > FREQ {
                self.resend_chunk(0, now);
            }
        }

        match self.state {
            State::Online => {
                if now - self.last_send > FREQ / 2 {
                    self.flush(now);
                }
                if now - self.last_send > FREQ {
                    self.send_control(CTRLMSG_KEEPALIVE, &[], now);
                }
            }
            State::Connect if now - self.last_send > FREQ / 2 => self.send_connect(now),
            _ => {}
        }
        self.drop_on_error(now);
    }

    /// C++: `if(State() == ERROR) Disconnect(ErrorString())` in `CNetClient::Update`;
    /// passing its own error string keeps it.
    fn drop_on_error(&mut self, now: i64) {
        if self.state != State::Error {
            return;
        }
        if !self.remote_closed {
            let error = self.error.clone();
            self.send_close(Some(&error), now);
        }
        self.reset();
    }
}

/// C++: the CLOSE branch of `CNetConnection::Feed`: `str_copy` at most `min(size, 256) - 1`
/// bytes up to a NUL (which also drops a trailing broken code point), replace control
/// chars, reject invalid UTF-8.
fn close_reason(data: &[u8]) -> String {
    let max = data.len().min(256).saturating_sub(1);
    let rest = data.get(1..).unwrap_or_default();
    let mut reason: Vec<u8> = rest
        .iter()
        .take(max)
        .take_while(|&&b| b != 0)
        .copied()
        .collect();
    utf8_fix_truncation(&mut reason);
    sanitize_cc(&mut reason);
    if !utf8_check(&reason) {
        return "(Invalid error message)".into();
    }
    String::from_utf8(reason).expect("checked UTF-8")
}

/// Read position in a packet accepted by `Connection::feed`.
/// C++: `CPacketChunkUnpacker`'s state.
pub struct ChunkUnpacker {
    packet: Packet,
    index: i32,
    offset: usize,
}

impl ChunkUnpacker {
    pub fn new(packet: Packet) -> Self {
        Self {
            packet,
            index: 0,
            offset: 0,
        }
    }
}

/// Bytes one resend entry takes in C++'s ring buffer: `sizeof(CNetChunkResend)` (40) plus
/// data, rounded up to its 24-byte item header (`CRingBufferBase::Allocate`).
fn resend_cost(len: usize) -> usize {
    (40 + len + 24).div_ceil(24) * 24
}
