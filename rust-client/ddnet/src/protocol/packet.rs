//! Packet and chunk headers. C++: `src/engine/shared/network.{h,cpp}`.
//!
//! 0.6 layout only (chunk header split 4). 0.7 ("sixup") packets are rejected,
//! see `docs/DECISIONS.md`.

use super::huffman;

pub const MAX_PACKET_SIZE: usize = 1400;
pub const PACKET_HEADER_SIZE: usize = 3;
pub const CONNLESS_EXTRA_SIZE: usize = 4;
pub const MAX_SEQUENCE: i32 = 1 << 10;
pub const MAX_PACKET_CHUNKS: i32 = 0xFF;
/// Room for chunk data in a packet. C++: `CNetPacketConstruct::m_aChunkData`.
pub const MAX_PAYLOAD: usize = MAX_PACKET_SIZE - PACKET_HEADER_SIZE;

pub const PACKETFLAG_UNUSED: i32 = 1 << 0;
pub const PACKETFLAG_CONTROL: i32 = 1 << 2;
pub const PACKETFLAG_CONNLESS: i32 = 1 << 3;
pub const PACKETFLAG_RESEND: i32 = 1 << 4;
pub const PACKETFLAG_COMPRESSION: i32 = 1 << 5;
/// Not sent on the wire; marks a connless packet with the `xe` header.
pub const PACKETFLAG_EXTENDED: i32 = 1 << 6;

pub const CHUNKFLAG_VITAL: i32 = 1;

/// C++: `NET_SECURITY_TOKEN_UNSUPPORTED` (network.h:132).
pub const SECURITY_TOKEN_UNSUPPORTED: i32 = 0;

const HEADER_EXTENDED: [u8; 2] = *b"xe";

/// C++: `CNetChunkHeader` with `Split = 4`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkHeader {
    pub flags: i32,
    pub size: usize,
    /// -1 for non-vital chunks after `unpack`.
    pub sequence: i32,
}

impl ChunkHeader {
    /// C++: `CNetChunkHeader::Pack` (network.cpp:443). Appends 2 bytes, 3 if vital.
    pub fn pack(&self, out: &mut Vec<u8>) {
        assert!(
            self.size < 1 << 10,
            "invalid network chunk size: {}",
            self.size
        );
        let size = self.size as i32;
        out.push((((self.flags & 3) << 6) | ((size >> 4) & 0x3f)) as u8);
        let mut b1 = (size & 0xf) as u8;
        if self.flags & CHUNKFLAG_VITAL != 0 {
            b1 |= ((self.sequence >> 2) & !0xf) as u8;
            out.push(b1);
            out.push((self.sequence & 0xff) as u8);
        } else {
            out.push(b1);
        }
    }

    /// C++: `CNetChunkHeader::Unpack` (network.cpp:458). Returns the header and its
    /// length, or `None` if `data` is too short (C++ relies on its callers for that).
    pub fn unpack(data: &[u8]) -> Option<(Self, usize)> {
        let (b0, b1) = (*data.first()? as i32, *data.get(1)? as i32);
        let flags = (b0 >> 6) & 3;
        let size = (((b0 & 0x3f) << 4) | (b1 & 0xf)) as usize;
        let (sequence, len) = if flags & CHUNKFLAG_VITAL != 0 {
            (((b1 & !0xf) << 2) | *data.get(2)? as i32, 3)
        } else {
            (-1, 2)
        };
        let header = Self {
            flags,
            size,
            sequence,
        };
        Some((header, len))
    }
}

/// A decoded packet. C++: `CNetPacketConstruct`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Packet {
    pub flags: i32,
    pub ack: i32,
    pub num_chunks: i32,
    pub data: Vec<u8>,
    /// Only meaningful with `PACKETFLAG_EXTENDED`.
    pub extra: [u8; CONNLESS_EXTRA_SIZE],
}

/// C++: `CNetBase::IsValidConnectionOrientedPacket` (network.cpp:134).
pub fn is_valid_connection_oriented(flags: i32, num_chunks: i32, data_size: usize) -> bool {
    if flags & !(PACKETFLAG_CONTROL | PACKETFLAG_RESEND | PACKETFLAG_COMPRESSION) != 0 {
        return false;
    }
    if flags & PACKETFLAG_CONTROL != 0 {
        // control packets carry at least the message id, no chunks, no compression
        return num_chunks == 0 && data_size > 0 && flags & PACKETFLAG_COMPRESSION == 0;
    }
    // only resend requests may have zero chunks
    let min_chunks = if flags & PACKETFLAG_RESEND != 0 { 0 } else { 1 };
    num_chunks >= min_chunks && num_chunks <= MAX_PACKET_CHUNKS
}

