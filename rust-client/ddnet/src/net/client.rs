//! Client protocol flow for one connection, sans-IO. C++: the network parts of
//! `CClient` (`src/engine/client/client.cpp`), `CSmoothTime` (`smooth_time.cpp`) and the
//! messages `CGameClient` sends (`gameclient.cpp`). No dummy, demos, rcon or 0.7.

use std::collections::VecDeque;

use super::conn::{ChunkUnpacker, Connection, FREQ, State as ConnState};
use crate::protocol::generated::{self as g, GameMsg, ObjPlayerInput};
use crate::protocol::packer::{
    Packer, SANITIZE_CC, SKIP_START_WHITESPACES, Unpacker, utf8_isspace,
};
use crate::protocol::packet::{self, CHUNKFLAG_VITAL, PACKETFLAG_CONNLESS, PACKETFLAG_CONTROL};
use crate::protocol::snapshot::{self, Item, MAX_NETOBJSIZES, Snapshot};
use crate::protocol::varint;

/// C++: `GAME_NETVERSION` (game/version.h).
const GAME_NETVERSION: &str = "0.6 626fce9a778df4d4";
/// Feature level this client implements. C++: `DDNET_VERSION_NUMBER` (PLAN.md §8).
pub const DDNET_VERSION: i32 = 20030;
/// Our honest version string, sent in `clientver`.
pub const VERSION_STRING: &str = concat!("DDNet-rs ", env!("CARGO_PKG_VERSION"));

const TICK_SPEED: i64 = 50; // C++: SERVER_TICK_SPEED
const MAX_SNAPSHOT_PACKSIZE: usize = 900;
const MAX_MAP_SIZE: i32 = 1024 * 1024 * 1024; // C++: 1 GiB check in NETMSG_MAP_CHANGE
const NUM_INPUTS: usize = 200;
const SHOW_OTHERS_DEFAULT: i32 = 0; // C++: cl_show_others
const SHOW_SPECTATOR_COUNT_DEFAULT: i32 = 1; // C++: cl_showhud_spectator_count
const PREDICTION_MARGIN_DEFAULT_MS: i64 = 10; // C++: cl_prediction_margin

// System message ids. C++: `src/engine/shared/protocol.h`.
const NETMSG_INFO: i32 = 1;
const NETMSG_MAP_CHANGE: i32 = 2;
const NETMSG_MAP_DATA: i32 = 3;
const NETMSG_CON_READY: i32 = 4;
const NETMSG_SNAP: i32 = 5;
const NETMSG_SNAPEMPTY: i32 = 6;
const NETMSG_SNAPSINGLE: i32 = 7;
const NETMSG_INPUTTIMING: i32 = 9;
const NETMSG_READY: i32 = 14;
const NETMSG_ENTERGAME: i32 = 15;
const NETMSG_INPUT: i32 = 16;
const NETMSG_RCON_CMD: i32 = 17;
const NETMSG_REQUEST_MAP_DATA: i32 = 19;
const NETMSG_PING: i32 = 22;
const NETMSG_PING_REPLY: i32 = 23;
// C++: `SERVERCAPFLAG_*` (protocol_ex.h)
const SERVERCAPFLAG_PINGEX: i32 = 1 << 3;
const SERVERCAPFLAG_SYNCWEAPONINPUT: i32 = 1 << 5;

/// Player settings sent in `Cl_StartInfo`. Defaults are the C++ config defaults.
#[derive(Clone, Debug)]
pub struct PlayerInfo {
    pub name: String,
    pub clan: String,
    pub country: i32,
    pub skin: String,
    pub use_custom_color: i32,
    pub color_body: i32,
    pub color_feet: i32,
}

impl Default for PlayerInfo {
    fn default() -> Self {
        Self {
            name: "nameless tee".into(), // C++: CClient::PlayerName() with no name set
            clan: String::new(),
            country: -1,
            skin: "default".into(),
            use_custom_color: 0,
            color_body: 65408,
            color_feet: 65408,
        }
    }
}

/// C++: `IClient::STATE_*` subset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Offline,
    Connecting,
    Loading,
    Online,
}

/// C++: `CServerCapabilities` and `GetServerCapabilities` (client.cpp:1602), 0.6 path.
#[derive(Clone, Copy, Debug, Default)]
/// Only the capabilities this client acts on so far.
struct Capabilities {
    ping_ex: bool,
    sync_weapon_input: bool,
}

impl Capabilities {
    fn new(version: i32, flags: i32) -> Self {
        Self {
            ping_ex: version >= 3 && flags & SERVERCAPFLAG_PINGEX != 0,
            sync_weapon_input: version >= 5 && flags & SERVERCAPFLAG_SYNCWEAPONINPUT != 0,
        }
    }
}

/// C++: `CSmoothTime` (smooth_time.cpp). `Init`/`UpdateInt` take `now` instead of
/// calling `time_get()`.
#[derive(Clone, Copy, Debug, Default)]
struct SmoothTime {
    snap: i64,
    current: i64,
    target: i64,
    margin: i64,
    spike_counter: i32,
    adjust_speed: [f32; 2], // [down, up]
}

