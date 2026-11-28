//! Client protocol flow, sans-IO: the main connection and the dummy's. C++: the network
//! parts of `CClient` (`src/engine/client/client.cpp`), `CSmoothTime`
//! (`smooth_time.cpp`) and the messages `CGameClient` sends (`gameclient.cpp`). No
//! demos or 0.7.

use std::collections::VecDeque;

use super::conn::{ChunkUnpacker, Connection, FREQ, State as ConnState};
use super::serverinfo::{self, ServerInfo};
use crate::physics::vmath::trunc_i32;
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
const SHOW_OTHERS_NOT_SET: i32 = -1;
const SHOW_SPECTATOR_COUNT_DEFAULT: i32 = 1; // C++: cl_showhud_spectator_count

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
const NETMSG_RCON_AUTH: i32 = 18;
const NETMSG_REQUEST_MAP_DATA: i32 = 19;
const NETMSG_PING: i32 = 22;
const NETMSG_PING_REPLY: i32 = 23;
// C++: `SERVERCAPFLAG_*` (protocol_ex.h)
const SERVERCAPFLAG_PINGEX: i32 = 1 << 3;
const SERVERCAPFLAG_ALLOWDUMMY: i32 = 1 << 4;
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

/// What `CGameClient::OnNewSnapshot` reads from the camera and the window to send
/// `Cl_ShowDistance` and `Cl_CameraInfo` (gameclient.cpp:2313). The defaults are a
/// C++ client with default settings in a 16:9 window that never zooms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraState {
    /// C++ `m_Camera.m_Zoom`, `m_Zooming`, `m_ZoomSmoothingTarget`.
    pub zoom: f32,
    pub zooming: bool,
    pub zoom_target: f32,
    /// C++ `m_Camera.Deadzone()`, `FollowFactor()`.
    pub deadzone: i32,
    pub follow_factor: i32,
    /// C++ `Graphics()->ScreenAspect()`: window width / height.
    pub aspect: f32,
}

impl Default for CameraState {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            zooming: false,
            zoom_target: 0.0,
            deadzone: 0,
            follow_factor: 0,
            aspect: 16.0 / 9.0,
        }
    }
}

/// C++: `IGraphics::CalcScreenParams` (engine/graphics.cpp): the world area a screen
/// with this aspect shows at this zoom, as (width, height).
pub fn calc_screen_params(aspect: f32, zoom: f32) -> (f32, f32) {
    const AMOUNT: f32 = 1150.0 * 1000.0;
    const W_MAX: f32 = 1500.0;
    const H_MAX: f32 = 1050.0;
    let f = AMOUNT.sqrt() / aspect.sqrt();
    let (mut w, mut h) = (f * aspect, f);
    if w > W_MAX {
        w = W_MAX;
        h = w / aspect;
    }
    if h > H_MAX {
        h = H_MAX;
        w = h * aspect;
    }
    (w * zoom, h * zoom)
}

/// C++: `m_Snap.m_SpecInfo.m_Active` (gameclient.cpp:1805, :1845): the local player is
/// in the spectator team, paused or spectating with /spec.
pub fn spectating(items: &[Item], local_id: Option<i32>) -> bool {
    let Some(id) = local_id else { return false };
    let find = |t: i32| items.iter().find(|it| it.type_id == t && it.id == id);
    let info = |it: &Item| crate::items::pad(&it.data, g::ObjPlayerInfo::NUM_INTS);
    let team = find(g::NETOBJTYPE_PLAYERINFO).map(|it| g::ObjPlayerInfo::from_ints(&info(it)).team);
    let flags = find(g::NETOBJTYPE_DDNETPLAYER).map_or(0, |it| {
        crate::items::pad(&it.data, g::ObjDDNetPlayer::NUM_INTS)[0]
    });
    team == Some(g::TEAM_SPECTATORS) || flags & (g::EXPLAYERFLAG_PAUSED | g::EXPLAYERFLAG_SPEC) != 0
}

/// A game message the prediction world needs, in the form the C++ handler reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PredictMsg {
    /// `Sv_TuneParams` on connection 0 (main) or 1 (dummy): the ints read before the
    /// first unpack error.
    TuneParams(usize, Vec<i32>),
    /// `Sv_TeamsState` (or its legacy form): the team ints read before the first
    /// unpack error (at most `MAX_CLIENTS`).
    TeamsState(Vec<i32>),
    KillMsg(g::MsgSvKillMsg),
    KillMsgTeam(g::MsgSvKillMsgTeam),
    PreInput(g::MsgSvPreInput),
}

/// Up to `n` ints, stopping at the first unpack error (C++ checks `Error()` after each
/// `GetInt`).
pub fn read_ints(u: &mut Unpacker, n: usize) -> Vec<i32> {
    let mut out = Vec::new();
    while out.len() < n {
        let v = u.get_int();
        if u.error() {
            break;
        }
        out.push(v);
    }
    out
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
    allow_dummy: bool,
    sync_weapon_input: bool,
}