/// Encodes a connection-oriented packet. C++: `CNetBase::SendPacket` (network.cpp:193),
/// non-sixup path. The security token is appended unless it is
/// `SECURITY_TOKEN_UNSUPPORTED` (-1, "unknown", is appended too); the
/// payload is Huffman-compressed when that makes it strictly smaller.
pub fn encode(flags: i32, ack: i32, num_chunks: i32, data: &[u8], security_token: i32) -> Vec<u8> {
    assert!(
        is_valid_connection_oriented(flags, num_chunks, data.len()),
        "invalid packet to send: flags={flags} ack={ack} chunks={num_chunks} size={}",
        data.len()
    );
    assert!(
        flags & PACKETFLAG_COMPRESSION == 0,
        "compression is set automatically"
    );

    let mut payload = data.to_vec();
    if security_token != SECURITY_TOKEN_UNSUPPORTED {
        payload.extend_from_slice(&security_token.to_be_bytes());
    }
    assert!(
        payload.len() <= MAX_PAYLOAD,
        "packet too large: {}",
        payload.len()
    );

    let mut flags = flags;
    let mut out = vec![0u8; MAX_PACKET_SIZE];
    let mut size = payload.len();
    let compressed = if flags & PACKETFLAG_CONTROL == 0 {
        huffman::compress(&payload, &mut out[PACKET_HEADER_SIZE..])
    } else {
        None
    };
    match compressed {
        Some(n) if n > 0 && n < payload.len() => {
            size = n;
            flags |= PACKETFLAG_COMPRESSION;
        }
        _ => out[PACKET_HEADER_SIZE..][..size].copy_from_slice(&payload),
    }
    out[0] = (((flags << 2) & 0xfc) | ((ack >> 8) & 0x3)) as u8;
    out[1] = (ack & 0xff) as u8;
    out[2] = num_chunks as u8;
    out.truncate(PACKET_HEADER_SIZE + size);
    out
}

/// Encodes a connectionless packet. C++: `CNetBase::SendPacketConnless` (network.cpp:159).
/// `extended` selects the `xe` header with 4 extra bytes; otherwise 6 bytes of 0xFF.
pub fn encode_connless(data: &[u8], extended: Option<[u8; CONNLESS_EXTRA_SIZE]>) -> Vec<u8> {
    const DATA_OFFSET: usize = HEADER_EXTENDED.len() + CONNLESS_EXTRA_SIZE;
    assert!(
        data.len() <= MAX_PACKET_SIZE - DATA_OFFSET,
        "connless packet too large"
    );
    let mut out = Vec::with_capacity(DATA_OFFSET + data.len());
    match extended {
        Some(extra) => {
            out.extend_from_slice(&HEADER_EXTENDED);
            out.extend_from_slice(&extra);
        }
        None => out.extend_from_slice(&[0xFF; DATA_OFFSET]),
    }
    out.extend_from_slice(data);
    out
}

/// Decodes a received datagram. C++: `CNetBase::UnpackPacket` (network.cpp:282) as
/// called by `CNetClient::Recv`. Returns `None` for anything C++ rejects, and also for
/// 0.7 packets, which C++ would parse.
pub fn decode(buf: &[u8]) -> Option<Packet> {
    if buf.len() < PACKET_HEADER_SIZE || buf.len() > MAX_PACKET_SIZE {
        return None;
    }
    let flags = (buf[0] >> 2) as i32;

    if flags & PACKETFLAG_CONNLESS != 0 {
        if buf[0] & 0x3 == 1 {
            return None; // 0.7 connless
        }
        const OFFSET: usize = 6;
        let data = buf.get(OFFSET..)?.to_vec();
        let mut packet = Packet {
            flags: PACKETFLAG_CONNLESS,
            data,
            ..Packet::default()
        };
        if buf[..2] == HEADER_EXTENDED {
            packet.flags |= PACKETFLAG_EXTENDED;
            packet.extra.copy_from_slice(&buf[2..OFFSET]);
        }
        return Some(packet);
    }

    if flags & PACKETFLAG_UNUSED != 0 {
        return None; // 0.7
    }
    let ack = (((buf[0] & 0x3) as i32) << 8) | buf[1] as i32;
    let num_chunks = buf[2] as i32;
    let raw = &buf[PACKET_HEADER_SIZE..];
    if !is_valid_connection_oriented(flags, num_chunks, raw.len()) {
        return None;
    }
    let data = if flags & PACKETFLAG_COMPRESSION != 0 {
        let mut out = vec![0u8; MAX_PAYLOAD];
        let n = huffman::decompress(raw, &mut out)?;
        out.truncate(n);
        out
    } else {
        raw.to_vec()
    };
    Some(Packet {
        flags,
        ack,
        num_chunks,
        data,
        ..Packet::default()
    })
}

/// True if `seq` is within the half of the sequence space behind `ack`, i.e. already
/// received. C++: `CNetBase::IsSeqInBackroom` (network.cpp:471).
pub fn is_seq_in_backroom(seq: i32, ack: i32) -> bool {
    let bottom = ack - MAX_SEQUENCE / 2;
    if bottom < 0 {
        seq <= ack || seq >= bottom + MAX_SEQUENCE
    } else {
        seq <= ack && seq >= bottom
    }
}
