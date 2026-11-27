//! The current server's info (C++ `CClient::m_CurrentServerInfo`): the connless
//! request `CServerBrowser::RequestImpl` sends while it is missing, and the parts of
//! `CClient::ProcessConnlessPacket`/`ProcessServerInfo` that fill it (client.cpp:1367).

use crate::console::strtol;
use crate::protocol::packer::{SANITIZE_CC, SKIP_START_WHITESPACES, Unpacker};
use crate::protocol::packet::{self, CONNLESS_EXTRA_SIZE};
use crate::text;

/// C++ `SERVERBROWSE_GETINFO`, `SERVERBROWSE_INFO`, `SERVERBROWSE_INFO_EXTENDED`,
/// `SERVERBROWSE_INFO_EXTENDED_MORE` (masterserver.cpp).
const GETINFO: [u8; 8] = *b"\xff\xff\xff\xffgie3";
const INFO: [u8; 8] = *b"\xff\xff\xff\xffinf3";
const INFO_EXTENDED: [u8; 8] = *b"\xff\xff\xff\xffiext";
const INFO_EXTENDED_MORE: [u8; 8] = *b"\xff\xff\xff\xffiex+";

/// C++ `SERVERINFO_VANILLA`, `SERVERINFO_EXTENDED`, `SERVERINFO_EXTENDED_MORE`.
pub const VANILLA: i32 = 0;
pub const EXTENDED: i32 = 2;
const EXTENDED_MORE: i32 = 3;
/// C++ `SERVER_FLAG_PASSWORD`.
pub const FLAG_PASSWORD: i32 = 1;
/// C++ `VANILLA_MAX_CLIENTS`.
const VANILLA_MAX_CLIENTS: i32 = 16;

/// C++ `CServerInfo`, the fields the client shows (no player list: "player info is
/// irrelevant for the client (while connected)").
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerInfo {
    /// C++ `m_Type` (`VANILLA` or `EXTENDED`).
    pub type_: i32,
    pub version: String,
    pub name: String,
    pub map: String,
    pub game_type: String,
    pub flags: i32,
    pub num_players: i32,
    pub max_players: i32,
    pub num_clients: i32,
    pub max_clients: i32,
}

/// C++: `RequestImpl` (serverbrowser.cpp:1102) for a 0.6 address: `gie3` plus the
/// token's low byte, the next two bytes as the extended header's extra data.
pub fn request(token: i32) -> Vec<u8> {
    let mut data = GETINFO.to_vec();
    data.push((token & 0xff) as u8);
    let extra = token >> 8;
    let mut extra_data = [0u8; CONNLESS_EXTRA_SIZE];
    extra_data[0] = (extra >> 8) as u8;
    extra_data[1] = (extra & 0xff) as u8;
    packet::encode_connless(&data, Some(extra_data))
}