const ADJUST_DOWN: usize = 0;
const ADJUST_UP: usize = 1;

impl SmoothTime {
    fn init(&mut self, target: i64, now: i64) {
        *self = Self {
            snap: now,
            current: target,
            target,
            margin: 0,
            spike_counter: 0,
            adjust_speed: [0.3, 0.3],
        };
    }

    fn get(&self, now: i64) -> i64 {
        let c = self.current + (now - self.snap);
        let t = self.target + (now - self.snap);
        // it's faster to adjust upward instead of downward
        let adjust_speed = self.adjust_speed[if t > c { ADJUST_UP } else { ADJUST_DOWN }];
        let a = (((now - self.snap) as f32 / FREQ as f32) * adjust_speed).min(1.0);
        c + ((t - c) as f32 * a) as i64 + self.margin
    }

    fn update_int(&mut self, target: i64, now: i64) {
        self.current = self.get(now) - self.margin;
        self.snap = now;
        self.target = target;
    }

    fn update(&mut self, target: i64, time_left: i32, direction: usize, now: i64) {
        let mut update_timer = true;
        if time_left < 0 {
            let is_spike = time_left < -50;
            if is_spike {
                self.spike_counter = (self.spike_counter + 5).min(50);
            }
            if is_spike && self.spike_counter < 15 {
                update_timer = false; // ignore this ping spike
            } else if self.adjust_speed[direction] < 30.0 {
                self.adjust_speed[direction] *= 2.0;
            }
        } else {
            if self.spike_counter != 0 {
                self.spike_counter -= 1;
            }
            self.adjust_speed[direction] = (self.adjust_speed[direction] * 0.95).max(2.0);
        }
        if update_timer {
            self.update_int(target, now);
        }
    }
}

/// C++: `CSnapshotStorage::CHolder`.
struct StoredSnapshot {
    tick: i32,
    snap: Snapshot,
    items: Vec<Item>,
}

/// C++: `CClient::m_aInputs` entry.
#[derive(Clone, Copy)]
struct SentInput {
    tick: i32,
    data: ObjPlayerInput,
    predicted_time: i64,
    time: i64,
}

struct MapDownload {
    name: String,
    crc: u32,
    size: i32,
    chunk: i32,
    data: Vec<u8>,
}

/// A downloaded, CRC-checked map.
pub struct Map {
    pub name: String,
    pub crc: u32,
    pub data: Vec<u8>,
}

pub struct Client {
    pub conn: Connection,
    state: State,
    player: PlayerInfo,
    password: String,
    connection_id: [u8; 16],
    caps: Capabilities,
    can_receive_caps: bool,
    download: Option<MapDownload>,
    map: Option<Map>,

    // snapshots (C++: m_aSnapshotStorage, m_aapSnapshots, ...)
    static_sizes: [usize; MAX_NETOBJSIZES],
    storage: VecDeque<StoredSnapshot>,
    prev_tick: Option<i32>,
    cur_tick: Option<i32>,
    received_snapshots: i32,
    ack_game_tick: i32,
    current_recv_tick: i32,
    snapshot_parts: u64,
    incoming: Vec<u8>,
    incoming_size: usize,
    snap_crc_errors: i32,

    // timing and input (C++: m_aGameTime, m_PredictedTime, m_aPredTick, m_aInputs)
    game_time: SmoothTime,
    predicted_time: SmoothTime,
    pred_tick: i32,
    /// C++: `IntraGameTick`, `IntraGameTickSincePrev`, `PredIntraGameTick`: how far the
    /// render time is between the previous and current (predicted) tick.
    pub intra_tick: f32,
    pub intra_tick_since_prev: f32,
    pub pred_intra_tick: f32,
    /// C++: `GameTickTime`: seconds since the previous snapshot's tick.
    pub game_tick_time: f32,
    inputs: [SentInput; NUM_INPUTS],
    current_input: usize,
    next_ping: i64,
    /// Wanted input; `send_input` adds the player flags like `CControls::SnapInput`.
    pub input: ObjPlayerInput,
    last_input: ObjPlayerInput,
    last_input_send: i64,

    // game layer (C++: CGameClient)
    local_id: Option<i32>,
    ddrace_msg_sent: bool,
    show_others: i32,
    spectator_count: i32,
    /// Game messages received, oldest first; the caller drains it.
    pub messages: Vec<GameMsg>,
    /// Snapshot switches as (previous tick, new tick), oldest first; the caller drains
    /// it. C++ calls `CGameClient::OnNewSnapshot` for each.
    pub snapshot_switches: Vec<(Option<i32>, i32)>,
    /// C++: `Repredict` in `CClient::Update`: a new snapshot or prediction tick since the
    /// caller last cleared it.
    pub repredict: bool,
    random: fn() -> [u8; 16],
}