impl Capabilities {
    fn new(version: i32, flags: i32) -> Self {
        Self {
            ping_ex: version >= 3 && flags & SERVERCAPFLAG_PINGEX != 0,
            allow_dummy: version < 4 || flags & SERVERCAPFLAG_ALLOWDUMMY != 0,
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

/// The render fractions of `CClient::Update` (client.cpp:2934).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IntraTicks {
    /// C++ `IntraGameTick`: between the previous and the current snapshot.
    pub intra: f32,
    /// C++ `IntraGameTickSincePrev`.
    pub since_prev: f32,
    /// C++ `GameTickTime`: seconds since the previous snapshot's tick.
    pub tick_time: f32,
    /// C++ `PredIntraGameTick`: within the predicted tick.
    pub pred_intra: f32,
    pub prev_pred_tick: i32,
}

/// C++: the render fractions of `CClient::Update` (client.cpp:2934) for the previous
/// and current snapshot ticks and the game and predicted times (`FREQ` units).
pub fn intra_ticks(prev_tick: i32, cur_tick: i32, game_now: i64, pred_now: i64) -> IntraTicks {
    let cur_tick_start = cur_tick as i64 * FREQ / TICK_SPEED;
    let prev_tick_start = prev_tick as i64 * FREQ / TICK_SPEED;
    let prev_pred_tick = (pred_now * TICK_SPEED / FREQ) as i32;
    let cur_pred_start = prev_pred_tick.wrapping_add(1) as i64 * FREQ / TICK_SPEED;
    let prev_pred_start = prev_pred_tick as i64 * FREQ / TICK_SPEED;
    IntraTicks {
        intra: (game_now - prev_tick_start) as f32 / (cur_tick_start - prev_tick_start) as f32,
        since_prev: (game_now - prev_tick_start) as f32 / (FREQ / TICK_SPEED) as f32,
        tick_time: (game_now - prev_tick_start) as f32 / FREQ as f32,
        pred_intra: (pred_now - prev_pred_start) as f32 / (cur_pred_start - prev_pred_start) as f32,
        prev_pred_tick,
    }
}

/// C++: `CClient::GetPredictionTime` (client.cpp:5383) for the smoothed game and
/// predicted times at `now`: how far ahead prediction runs, in ms.
pub fn prediction_time(game_now: i64, pred_now: i64) -> i32 {
    (((pred_now - game_now) * 1000) as f32 / FREQ as f32) as i32
}

/// C++: `CClient::GetPredictionTick` (client.cpp:5389): the tick other players are
/// predicted to, from `cl_antiping_limit` (ms) and `cl_antiping_percent`.
pub fn prediction_tick(
    prediction_time: i32,
    limit: i32,
    percent: i32,
    game_tick: i32,
    pred_tick: i32,
) -> i32 {
    let tick_speed = TICK_SPEED as i32;
    let prediction_tick = ((prediction_time * tick_speed) as f32 / 1000.0) as i32;
    let mut prediction_min = ((limit * tick_speed) as f32 / 1000.0) as i32;
    if limit == 0 {
        let percentage = 1.0 - percent as f32 / 100.0;
        prediction_min = (prediction_tick as f32 * percentage).floor() as i32;
    }
    if prediction_min > prediction_tick - 1 {
        prediction_min = prediction_tick - 1;
    }
    if prediction_min <= 0 {
        return pred_tick;
    }
    (pred_tick - prediction_min).max(game_tick + 1)
}

/// C++: `CClient::GetSmoothTick` (client.cpp:5418): the tick and intra tick at
/// `mix` between the game and the predicted time.
pub fn smooth_tick(game_now: i64, pred_now: i64, mix: f32) -> (i32, f32) {
    let v = game_now + (mix * (pred_now - game_now) as f32) as i64;
    // std::clamp as MSVC's STL does it: the upper bound wins when the bounds are
    // reversed (predicted time behind game time; C++ UB, see DECISIONS.md)
    let smooth_time = if pred_now < v {
        pred_now
    } else if v < game_now {
        game_now
    } else {
        v
    };
    let tick = (smooth_time * TICK_SPEED / FREQ) as i32 + 1;
    let intra =
        (smooth_time - (tick - 1) as i64 * FREQ / TICK_SPEED) as f32 / (FREQ / TICK_SPEED) as f32;
    (tick, intra)
}

/// What `CGameClient::OnPredict` and the rendering read from `CClient` at one moment.
#[derive(Clone, Copy, Debug, Default)]
pub struct Timing {
    /// C++: `GameTick`, `PrevGameTick`, `PredGameTick`.
    pub game_tick: i32,
    pub prev_game_tick: i32,
    pub pred_tick: i32,
    /// C++: `IntraGameTick`, `PredIntraGameTick`.
    pub intra_tick: f32,
    pub pred_intra_tick: f32,
    /// C++: `time_get()` and the smoothed game and predicted times at it.
    pub now: i64,
    pub game_now: i64,
    pub pred_now: i64,
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

/// One connection's part of `CClient` (`m_aNetClient[i]`, `m_aSnapshotStorage[i]`,
/// `m_aInputs[i]`, ...) and of `CGameClient` (`m_aLocalIds[i]`, ...). Index 0 is the
/// main connection, 1 the dummy's (C++ `CONN_MAIN`, `CONN_DUMMY`).
struct Link {
    net: Connection,
    storage: VecDeque<StoredSnapshot>,
    prev_tick: Option<i32>,
    cur_tick: Option<i32>,
    received_snapshots: i32,
    ack_game_tick: i32,
    current_recv_tick: i32,
    snapshot_parts: u64,
    incoming: Vec<u8>,
    incoming_size: usize,
    game_time: SmoothTime,
    pred_tick: i32,
    inputs: [SentInput; NUM_INPUTS],
    current_input: usize,
    /// C++ `m_aLocalIds[i]`: learned while this connection is the active one.
    local_id: Option<i32>,
    /// C++ `m_aDDRaceMsgSent[i]`, `m_aShowOthers[i]`, `m_aEnableSpectatorCount[i]`.
    ddrace_msg_sent: bool,
    show_others: i32,
    spectator_count: i32,
}

impl Default for Link {
    fn default() -> Self {
        Self {
            net: Connection::default(),
            storage: VecDeque::new(),
            prev_tick: None,
            cur_tick: None,
            received_snapshots: 0,
            ack_game_tick: -1,
            current_recv_tick: 0,
            snapshot_parts: 0,
            incoming: vec![0; snapshot::MAX_SIZE],
            incoming_size: 0,
            game_time: SmoothTime::default(),
            pred_tick: 0,
            inputs: [SentInput {
                tick: -1,
                data: ObjPlayerInput::default(),
                predicted_time: 0,
                time: 0,
            }; NUM_INPUTS],
            current_input: 0,
            local_id: None,
            ddrace_msg_sent: false,
            show_others: SHOW_OTHERS_NOT_SET,
            spectator_count: -1,
        }
    }
}

pub struct Client {
    links: [Link; 2],
    state: State,
    player: PlayerInfo,
    /// What `Cl_StartInfo` of the dummy sends (C++ `DummyName()` and the `dummy_*`
    /// settings); the caller keeps it current.
    pub dummy_player: PlayerInfo,
    password: String,
    connection_id: [u8; 16],
    caps: Capabilities,
    can_receive_caps: bool,
    download: Option<MapDownload>,
    map: Option<Map>,
    static_sizes: [usize; MAX_NETOBJSIZES],
    snap_crc_errors: i32,

    // timing and input (C++: m_PredictedTime, and the render fractions of the active
    // connection)
    predicted_time: SmoothTime,
    /// C++: `IntraGameTick`, `IntraGameTickSincePrev`, `PredIntraGameTick` of the active
    /// connection: how far the render time is between the previous and current
    /// (predicted) tick.
    pub intra_tick: f32,
    pub intra_tick_since_prev: f32,
    pub pred_intra_tick: f32,
    /// C++: `GameTickTime`: seconds since the previous snapshot's tick.
    pub game_tick_time: f32,
    next_ping: i64,
    /// The input to send for the active tee (C++ `CControls::SnapInput`'s result).
    /// Starts as a player playing with an absolute mouse.
    pub input: ObjPlayerInput,
    /// C++ `CControls::m_aLastData` (as far as the send rules read it) and
    /// `m_LastSendTime`.
    last_input: [ObjPlayerInput; 2],
    last_input_send: i64,

    // the dummy (C++: CClient m_DummyConnected ..., CGameClient m_DummyInput ...)
    /// C++ `cl_dummy`: the dummy is the active tee.
    dummy: bool,
    /// C++ `m_LastDummy`: `cl_dummy` at the end of the last online update.
    last_dummy: bool,
    dummy_connected: bool,
    dummy_connecting: bool,
    dummy_send_conn_info: bool,
    /// C++ `m_LastDummyConnectTime` (0: none since entering the game).
    last_dummy_connect_time: i64,
    /// C++ `CGameClient::m_LastDummyConnected`.
    last_dummy_connected: bool,
    /// C++ `m_DummyInput`, `m_HammerInput`, `m_DummyFire`: what the other tee sends.
    /// The controls keep its input current (copy moves, dummy control, swaps).
    pub dummy_snap: DummySnap,
    /// C++ `cl_dummy_hammer`, `cl_dummy_restore_weapon`, `cl_dummy_copy_moves`; the
    /// caller keeps them current.
    pub dummy_hammer: bool,
    pub dummy_restore_weapon: bool,
    pub dummy_copy_moves: bool,
    /// C++ `m_LocalCharacterPos - m_aClients[m_aLocalIds[!cl_dummy]].m_Predicted.m_Pos`:
    /// where the dummy's hammer aims; the caller keeps it current.
    pub hammer_dir: (f32, f32),
    /// Set when the dummy connection dropped while connecting (C++ echoes "Could not
    /// connect dummy"); the caller takes it.
    pub dummy_error: Option<String>,

    // game layer (C++: CGameClient)
    /// C++ `m_Snap.m_LocalClientId`.
    snap_local_id: Option<i32>,
    /// The camera to report; the caller keeps it current. C++ `m_LastShowDistanceZoom`,
    /// `m_LastZoom`, `m_LastScreenAspect`, `m_LastDeadzone`, `m_LastFollowFactor`.
    pub camera: CameraState,
    last_show_distance_zoom: f32,
    last_zoom: f32,
    last_screen_aspect: f32,
    last_deadzone: f32,
    last_follow_factor: f32,
    /// Game messages received on the active connection, oldest first; the caller
    /// drains it.
    pub messages: Vec<GameMsg>,
    /// `Sv_Chat` lines of the other connection while both tees are known (C++
    /// `OnMessage` with `Dummy`); the caller drains them and keeps the team and
    /// whisper lines only the other tee gets (gameclient.cpp:1115).
    pub passive_chat: Vec<g::MsgSvChat>,
    /// The game messages that change the prediction world, in arrival order; the
    /// predictor drains it (`Predictor::update`).
    pub predict_msgs: Vec<PredictMsg>,
    /// Snapshot switches as (previous tick, new tick), oldest first; the caller drains
    /// it. C++ calls `CGameClient::OnNewSnapshot` for each.
    pub snapshot_switches: Vec<(Option<i32>, i32)>,
    /// C++: `Repredict` in `CClient::Update`: a new snapshot or prediction tick since the
    /// caller last cleared it.
    pub repredict: bool,
    /// C++: `cl_prediction_margin` (ms), used when the server syncs weapon input.
    pub prediction_margin: i32,
    random: fn() -> [u8; 16],
    /// C++ `m_CurrentServerInfo` and `m_CurrentServerInfoRequestTime` (-1 once received).
    server_info: ServerInfo,
    info_request_time: i64,
    /// C++ `CServerBrowser::GenerateToken` for the server: 24 bits from a random seed.
    info_token: i32,
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
            links: Default::default(),
            state: State::Offline,
            // C++ DummyName() without dummy_name
            dummy_player: PlayerInfo {
                name: format!("[D] {}", player.name),
                ..PlayerInfo::default()
            },
            player,
            password: password.into(),
            connection_id: [0; 16],
            caps: Capabilities::default(),
            can_receive_caps: false,
            download: None,
            map: None,
            static_sizes,
            snap_crc_errors: 0,
            predicted_time: SmoothTime::default(),
            intra_tick: 0.0,
            intra_tick_since_prev: 0.0,
            pred_intra_tick: 0.0,
            game_tick_time: 0.0,
            next_ping: -1,
            input: ObjPlayerInput {
                player_flags: g::PLAYERFLAG_PLAYING
                    | g::PLAYERFLAG_INPUT_ABSOLUTE
                    | g::PLAYERFLAG_INPUT_MANUAL,
                ..Default::default()
            },
            last_input: Default::default(),
            last_input_send: 0,
            dummy: false,
            last_dummy: false,
            dummy_connected: false,
            dummy_connecting: false,
            dummy_send_conn_info: false,
            last_dummy_connect_time: 0,
            last_dummy_connected: false,
            dummy_snap: DummySnap::default(),
            dummy_hammer: false,
            dummy_restore_weapon: true,
            dummy_copy_moves: false,
            hammer_dir: (0.0, 0.0),
            dummy_error: None,
            snap_local_id: None,
            camera: CameraState::default(),
            last_show_distance_zoom: 0.0,
            last_zoom: 0.0,
            last_screen_aspect: 0.0,
            last_deadzone: 0.0,
            last_follow_factor: 0.0,
            messages: Vec::new(),
            passive_chat: Vec::new(),
            predict_msgs: Vec::new(),
            snapshot_switches: Vec::new(),
            repredict: false,
            prediction_margin: 10,
            random,
            server_info: ServerInfo::default(),
            info_request_time: 0,
            info_token: {
                let r = random();
                (r[0] as i32) << 16 | (r[1] as i32) << 8 | r[2] as i32
            },
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn map(&self) -> Option<&Map> {
        self.map.as_ref()
    }

    /// The active connection (C++ `g_Config.m_ClDummy`).
    fn active(&self) -> &Link {
        &self.links[self.dummy as usize]
    }

    /// The main connection's network state.
    pub fn conn(&self) -> &Connection {
        &self.links[0].net
    }

    /// Datagrams to send on connection `i` (0 main, 1 dummy).
    pub fn outbox(&mut self, i: usize) -> &mut Vec<Vec<u8>> {
        &mut self.links[i].net.outbox
    }

    /// C++ `m_Snap.m_LocalClientId`: the active tee's client id in the current
    /// snapshot.
    pub fn local_id(&self) -> Option<i32> {
        self.snap_local_id
    }

    /// C++ `m_aLocalIds`: the main tee's and the dummy's client ids.
    pub fn local_ids(&self) -> [Option<i32>; 2] {
        [self.links[0].local_id, self.links[1].local_id]
    }

    /// C++ `cl_dummy`: the dummy is the active tee.
    pub fn dummy(&self) -> bool {
        self.dummy
    }

    /// C++: setting `cl_dummy` (`ConchainSpecialDummy`): only while the dummy is
    /// connected.
    pub fn set_dummy(&mut self, on: bool) {
        self.dummy = on && self.dummy_connected;
    }

    /// C++: `m_LastDummy != cl_dummy` while online: the next `update` swaps the tees
    /// (the caller runs `CGameClient::OnDummySwap` first).
    pub fn swapping(&self) -> bool {
        self.state == State::Online && self.last_dummy != self.dummy
    }

    /// C++: `DummyConnected`, `DummyConnecting`.
    pub fn dummy_connected(&self) -> bool {
        self.dummy_connected
    }

    pub fn dummy_connecting(&self) -> bool {
        self.dummy_connecting
    }

    /// C++: `DummyConnectingDelayed`: within 5 s of the last connect.
    pub fn dummy_connecting_delayed(&self, now: i64) -> bool {
        !self.dummy_connected
            && !self.dummy_connecting
            && self.last_dummy_connect_time > 0
            && self.last_dummy_connect_time + 5 * FREQ > now
    }

    /// C++: `DummyAllowed`.
    pub fn dummy_allowed(&self) -> bool {
        self.caps.allow_dummy
    }

    /// Server tick of the active connection's current snapshot. C++: `GameTick`.
    pub fn game_tick(&self) -> Option<i32> {
        self.active().cur_tick
    }

    /// Server tick of the previous snapshot. C++: `PrevGameTick`.
    pub fn prev_game_tick(&self) -> Option<i32> {
        self.active().prev_tick
    }

    /// C++: `PredGameTick`.
    pub fn pred_tick(&self) -> i32 {
        self.active().pred_tick
    }

    /// Validated items of the active connection's current snapshot.
    pub fn snapshot(&self) -> &[Item] {
        self.active()
            .cur_tick
            .and_then(|t| self.stored(t))
            .map_or(&[], |s| &s.items[..])
    }

    /// Validated items of the active connection's stored snapshot of `tick`.
    pub fn snapshot_at(&self, tick: i32) -> Option<&[Item]> {
        self.stored(tick).map(|s| &s.items[..])
    }

    /// C++: `CClient::GetInput(tick)`: the latest input sent for the active tee for a
    /// tick at or before `tick`.
    pub fn input_at(&self, tick: i32) -> Option<ObjPlayerInput> {
        self.input_of(tick, false)
    }

    /// C++: `CClient::GetInput(tick, is_dummy)` (client.cpp:415): `is_dummy` picks the
    /// other tee. Of several inputs for the same tick (sent again right after a dummy
    /// swap) the one at the lowest ring index wins, like C++ and the server.
    pub fn input_of(&self, tick: i32, is_dummy: bool) -> Option<ObjPlayerInput> {
        let inputs = &self.links[(is_dummy ^ self.dummy) as usize].inputs;
        let mut best: Option<&SentInput> = None;
        for i in inputs.iter().filter(|i| i.tick != -1 && i.tick <= tick) {
            if best.is_none_or(|b| b.tick < i.tick) {
                best = Some(i);
            }
        }
        best.map(|i| i.data)
    }

    /// C++: `m_Snap.m_pLocalCharacter`: the local character of the current snapshot.
    pub fn local_character(&self) -> Option<g::ObjCharacter> {
        let id = self.local_id()?;
        let it = self
            .snapshot()
            .iter()
            .find(|it| it.type_id == g::NETOBJTYPE_CHARACTER && it.id == id)?;
        Some(g::ObjCharacter::from_ints(&crate::items::pad(
            &it.data,
            g::ObjCharacter::NUM_INTS,
        )))
    }

    fn stored(&self, tick: i32) -> Option<&StoredSnapshot> {
        self.active().storage.iter().rev().find(|s| s.tick == tick)
    }

    /// C++: `CClient::Connect` network part.
    pub fn connect(&mut self, now: i64) {
        self.connection_id = random_uuid(self.random);
        self.server_info_request();
        self.state = State::Connecting;
        self.links[0].net.connect(now);
    }

    /// C++: `CClient::DisconnectWithReason` (client.cpp:735); `None` sends no reason.
    pub fn disconnect(&mut self, reason: Option<&str>, now: i64) {
        let reason = reason.filter(|r| !r.is_empty());
        self.dummy_disconnect(reason, now);
        let main = &mut self.links[0];
        main.net.disconnect(reason, now);
        self.state = State::Offline;
        self.download = None;
        (main.prev_tick, main.cur_tick, main.received_snapshots) = (None, None, 0);
        self.last_dummy = false;
    }

    /// C++: `CClient::DummyConnect` (client.cpp:816). `Err` holds the C++ log line (empty
    /// when C++ logs nothing). On `Ok` the caller turns `cl_dummy_copy_moves` and
    /// `cl_dummy_hammer` off, as C++ does.
    pub fn dummy_connect(&mut self, now: i64) -> Result<(), &'static str> {
        if self.links[0].net.state() != ConnState::Online {
            return Err("Not online.");
        }
        if !self.dummy_allowed() {
            return Err("Dummy is not allowed on this server.");
        }
        if self.dummy_connecting {
            return Err("Dummy is already connecting.");
        }
        if self.dummy_connected {
            return Err("");
        }
        if self.dummy_connecting_delayed(now) {
            return Err("Wait before connecting dummy again.");
        }
        self.last_dummy_connect_time = now;
        self.dummy_send_conn_info = true;
        self.dummy_connecting = true;
        self.links[1].net.connect(now);
        Ok(())
    }

    /// C++: `CClient::DummyDisconnect` (client.cpp:865) and
    /// `CGameClient::OnDummyDisconnect`.
    pub fn dummy_disconnect(&mut self, reason: Option<&str>, now: i64) {
        let d = &mut self.links[1];
        d.net.disconnect(reason, now);
        self.dummy = false;
        (d.prev_tick, d.cur_tick, d.received_snapshots) = (None, None, 0);
        self.dummy_connected = false;
        self.dummy_connecting = false;
        d.local_id = None;
        d.ddrace_msg_sent = false;
        d.show_others = SHOW_OTHERS_NOT_SET;
        d.spectator_count = -1;
    }

    /// One client frame without a game layer in between: `pump_network`, then
    /// `update`.
    pub fn frame(&mut self, incoming: [Vec<Vec<u8>>; 2], now: i64) {
        self.pump_network(incoming, now);
        self.update(now);
    }

    /// Network timers and the received datagrams of both connections. C++:
    /// `CClient::PumpNetwork` (client.cpp:2671).
    pub fn pump_network(&mut self, incoming: [Vec<Vec<u8>>; 2], now: i64) {
        for link in &mut self.links {
            link.net.update(now);
        }
        if self.state != State::Offline {
            if self.links[0].net.state() == ConnState::Offline {
                // C++: "offline error", Disconnect() also drops the dummy
                self.dummy_disconnect(None, now);
                self.state = State::Offline;
                return;
            }
            if (self.dummy_connecting || self.dummy_connected)
                && self.links[1].net.state() == ConnState::Offline
            {
                let was_connecting = self.dummy_connecting;
                self.dummy_disconnect(None, now);
                if was_connecting {
                    self.dummy_error = Some(self.links[1].net.error().to_owned());
                }
            }
        }
        if self.state == State::Connecting && self.links[0].net.state() == ConnState::Online {
            self.state = State::Loading; // connected, sending info
            self.send_info(0, now);
        }
        // progress on dummy connect when the connection is online
        if self.dummy_send_conn_info && self.links[1].net.state() == ConnState::Online {
            self.dummy_send_conn_info = false;
            self.send_info(1, now);
            self.links[1].net.update(now);
            self.send_msg(1, NETMSG_READY, true, &Packer::default(), true, true, now);
            self.send_dummy_info(now);
            self.send_msg(
                1,
                NETMSG_ENTERGAME,
                true,
                &Packer::default(),
                true,
                true,
                now,
            );
        }
        for (c, datagrams) in incoming.into_iter().enumerate() {
            for datagram in datagrams {
                self.on_datagram(c, &datagram, now);
            }
        }
    }

    /// The online part of `CClient::Update` (snapshot switching, prediction tick,
    /// input) and the prediction margin at its end.
    pub fn update(&mut self, now: i64) {
        if self.state == State::Online {
            self.update_online(now);
        }
        // C++: the end of CClient::Update
        self.predicted_time.margin = self.prediction_margin_ms() * FREQ / 1000;
    }

    /// C++: `CNetClient::Recv` + `ProcessServerPacket` for connection-oriented packets of
    /// connection `c`.
    fn on_datagram(&mut self, c: usize, datagram: &[u8], now: i64) {
        let Some(mut p) = packet::decode(datagram) else {
            return;
        };
        if p.flags & PACKETFLAG_CONNLESS != 0 {
            self.on_connless(&p.data);
            return;
        }
        let net = &mut self.links[c].net;
        if !matches!(net.state(), ConnState::Connect | ConnState::Online) {
            return;
        }
        if !net.feed(&mut p, now)
            || p.flags & PACKETFLAG_CONTROL != 0
            || p.data.is_empty()
            || p.num_chunks == 0
        {
            return;
        }
        let mut chunks = ChunkUnpacker::new(p);
        while let Some(chunk) = self.links[c].net.next_chunk(&mut chunks) {
            if self.links[c].net.state() == ConnState::Offline {
                break;
            }
            self.process_chunk(c, &chunk.data, chunk.flags & CHUNKFLAG_VITAL != 0, now);
        }
    }

    /// C++: the send in `CChat::SendChat` (chat.cpp:1382): a chat line, `team` 1 for
    /// the team chat, on the active connection.
    pub fn say(&mut self, team: i32, line: &str, now: i64) {
        let mut p = Packer::default();
        g::MsgClSay {
            team,
            message: line.into(),
        }
        .pack(&mut p);
        self.send_active(g::MsgClSay::ID, &p, now);
    }

    /// C++: `CGameClient::SendSwitchTeam` (gameclient.cpp:3110).
    pub fn send_switch_team(&mut self, team: i32, now: i64) {
        let mut p = Packer::default();
        g::MsgClSetTeam { team }.pack(&mut p);
        self.send_active(g::MsgClSetTeam::ID, &p, now);
    }

    /// C++: `CGameClient::SendKill` (gameclient.cpp:3276): the active tee, and the
    /// other one too with `cl_dummy_copy_moves`.
    pub fn send_kill(&mut self, now: i64) {
        self.send_active(g::MsgClKill::ID, &Packer::default(), now);
        if self.dummy_copy_moves {
            let other = !self.dummy as usize;
            self.send_msg(
                other,
                g::MsgClKill::ID,
                false,
                &Packer::default(),
                true,
                false,
                now,
            );
        }
    }

    /// C++: `SendPackMsgActive(.., MSGFLAG_VITAL)` for a game message.
    fn send_active(&mut self, id: i32, body: &Packer, now: i64) {
        self.send_msg(self.dummy as usize, id, false, body, true, false, now);
    }

    #[allow(clippy::too_many_arguments)]
    fn send_msg(
        &mut self,
        c: usize,
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
        self.links[c].net.send(&data, vital, flush, now);
    }

    /// C++: `CClient::SendInfo(Conn)`.
    fn send_info(&mut self, c: usize, now: i64) {
        let mut p = Packer::default();
        p.add_raw(&self.connection_id);
        p.add_int(DDNET_VERSION);
        p.add_string(VERSION_STRING.as_bytes(), 0, true);
        self.send_msg(c, g::NETMSG_CLIENTVER, true, &p, true, false, now);

        let mut p = Packer::default();
        p.add_string(GAME_NETVERSION.as_bytes(), 0, true);
        p.add_string(self.password.as_bytes(), 0, true);
        self.send_msg(c, NETMSG_INFO, true, &p, true, true, now);
        if c == 0 {
            self.can_receive_caps = true;
        }
    }

    /// C++: `CGameClient::SendDummyInfo(true)` (gameclient.cpp:3234).
    fn send_dummy_info(&mut self, now: i64) {
        let p = start_info(&self.dummy_player);
        self.send_msg(1, g::MsgClStartInfo::ID, false, &p, true, false, now);
    }

    /// C++: `CClient::ProcessServerPacket` (client.cpp:1638), 0.6, for connection `c`.
    fn process_chunk(&mut self, c: usize, data: &[u8], vital: bool, now: i64) {
        let mut u = Unpacker::new(data);
        let Some((id, system)) = unpack_msg_id(&mut u) else {
            return;
        };
        // C++: `Dummy`, the connection that is not the active one
        let passive = c != self.dummy as usize;
        if !system {
            // the client handles only vital game messages (and PREINPUT)
            if !(vital || id == g::NETMSGTYPE_SV_PREINPUT) {
                return;
            }
            // C++: CGameClient::OnMessage handles the tuning before unpacking
            if id == g::NETMSGTYPE_SV_TUNEPARAMS {
                let ints = read_ints(&mut u, crate::physics::tuning::NUM_PARAMS);
                self.predict_msgs.push(PredictMsg::TuneParams(c, ints));
                return;
            }
            let Ok(msg) = g::unpack_game_msg(id, &mut u) else {
                return;
            };
            if passive {
                // no need of all that stuff for the dummy (gameclient.cpp:1109)
                match msg {
                    GameMsg::SvReadyToEnter(_) => self.enter_game(c, now),
                    GameMsg::SvChat(m) if self.local_ids().iter().all(Option::is_some) => {
                        self.passive_chat.push(m);
                    }
                    _ => {}
                }
                return;
            }
            match &msg {
                GameMsg::SvReadyToEnter(_) => self.enter_game(c, now),
                GameMsg::SvTeamsState(_) | GameMsg::SvTeamsStateLegacy(_) => {
                    let ints = read_ints(&mut u, crate::physics::teams::MAX_CLIENTS);
                    self.predict_msgs.push(PredictMsg::TeamsState(ints));
                }
                GameMsg::SvKillMsg(m) => self.predict_msgs.push(PredictMsg::KillMsg(m.clone())),
                GameMsg::SvKillMsgTeam(m) => {
                    self.predict_msgs.push(PredictMsg::KillMsgTeam(m.clone()));
                }
                GameMsg::SvPreInput(m) => self.predict_msgs.push(PredictMsg::PreInput(m.clone())),
                _ => {}
            }
            self.messages.push(msg);
            return;
        }
        match id {
            g::NETMSG_CAPABILITIES if c == 0 && vital => {
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
            NETMSG_MAP_CHANGE if c == 0 && vital => self.on_map_change(&mut u, now),
            NETMSG_MAP_DATA if c == 0 => self.on_map_data(&mut u, now),
            NETMSG_CON_READY if c == 0 && vital => {
                if self.map.is_none() {
                    return;
                }
                self.on_connected(now);
            }
            NETMSG_CON_READY if c == 1 => {
                self.dummy_connected = true;
                self.dummy_connecting = false;
                self.dummy = true;
                // C++ Rcon("crashmeplx"): on the active connection, now the dummy's
                let mut p = Packer::default();
                p.add_string(b"crashmeplx", 0, true);
                self.send_msg(1, NETMSG_RCON_CMD, true, &p, true, false, now);
            }
            NETMSG_PING => {
                self.send_msg(
                    c,
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
                self.send_msg(c, g::NETMSG_PONGEX, true, &p, vital, true, now);
            }
            NETMSG_INPUTTIMING => {
                let (input_pred_tick, time_left) = (u.get_int(), u.get_int());
                if u.error() {
                    return;
                }
                // adjust our prediction time
                let inputs = &self.links[c].inputs;
                let Some(input) = inputs.iter().find(|i| i.tick == input_pred_tick) else {
                    return;
                };
                let target = input.predicted_time + (now - input.time)
                    - ((time_left as f32 / 1000.0) * FREQ as f32) as i64;
                if target != 0 {
                    self.predicted_time
                        .update(target, time_left, ADJUST_UP, now);
                }
            }
            NETMSG_SNAP | NETMSG_SNAPEMPTY | NETMSG_SNAPSINGLE => {
                self.on_snapshot(c, passive, id, &mut u, now)
            }
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
        // ponytail: no map reload yet (C++ keeps the dummy over NETMSG_MAP_RELOAD)
        if self.dummy_connected {
            self.dummy_disconnect(None, now);
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
        self.send_msg(0, NETMSG_REQUEST_MAP_DATA, true, &p, true, true, now);
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
        self.send_msg(0, NETMSG_READY, true, &Packer::default(), true, true, now);
    }

    /// C++: `CClient::RconAuth` as the `rcon_auth` console command calls it (empty name,
    /// "send rcon commands" 1), on the active connection.
    pub fn rcon_auth(&mut self, password: &str, now: i64) {
        // ponytail: C++ skips this once authed (m_aRconAuthed); we do not track the
        // auth status yet, so a second rcon_auth is sent again
        let mut p = Packer::default();
        p.add_string(b"", 0, true);
        p.add_string(password.as_bytes(), 0, true);
        p.add_int(1);
        self.send_msg(
            self.dummy as usize,
            NETMSG_RCON_AUTH,
            true,
            &p,
            true,
            false,
            now,
        );
    }

    /// C++: `CClient::Rcon` (the `rcon` console command), on the active connection.
    pub fn rcon(&mut self, command: &str, now: i64) {
        let mut p = Packer::default();
        p.add_string(command.as_bytes(), 0, true);
        self.send_msg(
            self.dummy as usize,
            NETMSG_RCON_CMD,
            true,
            &p,
            true,
            false,
            now,
        );
    }

    /// C++: `CGameClient::OnConnected` network part: start info and the `crashmeplx`
    /// rcon line the C++ client always sends.
    fn on_connected(&mut self, now: i64) {
        let p = start_info(&self.player);
        self.send_msg(0, g::MsgClStartInfo::ID, false, &p, true, true, now);

        let mut p = Packer::default();
        p.add_string(b"crashmeplx", 0, true);
        self.send_msg(0, NETMSG_RCON_CMD, true, &p, true, false, now);
    }

    /// C++: `CClient::EnterGame` + `OnEnterGame` (client.cpp:473, 514) for connection `c`.
    fn enter_game(&mut self, c: usize, now: i64) {
        self.send_msg(
            c,
            NETMSG_ENTERGAME,
            true,
            &Packer::default(),
            true,
            true,
            now,
        );
        let l = &mut self.links[c];
        for input in &mut l.inputs {
            input.tick = -1;
        }
        l.current_input = 0;
        l.storage.clear();
        (l.prev_tick, l.cur_tick) = (None, None);
        l.received_snapshots = 0;
        l.snapshot_parts = 0;
        l.incoming_size = 0;
        l.ack_game_tick = -1;
        l.current_recv_tick = 0;
        l.pred_tick = 0;
        l.game_time.init(0, now);
        self.snap_crc_errors = 0;
        self.predicted_time.init(0, now);
        if c == 0 {
            self.last_dummy_connect_time = 0;
        }
        self.server_info_request(); // fresh one for timeout protection
        self.next_ping = now + FREQ / 2;
    }

    /// C++: `CClient::ServerInfoRequest`.
    fn server_info_request(&mut self) {
        self.server_info = ServerInfo::default();
        self.info_request_time = 0;
    }

    /// C++ `CClient::ServerInfo()`, once the server answered.
    pub fn server_info(&self) -> Option<&ServerInfo> {
        (self.info_request_time < 0).then_some(&self.server_info)
    }

    /// C++: the current-server part of `ProcessServerInfo` (client.cpp:1559): only
    /// while online with a map, and a type at least as new as the one we have.
    fn on_connless(&mut self, data: &[u8]) {
        let Some(mut info) = serverinfo::parse(data) else {
            return;
        };
        let Some(map) = &self.map else { return };
        if self.links[0].net.state() == ConnState::Online && info.type_ >= self.server_info.type_ {
            // C++ SetCurrentServerInfo: the map's own name
            info.map = map.name.clone();
            self.server_info = info;
            self.info_request_time = -1;
        }
    }

    /// C++: the `NETMSG_SNAP*` branch of `ProcessServerPacket` (client.cpp:2109) for
    /// connection `c`.
    fn on_snapshot(&mut self, c: usize, passive: bool, msg: i32, u: &mut Unpacker, now: i64) {
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

        let l = &mut self.links[c];
        // check ack tick to see if we already got a snapshot for that tick
        if game_tick < l.current_recv_tick || game_tick <= l.ack_game_tick {
            return;
        }
        if game_tick != l.current_recv_tick {
            l.snapshot_parts = 0;
            l.current_recv_tick = game_tick;
            l.incoming_size = 0;
        }
        let offset = part as usize * MAX_SNAPSHOT_PACKSIZE;
        l.incoming[offset..offset + data.len()].copy_from_slice(data);
        l.snapshot_parts |= 1 << part;
        if part == num_parts - 1 {
            l.incoming_size = (num_parts as usize - 1) * MAX_SNAPSHOT_PACKSIZE + part_size as usize;
        }
        let all = if num_parts == 64 {
            u64::MAX
        } else {
            (1 << num_parts) - 1
        };
        if l.snapshot_parts != all {
            return;
        }
        l.snapshot_parts = 0;

        // find snapshot that we should use as delta
        let empty = Snapshot::default();
        let delta_shot = if delta_tick >= 0 {
            match l.storage.iter().rev().find(|s| s.tick == delta_tick) {
                Some(s) => &s.snap,
                None => {
                    // couldn't find the delta snapshot, force the server to resync
                    l.ack_game_tick = -1;
                    return self.send_input(now);
                }
            }
        } else {
            &empty
        };
        let delta = if l.incoming_size == 0 {
            vec![0, 0, 0] // C++: EmptyDelta()
        } else {
            match varint::decompress(&l.incoming[..l.incoming_size], snapshot::MAX_DELTA_INTS) {
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
                self.links[c].ack_game_tick = -1;
                self.send_input(now);
                self.snap_crc_errors = 0;
            }
            return;
        }
        if self.snap_crc_errors > 0 {
            self.snap_crc_errors -= 1;
        }

        // purge old snapshots
        let l = &mut self.links[c];
        let purge_tick = [l.prev_tick, l.cur_tick]
            .into_iter()
            .flatten()
            .fold(delta_tick, i32::min);
        l.storage.retain(|s| s.tick >= purge_tick);

        let items = snapshot::unpack_and_validate(&snap);
        l.storage.push_back(StoredSnapshot {
            tick: game_tick,
            snap,
            items,
        });
        l.received_snapshots += 1;

        // we got two snapshots until we see us self as connected
        if l.received_snapshots == 2 {
            if !passive {
                // start at 200ms and work from there
                self.predicted_time
                    .init(game_tick as i64 * FREQ / TICK_SPEED, now);
                self.predicted_time.adjust_speed[ADJUST_UP] = 1000.0;
                self.predicted_time.margin = self.prediction_margin_ms() * FREQ / 1000;
            }
            let l = &mut self.links[c];
            l.game_time
                .init((game_tick as i64 - 1) * FREQ / TICK_SPEED, now);
            l.prev_tick = l.storage.front().map(|s| s.tick);
            l.cur_tick = l.storage.back().map(|s| s.tick);
            if !passive {
                self.on_new_snapshot(now);
            }
            self.state = State::Online;
        }
        let l = &mut self.links[c];
        if l.received_snapshots > 2 {
            // adjust game time
            let game_now = l.game_time.get(now);
            let tick_start = game_tick as i64 * FREQ / TICK_SPEED;
            let time_left = (tick_start - game_now) * 1000 / FREQ;
            l.game_time.update(
                (game_tick as i64 - 1) * FREQ / TICK_SPEED,
                time_left as i32,
                ADJUST_DOWN,
                now,
            );
        }
        l.ack_game_tick = game_tick;
    }

    /// C++: `CClient::PredictionMargin`.
    fn prediction_margin_ms(&self) -> i64 {
        if self.caps.sync_weapon_input {
            self.prediction_margin as i64
        } else {
            10
        }
    }

    /// The timing values `OnPredict` and the rendering read at `now`.
    pub fn timing(&self, now: i64) -> Timing {
        let a = self.active();
        Timing {
            game_tick: a.cur_tick.unwrap_or(0),
            prev_game_tick: a.prev_tick.unwrap_or(0),
            pred_tick: a.pred_tick,
            intra_tick: self.intra_tick,
            pred_intra_tick: self.pred_intra_tick,
            now,
            game_now: a.game_time.get(now),
            pred_now: self.predicted_time.get(now),
        }
    }

    /// C++: the `STATE_ONLINE` part of `CClient::Update` (client.cpp:2870).
    fn update_online(&mut self, now: i64) {
        let a = self.dummy as usize;
        let swapped = self.last_dummy != self.dummy;
        // switch dummy snapshot
        let p = &mut self.links[a ^ 1];
        if let Some(mut cur) = p.cur_tick {
            let game_now = p.game_time.get(now);
            while let Some(next) = p.storage.iter().map(|s| s.tick).find(|&t| t > cur) {
                if cur as i64 * FREQ / TICK_SPEED >= game_now {
                    break;
                }
                (p.prev_tick, p.cur_tick) = (Some(cur), Some(next));
                cur = next;
            }
        }
        if let Some(mut cur) = self.links[a].cur_tick {
            let game_now = self.links[a].game_time.get(now);
            let pred_now = self.predicted_time.get(now);
            if swapped && self.links[a].prev_tick.is_some() {
                // load the snapshot of the tee we switched to
                self.on_new_snapshot(now);
            }
            // switch snapshot
            while let Some(next) = (self.links[a].storage.iter())
                .map(|s| s.tick)
                .find(|&t| t > cur)
            {
                if cur as i64 * FREQ / TICK_SPEED >= game_now {
                    break;
                }
                let l = &mut self.links[a];
                (l.prev_tick, l.cur_tick) = (Some(cur), Some(next));
                cur = next;
                self.on_new_snapshot(now);
            }
            if let Some(prev) = self.links[a].prev_tick {
                let t = intra_ticks(prev, cur, game_now, pred_now);
                (self.intra_tick, self.intra_tick_since_prev) = (t.intra, t.since_prev);
                (self.game_tick_time, self.pred_intra_tick) = (t.tick_time, t.pred_intra);
                let new_pred_tick = t.prev_pred_tick + 1;
                if (new_pred_tick - prev).abs() > self.max_latency_ticks() {
                    // prediction time reset
                    self.predicted_time
                        .init(cur as i64 * FREQ / TICK_SPEED + 2 * FREQ / TICK_SPEED, now);
                }
                if new_pred_tick > self.links[a].pred_tick {
                    self.links[a].pred_tick = new_pred_tick;
                    self.repredict = true;
                    self.send_input(now);
                }
            }
            // fetch server info if we don't have it
            if self.info_request_time >= 0 && now > self.info_request_time {
                (self.links[0].net.outbox).push(serverinfo::request(self.info_token));
                self.info_request_time = now + FREQ * 2;
            }
            // periodically ping server
            if self.next_ping >= 0 && now > self.next_ping {
                if self.caps.ping_ex {
                    let mut p = Packer::default();
                    p.add_raw(&random_uuid(self.random));
                    self.send_msg(0, g::NETMSG_PINGEX, true, &p, false, true, now);
                }
                self.next_ping = now + 600 * FREQ; // ping every 10 minutes
            }
        }
        self.last_dummy = self.dummy;
    }

    /// C++: `CClient::MaxLatencyTicks`.
    pub fn max_latency_ticks(&self) -> i32 {
        (TICK_SPEED + self.prediction_margin_ms() * TICK_SPEED / 1000) as i32
    }

    /// C++: `CClient::SendInput` (client.cpp:350): the active tee's input, then the
    /// other tee's while the dummy is connected, both stamped with the active
    /// connection's prediction tick.
    fn send_input(&mut self, now: i64) {
        let a = self.dummy as usize;
        let pred_tick = self.links[a].pred_tick;
        if pred_tick <= 0 {
            return;
        }
        let mut force = false;
        for d in 0..2 {
            if !self.dummy_connected && d != 0 {
                break;
            }
            let i = a ^ d;
            let input = if d == 0 {
                self.snap_input(now)
            } else {
                self.snap_dummy_input(force)
            };
            let Some(input) = input else { continue };
            let ints = input.to_ints();
            let mut p = Packer::default();
            p.add_int(self.links[i].ack_game_tick);
            p.add_int(pred_tick);
            p.add_int(ints.len() as i32 * 4);
            for v in ints {
                p.add_int(v);
            }
            let predicted_time = self.predicted_time.get(now);
            let l = &mut self.links[i];
            l.inputs[l.current_input] = SentInput {
                tick: pred_tick,
                data: input,
                predicted_time,
                time: now,
            };
            l.current_input = (l.current_input + 1) % NUM_INPUTS;
            let odd = l.current_input % 2 == 1;
            self.send_msg(i, NETMSG_INPUT, true, &p, false, true, now);
            // ugly workaround for dummy. we need to send input with dummy to prevent
            // prediction time resets. but if we do it too often, then it's impossible
            // to use grenade with frozen dummy that gets hammered...
            if self.dummy_copy_moves || odd {
                force = true;
            }
        }
    }

    /// The sending rules of C++ `CControls::SnapInput` (controls.cpp:241) for the
    /// active tee, playing or in a menu; `input` (incl. the player flags) is what
    /// SnapInput produced. `None`: nothing to send.
    fn snap_input(&mut self, now: i64) -> Option<ObjPlayerInput> {
        let mut input = self.input;
        let last = self.last_input[self.dummy as usize];
        let mut send = last.player_flags != input.player_flags;
        if input.player_flags & g::PLAYERFLAG_PLAYING == 0 {
            // in a menu: send once a second just to be sure
            send = send || now > self.last_input_send + FREQ;
        } else {
            if input.target_x == 0 && input.target_y == 0 {
                input.target_x = 1;
            }
            let ninja = self
                .local_character()
                .is_some_and(|c| c.weapon == g::WEAPON_NINJA)
                && (input.direction != 0 || input.jump != 0 || input.hook != 0);
            send = send
                || last.direction != input.direction
                || last.jump != input.jump
                || last.fire != input.fire
                || last.hook != input.hook
                || last.wanted_weapon != input.wanted_weapon
                || last.next_weapon != input.next_weapon
                || last.prev_weapon != input.prev_weapon
                || now > self.last_input_send + FREQ / 25 // send at least 25 Hz
                || ninja;
        }
        self.last_input[self.dummy as usize] = input;
        if !send {
            return None;
        }
        self.last_input_send = now;
        Some(input)
    }

    /// C++: `CGameClient::OnSnapInput` for the other tee, once its id is known.
    fn snap_dummy_input(&mut self, force: bool) -> Option<ObjPlayerInput> {
        self.links[!self.dummy as usize].local_id?;
        let (hammer, restore) = (self.dummy_hammer, self.dummy_restore_weapon);
        self.dummy_snap
            .snap(force, hammer, restore, self.hammer_dir)
    }

    /// C++: the messages `CGameClient::OnNewSnapshot` sends (gameclient.cpp:2255), and
    /// `m_aLocalIds[cl_dummy]`.
    fn on_new_snapshot(&mut self, now: i64) {
        let a = self.dummy as usize;
        if let Some(cur) = self.links[a].cur_tick {
            self.snapshot_switches.push((self.links[a].prev_tick, cur));
            self.repredict = true;
        }
        let local_id = self.snapshot().iter().find_map(|it| {
            (it.type_id == g::NETOBJTYPE_PLAYERINFO)
                .then(|| g::ObjPlayerInfo::from_ints(&it.data))
                .filter(|p| p.local != 0)
                .map(|p| p.client_id)
        });
        self.snap_local_id = local_id;
        // C++ only writes m_aLocalIds when there is local info
        if local_id.is_some() {
            self.links[a].local_id = local_id;
        }
        for i in 0..2 {
            if self.links[i].ddrace_msg_sent || local_id.is_none() {
                continue;
            }
            if i == 1 && !self.dummy_connected {
                continue;
            }
            let mut p = Packer::default();
            p.add_int(DDNET_VERSION);
            self.send_msg(
                i,
                g::NETMSGTYPE_CL_ISDDNETLEGACY,
                false,
                &p,
                true,
                false,
                now,
            );
            self.links[i].ddrace_msg_sent = true;
        }
        if self.links[a].show_others != SHOW_OTHERS_DEFAULT {
            let mut p = Packer::default();
            g::MsgClShowOthers {
                show: SHOW_OTHERS_DEFAULT,
            }
            .pack(&mut p);
            self.send_active(g::MsgClShowOthers::ID, &p, now);
            self.links[a].show_others = SHOW_OTHERS_DEFAULT;
        }
        for i in 0..2 {
            if i == 1 && !self.dummy_connected {
                continue;
            }
            if self.links[i].spectator_count != SHOW_SPECTATOR_COUNT_DEFAULT {
                let mut p = Packer::default();
                g::MsgClEnableSpectatorCount {
                    enable: SHOW_SPECTATOR_COUNT_DEFAULT,
                }
                .pack(&mut p);
                let id = g::MsgClEnableSpectatorCount::ID;
                self.send_msg(i, id, false, &p, true, false, now);
                self.links[i].spectator_count = SHOW_SPECTATOR_COUNT_DEFAULT;
            }
        }
        self.send_camera(now);
    }

    /// C++: `Cl_ShowDistance` and `Cl_CameraInfo` in `CGameClient::OnNewSnapshot`
    /// (gameclient.cpp:2313-2397), sent when they change, and once to a newly
    /// connected dummy.
    fn send_camera(&mut self, now: i64) {
        let cam = self.camera;
        let mut show_distance_zoom = cam.zoom;
        let mut zoom = cam.zoom;
        if cam.zooming {
            if cam.zoom_target > cam.zoom {
                show_distance_zoom = cam.zoom_target; // zooming out
            } else if cam.zoom_target < cam.zoom && self.last_show_distance_zoom > 0.0 {
                show_distance_zoom = self.last_show_distance_zoom; // zooming in
            }
            zoom = cam.zoom_target;
        }
        let mut deadzone = cam.deadzone as f32;
        let mut follow_factor = cam.follow_factor as f32;
        if spectating(self.snapshot(), self.local_id()) {
            // don't send camera information when spectating
            zoom = self.last_zoom;
            deadzone = self.last_deadzone;
            follow_factor = self.last_follow_factor;
        }
        let mut show_distance = Packer::default();
        let (x, y) = calc_screen_params(cam.aspect, show_distance_zoom);
        g::MsgClShowDistance {
            x: trunc_i32(x),
            y: trunc_i32(y),
        }
        .pack(&mut show_distance);
        let mut camera_info = Packer::default();
        g::MsgClCameraInfo {
            zoom: trunc_i32(zoom * 1000.0), // C++ round_truncate
            deadzone: trunc_i32(deadzone),
            follow_factor: trunc_i32(follow_factor),
        }
        .pack(&mut camera_info);
        let (show_id, camera_id) = (g::MsgClShowDistance::ID, g::MsgClCameraInfo::ID);
        // initialize dummy vital when first connected
        if self.dummy_connected && !self.last_dummy_connected {
            self.send_msg(1, show_id, false, &show_distance, true, false, now);
            self.send_msg(1, camera_id, false, &camera_info, true, false, now);
        }
        let to_dummy = self.dummy_connected && self.last_dummy_connected;
        if show_distance_zoom != self.last_show_distance_zoom
            || cam.aspect != self.last_screen_aspect
        {
            self.send_msg(0, show_id, false, &show_distance, true, false, now);
            if to_dummy {
                self.send_msg(1, show_id, false, &show_distance, true, false, now);
            }
        }
        if zoom != self.last_zoom
            || deadzone != self.last_deadzone
            || follow_factor != self.last_follow_factor
        {
            self.send_msg(0, camera_id, false, &camera_info, true, false, now);
            if to_dummy {
                self.send_msg(1, camera_id, false, &camera_info, true, false, now);
            }
        }
        self.last_show_distance_zoom = show_distance_zoom;
        self.last_zoom = zoom;
        self.last_screen_aspect = cam.aspect;
        self.last_deadzone = deadzone;
        self.last_follow_factor = follow_factor;
        self.last_dummy_connected = self.dummy_connected;
    }
}

/// C++: the other tee's input state of `CGameClient` (`m_DummyInput`, `m_HammerInput`,
/// `m_DummyFire`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DummySnap {
    pub input: ObjPlayerInput,
    pub hammer_input: ObjPlayerInput,
    pub fire: i32,
}

impl DummySnap {
    /// C++: `CGameClient::OnSnapInput` with `Dummy` (gameclient.cpp:527) once
    /// `m_aLocalIds[!cl_dummy]` is known: `input`, or with `cl_dummy_hammer` every 25th
    /// call a hammer hit towards `hammer_dir`. `None`: nothing to send.
    pub fn snap(
        &mut self,
        force: bool,
        hammer: bool,
        restore_weapon: bool,
        hammer_dir: (f32, f32),
    ) -> Option<ObjPlayerInput> {
        if !hammer {
            if self.fire != 0 {
                self.input.fire = self.hammer_input.fire.wrapping_add(1) & !1;
                self.fire = 0;
            }
            let d = &self.input;
            if !force && d.direction == 0 && d.jump == 0 && d.hook == 0 {
                return None;
            }
            return Some(self.input);
        }
        if self.fire % 25 != 0 {
            self.fire += 1;
            return None;
        }
        self.fire += 1;
        self.hammer_input.fire = self.hammer_input.fire.wrapping_add(1) | 1;
        self.hammer_input.wanted_weapon = g::WEAPON_HAMMER + 1;
        if !restore_weapon {
            self.input.wanted_weapon = g::WEAPON_HAMMER + 1;
        }
        self.hammer_input.target_x = trunc_i32(hammer_dir.0);
        self.hammer_input.target_y = trunc_i32(hammer_dir.1);
        Some(self.hammer_input)
    }
}

/// `Cl_StartInfo` for a player (C++ `SendInfo(true)`, `SendDummyInfo(true)`).
fn start_info(i: &PlayerInfo) -> Packer {
    let mut p = Packer::default();
    g::MsgClStartInfo {
        name: i.name.clone(),
        clan: i.clan.clone(),
        country: i.country,
        skin: i.skin.clone(),
        use_custom_color: i.use_custom_color,
        color_body: i.color_body,
        color_feet: i.color_feet,
    }
    .pack(&mut p);
    p
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