/// C++: `ProcessConnlessPacket` + `ProcessServerInfo` up to the point where it would
/// set the current server info: the info and its saved type, or `None` where C++
/// returns early, rejects it, or it carries no server information (`iex+`).
pub fn parse(data: &[u8]) -> Option<ServerInfo> {
    let raw_type = match data.get(..8)? {
        h if h == INFO => VANILLA,
        h if h == INFO_EXTENDED => EXTENDED,
        h if h == INFO_EXTENDED_MORE => EXTENDED_MORE,
        _ => return None,
    };
    if raw_type == EXTENDED_MORE {
        return None;
    }
    let saved_type = raw_type;
    let mut up = Unpacker::new(&data[8..]);
    let string = |up: &mut Unpacker, size: usize| {
        let s = up.get_string(SANITIZE_CC | SKIP_START_WHITESPACES);
        String::from_utf8_lossy(&text::truncate(s.as_bytes(), size)).into_owned()
    };
    let int = |up: &mut Unpacker| strtol(&up.get_string(0)).0;
    let _token = int(&mut up);
    let mut info = ServerInfo {
        type_: saved_type,
        version: string(&mut up, 32),
        name: string(&mut up, 64),
        map: string(&mut up, 128),
        ..Default::default()
    };
    if saved_type == EXTENDED {
        let (_crc, _size) = (int(&mut up), int(&mut up));
    }
    info.game_type = string(&mut up, 16);
    info.flags = int(&mut up);
    info.num_players = int(&mut up);
    info.max_players = int(&mut up);
    info.num_clients = int(&mut up);
    info.max_clients = int(&mut up);
    let i = &info;
    if i.num_clients < 0
        || i.max_clients < 0
        || i.num_players < 0
        || i.max_players < 0
        || i.num_players > i.num_clients
        || i.max_players > i.max_clients
        || (saved_type == VANILLA
            && (i.max_players > VANILLA_MAX_CLIENTS || i.max_clients > VANILLA_MAX_CLIENTS))
    {
        return None;
    }
    if saved_type == EXTENDED {
        up.get_string(0); // extra info, reserved
    }
    // the player list only decides whether the packet is accepted
    let mut ignore_error = false;
    let mut clients = 0;
    while clients < crate::clients::MAX_CLIENTS && !up.error() {
        up.get_string(SANITIZE_CC | SKIP_START_WHITESPACES); // name
        if up.error() {
            // packet end, no problem unless it happens during one player info
            ignore_error = true;
            break;
        }
        up.get_string(SANITIZE_CC | SKIP_START_WHITESPACES); // clan
        for _ in 0..3 {
            up.get_string(0); // country, score, player
        }
        if saved_type == EXTENDED {
            up.get_string(0); // extra info, reserved
        }
        clients += !up.error() as usize;
    }
    info.name = clean_whitespaces(&info.name);
    (!up.error() || ignore_error).then_some(info)
}

/// C++: `str_clean_whitespaces` (str.cpp:294): no leading or trailing spaces, runs of
/// spaces become one.
fn clean_whitespaces(s: &str) -> String {
    s.split(' ')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::packer::Packer;

    fn info_packet(head: [u8; 8], fields: &[&str]) -> Vec<u8> {
        let mut p = Packer::default();
        p.add_raw(&head);
        for f in fields {
            p.add_string(f.as_bytes(), 0, true);
        }
        p.data().to_vec()
    }

    #[test]
    fn parses_like_process_server_info() {
        let base = [
            "7",
            "0.6.4, 19.1",
            "  My   server ",
            "dm1",
            "DM",
            "1",
            "2",
            "4",
            "3",
            "8",
        ];
        let mut fields = base.to_vec();
        fields.extend(["a", "", "-1", "5", "1"]);
        let i = parse(&info_packet(INFO, &fields)).unwrap();
        assert_eq!(
            (i.name.as_str(), i.flags, i.type_),
            ("My server", 1, VANILLA)
        );
        // ends in the middle of a player: rejected
        fields.extend(["b", "clan"]);
        assert_eq!(parse(&info_packet(INFO, &fields)), None);
        // more players than clients, or a vanilla server over 16 slots: rejected
        let mut bad = base.to_vec();
        bad[7] = "9";
        assert_eq!(parse(&info_packet(INFO, &bad)), None);
        let mut big = base.to_vec();
        big[9] = "64";
        assert_eq!(parse(&info_packet(INFO, &big)), None);
        // extended: map crc and size, a reserved string, 64 slots are fine
        let ext = [
            "7",
            "0.6.4",
            "n",
            "dm1",
            "123",
            "4567",
            "DDraceNetwork",
            "0",
            "1",
            "64",
            "1",
            "64",
            "",
        ];
        let i = parse(&info_packet(INFO_EXTENDED, &ext)).unwrap();
        assert_eq!((i.game_type.as_str(), i.max_clients), ("DDraceNetwork", 64));
        assert_eq!(parse(&info_packet(INFO_EXTENDED_MORE, &ext)), None);
        // garbage never panics
        for n in 0..64 {
            let _ = parse(&[0xff; 64][..n]);
        }
    }

    #[test]
    fn request_bytes() {
        let r = request(0x00ab_cdef);
        assert_eq!(&r[..2], b"xe");
        assert_eq!(&r[2..6], &[0xab, 0xcd, 0, 0]);
        assert_eq!(&r[6..], b"\xff\xff\xff\xffgie3\xef");
    }
}