impl Client {
    /// `random` must return fresh random bytes; it is used for the connection UUID
    /// and ping ids (C++: `RandomUuid`).
    pub fn new(player: PlayerInfo, password: &str, random: fn() -> [u8; 16]) -> Self {
        let mut static_sizes = [0; MAX_NETOBJSIZES];
        // C++: CGameClient::OnInit, SnapSetStaticsize for every non-extended type
        for spec in g::OBJ_SPECS
            .iter()
            .filter(|s| (s.type_id as usize) < MAX_NETOBJSIZES)
        {
            static_sizes[spec.type_id as usize] = spec.fields.len() * 4;
        }
        Self {
            conn: Connection::default(),
            state: State::Offline,
            player,
            password: password.into(),
            connection_id: [0; 16],
            caps: Capabilities::default(),
            can_receive_caps: false,
            download: None,
            map: None,
            static_sizes,
            storage: VecDeque::new(),
            prev_tick: None,
            cur_tick: None,
            received_snapshots: 0,
            ack_game_tick: -1,
            current_recv_tick: 0,
            snapshot_parts: 0,
            incoming: vec![0; snapshot::MAX_SIZE],
            incoming_size: 0,
            snap_crc_errors: 0,
            game_time: SmoothTime::default(),
            predicted_time: SmoothTime::default(),
            pred_tick: 0,
            intra_tick: 0.0,
            intra_tick_since_prev: 0.0,
            pred_intra_tick: 0.0,
            game_tick_time: 0.0,
            inputs: [SentInput {
                tick: -1,
                data: ObjPlayerInput::default(),
                predicted_time: 0,
                time: 0,
            }; NUM_INPUTS],
            current_input: 0,
            next_ping: -1,
            input: ObjPlayerInput::default(),
            last_input: ObjPlayerInput::default(),
            last_input_send: 0,
            local_id: None,
            ddrace_msg_sent: false,
            show_others: -1, // C++: SHOW_OTHERS_NOT_SET
            spectator_count: -1,
            messages: Vec::new(),
            snapshot_switches: Vec::new(),
            repredict: false,
            random,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn map(&self) -> Option<&Map> {
        self.map.as_ref()
    }

    pub fn local_id(&self) -> Option<i32> {
        self.local_id
    }

    /// Server tick of the current snapshot. C++: `GameTick`.
    pub fn game_tick(&self) -> Option<i32> {
        self.cur_tick
    }

    /// Server tick of the previous snapshot. C++: `PrevGameTick`.
    pub fn prev_game_tick(&self) -> Option<i32> {
        self.prev_tick
    }

    /// C++: `PredGameTick`.
    pub fn pred_tick(&self) -> i32 {
        self.pred_tick
    }

    /// Validated items of the current snapshot.
    pub fn snapshot(&self) -> &[Item] {
        self.cur_tick
            .and_then(|t| self.stored(t))
            .map_or(&[], |s| &s.items[..])
    }

    /// Validated items of the stored snapshot of `tick`.
    pub fn snapshot_at(&self, tick: i32) -> Option<&[Item]> {
        self.stored(tick).map(|s| &s.items[..])
    }

    /// C++: `CClient::GetInput`: the latest sent input for a tick at or before `tick`.
    pub fn input_at(&self, tick: i32) -> Option<ObjPlayerInput> {
        self.inputs
            .iter()
            .filter(|i| i.tick != -1 && i.tick <= tick)
            .max_by_key(|i| i.tick)
            .map(|i| i.data)
    }

    fn stored(&self, tick: i32) -> Option<&StoredSnapshot> {
        self.storage.iter().rev().find(|s| s.tick == tick)
    }

    /// C++: `CClient::Connect` network part.
    pub fn connect(&mut self, now: i64) {
        self.connection_id = random_uuid(self.random);
        self.state = State::Connecting;
        self.conn.connect(now);
    }

    /// C++: `CClient::DisconnectWithReason` (client.cpp:735); `None` sends no reason.
    pub fn disconnect(&mut self, reason: Option<&str>, now: i64) {
        let reason = reason.filter(|r| !r.is_empty());
        self.conn.disconnect(reason, now);
        self.state = State::Offline;
        self.download = None;
    }

    /// One client frame: network timers, received datagrams, snapshot switching and
    /// input. C++: `CClient::Update` -> `PumpNetwork`.
    pub fn frame(&mut self, incoming: impl IntoIterator<Item = Vec<u8>>, now: i64) {
        self.conn.update(now);
        if self.state != State::Offline && self.conn.state() == ConnState::Offline {
            self.state = State::Offline; // C++: "offline error"
            return;
        }
        if self.state == State::Connecting && self.conn.state() == ConnState::Online {
            self.state = State::Loading; // connected, sending info
            self.send_info(now);
        }
        for datagram in incoming {
            self.on_datagram(&datagram, now);
        }
        if self.state == State::Online {
            self.update_online(now);
        }
    }

    /// C++: `CNetClient::Recv` + `ProcessServerPacket` for connection-oriented packets.
    fn on_datagram(&mut self, datagram: &[u8], now: i64) {
        let Some(mut p) = packet::decode(datagram) else {
            return;
        };
        if p.flags & PACKETFLAG_CONNLESS != 0
            || !matches!(self.conn.state(), ConnState::Connect | ConnState::Online)
        {
            return;
        }
        if !self.conn.feed(&mut p, now)
            || p.flags & PACKETFLAG_CONTROL != 0
            || p.data.is_empty()
            || p.num_chunks == 0
        {
            return;
        }
        let mut chunks = ChunkUnpacker::new(p);
        while let Some(chunk) = self.conn.next_chunk(&mut chunks) {
            if self.conn.state() == ConnState::Offline {
                break;
            }
            self.process_chunk(&chunk.data, chunk.flags & CHUNKFLAG_VITAL != 0, now);
        }
    }

    fn send_msg(
        &mut self,
        id: i32,
        system: bool,
        body: &Packer,
        vital: bool,
        flush: bool,
        now: i64,
    ) {
        if self.state == State::Offline {
            return;
        }
        let data = pack_msg(id, system, body.data());
        self.conn.send(&data, vital, flush, now);
    }

    /// C++: `CClient::SendInfo`.
    fn send_info(&mut self, now: i64) {
        let mut p = Packer::default();
        p.add_raw(&self.connection_id);
        p.add_int(DDNET_VERSION);
        p.add_string(VERSION_STRING.as_bytes(), 0, true);
        self.send_msg(g::NETMSG_CLIENTVER, true, &p, true, false, now);

        let mut p = Packer::default();
        p.add_string(GAME_NETVERSION.as_bytes(), 0, true);
        p.add_string(self.password.as_bytes(), 0, true);
        self.send_msg(NETMSG_INFO, true, &p, true, true, now);
        self.can_receive_caps = true;
    }

    /// C++: `CClient::ProcessServerPacket` (client.cpp:1638), 0.6 main connection.
    fn process_chunk(&mut self, data: &[u8], vital: bool, now: i64) {
        let mut u = Unpacker::new(data);
        let Some((id, system)) = unpack_msg_id(&mut u) else {
            return;
        };
        if !system {
            // the client handles only vital game messages (and PREINPUT)
            if (vital || id == g::NETMSGTYPE_SV_PREINPUT)
                && let Ok(msg) = g::unpack_game_msg(id, &mut u)
            {
                if let GameMsg::SvReadyToEnter(_) = msg {
                    self.enter_game(now);
                }
                self.messages.push(msg);
            }
            return;
        }
        match id {
            g::NETMSG_CAPABILITIES if vital => {
                if !self.can_receive_caps {
                    return;
                }
                let (version, flags) = (u.get_int(), u.get_int());
                if u.error() || version <= 0 {
                    return;
                }
                self.caps = Capabilities::new(version, flags);
                self.can_receive_caps = false;
            }
            NETMSG_MAP_CHANGE if vital => self.on_map_change(&mut u, now),
            NETMSG_MAP_DATA => self.on_map_data(&mut u, now),
            NETMSG_CON_READY if vital => {
                if self.map.is_none() {
                    return;
                }
                self.on_connected(now);
            }
            NETMSG_PING => {
                self.send_msg(
                    NETMSG_PING_REPLY,
                    true,
                    &Packer::default(),
                    vital,
                    true,
                    now,
                );
            }
            g::NETMSG_PINGEX => {
                let Some(uuid) = u.get_raw(16) else { return };
                let mut p = Packer::default();
                p.add_raw(uuid);
                self.send_msg(g::NETMSG_PONGEX, true, &p, vital, true, now);
            }
            NETMSG_INPUTTIMING => {
                let (input_pred_tick, time_left) = (u.get_int(), u.get_int());
                if u.error() {
                    return;
                }
                // adjust our prediction time
                let Some(input) = self.inputs.iter().find(|i| i.tick == input_pred_tick) else {
                    return;
                };
                let target = input.predicted_time + (now - input.time)
                    - ((time_left as f32 / 1000.0) * FREQ as f32) as i64;
                if target != 0 {
                    self.predicted_time
                        .update(target, time_left, ADJUST_UP, now);
                }
            }
            NETMSG_SNAP | NETMSG_SNAPEMPTY | NETMSG_SNAPSINGLE => self.on_snapshot(id, &mut u, now),
            _ => {}
        }
    }

    /// C++: the `NETMSG_MAP_CHANGE` branch (client.cpp:1716). The map is always
    /// downloaded in-band and kept in memory (PLAN.md §8: no HTTP download).
    fn on_map_change(&mut self, u: &mut Unpacker, now: i64) {
        if self.can_receive_caps {
            self.caps = Capabilities::new(0, 0);
            self.can_receive_caps = false;
        }
        let name = u.get_string(SANITIZE_CC | SKIP_START_WHITESPACES);
        let (crc, size) = (u.get_int(), u.get_int());
        if u.error() {
            return;
        }
        if !(0..=MAX_MAP_SIZE).contains(&size) {
            return self.disconnect(Some("invalid map size"), now);
        }
        if !valid_filename(&name) {
            return self.disconnect(Some("map name is not a valid filename"), now);
        }
        self.map = None;
        self.download = Some(MapDownload {
            name,
            crc: crc as u32,
            size,
            chunk: 0,
            data: Vec::new(),
        });
        self.send_map_request(now);
    }

    fn send_map_request(&mut self, now: i64) {
        let Some(chunk) = self.download.as_ref().map(|d| d.chunk) else {
            return;
        };
        let mut p = Packer::default();
        p.add_int(chunk);
        self.send_msg(NETMSG_REQUEST_MAP_DATA, true, &p, true, true, now);
    }

    /// C++: the `NETMSG_MAP_DATA` branch (client.cpp:1804) + `FinishMapDownload`.
    fn on_map_data(&mut self, u: &mut Unpacker, now: i64) {
        let Some(download) = &mut self.download else {
            return;
        };
        let (last, crc, chunk, size) = (u.get_int(), u.get_int(), u.get_int(), u.get_int());
        let Some(data) = u.get_raw(size) else { return };
        if size <= 0 || crc as u32 != download.crc || chunk != download.chunk {
            return;
        }
        // C++ keeps appending to the file; cap at the announced size (DECISIONS.md)
        if download.data.len() + data.len() > download.size as usize {
            return self.disconnect(Some("map data exceeds the announced size"), now);
        }
        download.data.extend_from_slice(data);
        if last == 0 {
            download.chunk += 1;
            return self.send_map_request(now);
        }
        let download = self.download.take().expect("checked above");
        let got = crc32(&download.data);
        if got != download.crc {
            // C++: LoadMap's "map differs from the server" error
            let reason = format!(
                "map differs from the server. {got:08x} != {:08x}",
                download.crc
            );
            return self.disconnect(Some(&reason), now);
        }
        self.map = Some(Map {
            name: download.name,
            crc: got,
            data: download.data,
        });
        self.send_msg(NETMSG_READY, true, &Packer::default(), true, true, now);
    }

    /// C++: `CGameClient::OnConnected` network part: start info and the `crashmeplx`
    /// rcon line the C++ client always sends.
    fn on_connected(&mut self, now: i64) {
        let msg = g::MsgClStartInfo {
            name: self.player.name.clone(),
            clan: self.player.clan.clone(),
            country: self.player.country,
            skin: self.player.skin.clone(),
            use_custom_color: self.player.use_custom_color,
            color_body: self.player.color_body,
            color_feet: self.player.color_feet,
        };
        let mut p = Packer::default();
        msg.pack(&mut p);
        self.send_msg(g::MsgClStartInfo::ID, false, &p, true, true, now);

        let mut p = Packer::default();
        p.add_string(b"crashmeplx", 0, true);
        self.send_msg(NETMSG_RCON_CMD, true, &p, true, false, now);
    }

    /// C++: `CClient::EnterGame` + `OnEnterGame` (client.cpp:473, 514).
    fn enter_game(&mut self, now: i64) {
        self.send_msg(NETMSG_ENTERGAME, true, &Packer::default(), true, true, now);
        for input in &mut self.inputs {
            input.tick = -1;
        }
        self.current_input = 0;
        self.storage.clear();
        self.prev_tick = None;
        self.cur_tick = None;
        self.received_snapshots = 0;
        self.snapshot_parts = 0;
        self.incoming_size = 0;
        self.snap_crc_errors = 0;
        self.ack_game_tick = -1;
        self.current_recv_tick = 0;
        self.pred_tick = 0;
        self.game_time.init(0, now);
        self.predicted_time.init(0, now);
        self.next_ping = now + FREQ / 2;
    }

    /// C++: the `NETMSG_SNAP*` branch of `ProcessServerPacket` (client.cpp:2109).
    fn on_snapshot(&mut self, msg: i32, u: &mut Unpacker, now: i64) {
        // we are not allowed to process snapshots yet
        if self.state == State::Connecting || self.map.is_none() {
            return;
        }
        let game_tick = u.get_int();
        let delta_tick = game_tick.wrapping_sub(u.get_int());
        let (mut num_parts, mut part) = (1, 0);
        if msg == NETMSG_SNAP {
            num_parts = u.get_int();
            part = u.get_int();
        }
        let (mut crc, mut part_size) = (0u32, 0);
        if msg != NETMSG_SNAPEMPTY {
            crc = u.get_int() as u32;
            part_size = u.get_int();
        }
        let data = u.get_raw(part_size);
        if u.error()
            || !(1..=snapshot::MAX_PARTS as i32).contains(&num_parts)
            || !(0..num_parts).contains(&part)
            || !(0..=MAX_SNAPSHOT_PACKSIZE as i32).contains(&part_size)
        {
            return;
        }
        let data = data.unwrap_or_default();

        // check ack tick to see if we already got a snapshot for that tick
        if game_tick < self.current_recv_tick || game_tick <= self.ack_game_tick {
            return;
        }
        if game_tick != self.current_recv_tick {
            self.snapshot_parts = 0;
            self.current_recv_tick = game_tick;
            self.incoming_size = 0;
        }
        let offset = part as usize * MAX_SNAPSHOT_PACKSIZE;
        self.incoming[offset..offset + data.len()].copy_from_slice(data);
        self.snapshot_parts |= 1 << part;
        if part == num_parts - 1 {
            self.incoming_size =
                (num_parts as usize - 1) * MAX_SNAPSHOT_PACKSIZE + part_size as usize;
        }
        let all = if num_parts == 64 {
            u64::MAX
        } else {
            (1 << num_parts) - 1
        };
        if self.snapshot_parts != all {
            return;
        }
        self.snapshot_parts = 0;

        // find snapshot that we should use as delta
        let empty = Snapshot::default();
        let delta_shot = if delta_tick >= 0 {
            match self.stored(delta_tick) {
                Some(s) => &s.snap,
                None => {
                    // couldn't find the delta snapshot, force the server to resync
                    self.ack_game_tick = -1;
                    return self.send_input(now);
                }
            }
        } else {
            &empty
        };
        let delta = if self.incoming_size == 0 {
            vec![0, 0, 0] // C++: EmptyDelta()
        } else {
            match varint::decompress(
                &self.incoming[..self.incoming_size],
                snapshot::MAX_DELTA_INTS,
            ) {
                Some(delta) => delta,
                None => return,
            }
        };
        let Ok(snap) = snapshot::unpack_delta(delta_shot, &delta, &self.static_sizes) else {
            return;
        };

        if msg != NETMSG_SNAPEMPTY && snap.crc() != crc {
            self.snap_crc_errors += 1;
            if self.snap_crc_errors > 10 {
                // too many errors, send reset
                self.ack_game_tick = -1;
                self.send_input(now);
                self.snap_crc_errors = 0;
            }
            return;
        }
        if self.snap_crc_errors > 0 {
            self.snap_crc_errors -= 1;
        }

        // purge old snapshots
        let purge_tick = [self.prev_tick, self.cur_tick]
            .into_iter()
            .flatten()
            .fold(delta_tick, i32::min);
        self.storage.retain(|s| s.tick >= purge_tick);

        let items = snapshot::unpack_and_validate(&snap);
        self.storage.push_back(StoredSnapshot {
            tick: game_tick,
            snap,
            items,
        });
        self.received_snapshots += 1;

        // we got two snapshots until we see us self as connected
        if self.received_snapshots == 2 {
            // start at 200ms and work from there
            self.predicted_time
                .init(game_tick as i64 * FREQ / TICK_SPEED, now);
            self.predicted_time.adjust_speed[ADJUST_UP] = 1000.0;
            self.predicted_time.margin = self.prediction_margin_ms() * FREQ / 1000;
            self.game_time
                .init((game_tick as i64 - 1) * FREQ / TICK_SPEED, now);
            self.prev_tick = self.storage.front().map(|s| s.tick);
            self.cur_tick = self.storage.back().map(|s| s.tick);
            self.on_new_snapshot(now);
            self.state = State::Online;
        }
        if self.received_snapshots > 2 {
            // adjust game time
            let game_now = self.game_time.get(now);
            let tick_start = game_tick as i64 * FREQ / TICK_SPEED;
            let time_left = (tick_start - game_now) * 1000 / FREQ;
            self.game_time.update(
                (game_tick as i64 - 1) * FREQ / TICK_SPEED,
                time_left as i32,
                ADJUST_DOWN,
                now,
            );
        }
        self.ack_game_tick = game_tick;
    }

    /// C++: `CClient::PredictionMargin`.
    fn prediction_margin_ms(&self) -> i64 {
        if self.caps.sync_weapon_input {
            PREDICTION_MARGIN_DEFAULT_MS
        } else {
            10
        }
    }

    /// C++: the `STATE_ONLINE` part of `CClient::Update` (client.cpp:2900).
    fn update_online(&mut self, now: i64) {
        let Some(mut cur) = self.cur_tick else { return };
        let game_now = self.game_time.get(now);
        let pred_now = self.predicted_time.get(now);
        // switch snapshot
        while let Some(next) = self.storage.iter().map(|s| s.tick).find(|&t| t > cur) {
            if cur as i64 * FREQ / TICK_SPEED >= game_now {
                break;
            }
            self.prev_tick = Some(cur);
            self.cur_tick = Some(next);
            cur = next;
            self.on_new_snapshot(now);
        }
        if let Some(prev) = self.prev_tick {
            // C++ client.cpp:2934: the render fractions between ticks
            let cur_tick_start = cur as i64 * FREQ / TICK_SPEED;
            let prev_tick_start = prev as i64 * FREQ / TICK_SPEED;
            let prev_pred_tick = (pred_now * TICK_SPEED / FREQ) as i32;
            self.intra_tick =
                (game_now - prev_tick_start) as f32 / (cur_tick_start - prev_tick_start) as f32;
            self.intra_tick_since_prev =
                (game_now - prev_tick_start) as f32 / (FREQ / TICK_SPEED) as f32;
            self.game_tick_time = (game_now - prev_tick_start) as f32 / FREQ as f32;
            let cur_pred_start = (prev_pred_tick + 1) as i64 * FREQ / TICK_SPEED;
            let prev_pred_start = prev_pred_tick as i64 * FREQ / TICK_SPEED;
            self.pred_intra_tick =
                (pred_now - prev_pred_start) as f32 / (cur_pred_start - prev_pred_start) as f32;
            let new_pred_tick = prev_pred_tick + 1;
            if (new_pred_tick - prev).abs() > self.max_latency_ticks() {
                // prediction time reset
                self.predicted_time
                    .init(cur as i64 * FREQ / TICK_SPEED + 2 * FREQ / TICK_SPEED, now);
            }
            if new_pred_tick > self.pred_tick {
                self.pred_tick = new_pred_tick;
                self.repredict = true;
                self.send_input(now);
            }
        }
        // periodically ping server
        if self.next_ping >= 0 && now > self.next_ping {
            if self.caps.ping_ex {
                let mut p = Packer::default();
                p.add_raw(&random_uuid(self.random));
                self.send_msg(g::NETMSG_PINGEX, true, &p, false, true, now);
            }
            self.next_ping = now + 600 * FREQ; // ping every 10 minutes
        }
    }

    /// C++: `CClient::MaxLatencyTicks`.
    pub fn max_latency_ticks(&self) -> i32 {
        (TICK_SPEED + self.prediction_margin_ms() * TICK_SPEED / 1000) as i32
    }

    /// C++: `CClient::SendInput` with `CControls::SnapInput` (controls.cpp:241) for a
    /// player who is playing with absolute mouse input.
    fn send_input(&mut self, now: i64) {
        if self.pred_tick <= 0 {
            return;
        }
        let mut input = self.input;
        input.player_flags =
            g::PLAYERFLAG_PLAYING | g::PLAYERFLAG_INPUT_ABSOLUTE | g::PLAYERFLAG_INPUT_MANUAL;
        if input.target_x == 0 && input.target_y == 0 {
            input.target_x = 1;
        }
        let last = self.last_input;
        let send = last.player_flags != input.player_flags
            || last.direction != input.direction
            || last.jump != input.jump
            || last.fire != input.fire
            || last.hook != input.hook
            || last.wanted_weapon != input.wanted_weapon
            || last.next_weapon != input.next_weapon
            || last.prev_weapon != input.prev_weapon
            || now > self.last_input_send + FREQ / 25; // send at least 25 Hz
        self.last_input = input;
        if !send {
            return;
        }
        self.last_input_send = now;

        let ints = input.to_ints();
        let mut p = Packer::default();
        p.add_int(self.ack_game_tick);
        p.add_int(self.pred_tick);
        p.add_int(ints.len() as i32 * 4);
        for v in ints {
            p.add_int(v);
        }
        self.inputs[self.current_input] = SentInput {
            tick: self.pred_tick,
            data: input,
            predicted_time: self.predicted_time.get(now),
            time: now,
        };
        self.current_input = (self.current_input + 1) % NUM_INPUTS;
        self.send_msg(NETMSG_INPUT, true, &p, false, true, now);
    }

    /// C++: the messages `CGameClient::OnNewSnapshot` sends (gameclient.cpp:2255).
    /// `Cl_ShowDistance` and `Cl_CameraInfo` need a camera and come with rendering.
    fn on_new_snapshot(&mut self, now: i64) {
        if let Some(cur) = self.cur_tick {
            self.snapshot_switches.push((self.prev_tick, cur));
            self.repredict = true;
        }
        self.local_id = self.snapshot().iter().find_map(|it| {
            (it.type_id == g::NETOBJTYPE_PLAYERINFO)
                .then(|| g::ObjPlayerInfo::from_ints(&it.data))
                .filter(|p| p.local != 0)
                .map(|p| p.client_id)
        });
        if !self.ddrace_msg_sent && self.local_id.is_some() {
            let mut p = Packer::default();
            p.add_int(DDNET_VERSION);
            self.send_msg(g::NETMSGTYPE_CL_ISDDNETLEGACY, false, &p, true, false, now);
            self.ddrace_msg_sent = true;
        }
        if self.show_others != SHOW_OTHERS_DEFAULT {
            let mut p = Packer::default();
            g::MsgClShowOthers {
                show: SHOW_OTHERS_DEFAULT,
            }
            .pack(&mut p);
            self.send_msg(g::MsgClShowOthers::ID, false, &p, true, false, now);
            self.show_others = SHOW_OTHERS_DEFAULT;
        }
        if self.spectator_count != SHOW_SPECTATOR_COUNT_DEFAULT {
            let mut p = Packer::default();
            g::MsgClEnableSpectatorCount {
                enable: SHOW_SPECTATOR_COUNT_DEFAULT,
            }
            .pack(&mut p);
            self.send_msg(
                g::MsgClEnableSpectatorCount::ID,
                false,
                &p,
                true,
                false,
                now,
            );
            self.spectator_count = SHOW_SPECTATOR_COUNT_DEFAULT;
        }
    }
}

/// C++: `RepackMsg` (client.cpp:139), 0.6 path: `(id << 1) | sys` for plain ids,
/// `sys` + UUID for extended ones.
fn pack_msg(id: i32, system: bool, body: &[u8]) -> Vec<u8> {
    let mut p = Packer::default();
    match ex_uuid(id) {
        None => p.add_int((id << 1) | system as i32),
        Some(uuid) => {
            p.add_int(system as i32);
            p.add_raw(&uuid);
        }
    }
    p.add_raw(body);
    p.data().to_vec()
}

fn ex_uuid(id: i32) -> Option<[u8; 16]> {
    let game = g::MSG_TYPES.iter().find(|m| m.0 == id).and_then(|m| m.2);
    game.or_else(|| g::SYS_EX_MSGS.iter().find(|m| m.0 == id).map(|m| m.2))
}

/// C++: `UnpackMessageId` (protocol_ex.cpp:21). Unknown UUIDs are dropped like C++;
/// `what-is` queries are not answered (no server sends them, DECISIONS.md).
fn unpack_msg_id(u: &mut Unpacker) -> Option<(i32, bool)> {
    let msg_id = u.get_int();
    if u.error() {
        return None;
    }
    let (id, system) = (msg_id >> 1, msg_id & 1 != 0);
    if !(0..0x10000).contains(&id) {
        return None;
    }
    if id != 0 {
        return Some((id, system));
    }
    let uuid: [u8; 16] = u.get_raw(16)?.try_into().ok()?;
    let game = g::MSG_TYPES.iter().find(|m| m.2 == Some(uuid)).map(|m| m.0);
    let id = game.or_else(|| g::SYS_EX_MSGS.iter().find(|m| m.2 == uuid).map(|m| m.0))?;
    Some((id, system))
}

/// Version 4 UUID. C++: `RandomUuid` (uuid_manager.cpp).
fn random_uuid(random: fn() -> [u8; 16]) -> [u8; 16] {
    let mut uuid = random();
    uuid[6] = (uuid[6] & 0x0f) | 0x40;
    uuid[8] = (uuid[8] & 0x3f) | 0x80;
    uuid
}

/// zlib CRC-32, as C++ `CDataFileReader::Crc` computes over the map file.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = flate2::Crc::new();
    crc.update(data);
    crc.sum()
}

/// C++: `str_valid_filename` (str.cpp:196).
pub fn valid_filename(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let (mut prev_space, mut prev_period, mut first_space_checked) = (false, false, false);
    for c in name.chars() {
        let code = c as u32;
        if code <= 0x1F || code == 0x7F || "\\/|:*?<>\"".contains(c) {
            return false; // disallowed characters, mostly for Windows
        }
        if c != ' ' && utf8_isspace(code as i32) {
            return false; // only regular spaces
        }
        if c == ' ' {
            if !first_space_checked || prev_space {
                return false; // leading or multiple consecutive spaces
            }
            prev_space = true;
            prev_period = false;
        } else {
            prev_space = false;
            prev_period = c == '.';
            first_space_checked = true;
        }
    }
    if prev_space || prev_period {
        return false; // trailing spaces and periods
    }
    const RESERVED: [&str; 30] = [
        "CON", "PRN", "AUX", "NUL", "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
        "COM8", "COM9", "COM¹", "COM²", "COM³", "LPT0", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5",
        "LPT6", "LPT7", "LPT8", "LPT9", "LPT¹", "LPT²", "LPT³",
    ];
    !RESERVED.iter().any(|r| {
        let (n, r) = (name.as_bytes(), r.as_bytes());
        n.len() >= r.len() && n[..r.len()].eq_ignore_ascii_case(r) && {
            let rest = &n[r.len()..];
            rest.is_empty() || rest[0] == b'.' || rest.starts_with(b" .")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_zlib() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn valid_filenames() {
        for ok in ["coverage", "Gold Mine", "a.b", "COMx", "nul2"] {
            assert!(valid_filename(ok), "{ok}");
        }
        for bad in [
            "",
            " a",
            "a ",
            "a  b",
            "a.",
            "a/b",
            "CON",
            "con.map",
            "LPT¹",
            "a\u{00A0}b",
            "NUL .x",
        ] {
            assert!(!valid_filename(bad), "{bad}");
        }
    }

    #[test]
    fn message_ids_round_trip() {
        for (id, sys) in [
            (NETMSG_INFO, true),
            (g::NETMSG_CLIENTVER, true),
            (g::MsgClShowOthers::ID, false),
            (g::NETMSGTYPE_CL_STARTINFO, false),
        ] {
            let data = pack_msg(id, sys, &[]);
            assert_eq!(unpack_msg_id(&mut Unpacker::new(&data)), Some((id, sys)));
        }
    }
}
