//! Client prediction driven by server snapshots. C++: the parts of
//! `CGameClient::OnNewSnapshot`, `UpdatePrediction` and `OnPredict` (gameclient.cpp)
//! that move characters, with the default client config (`cl_predict 1`,
//! `cl_predict_freeze 1`, `cl_antiping 0`). Weapons are not predicted.
//!
//! `rendered` ports `UpdateRenderedCharacters`: what the renderer draws between ticks.
//!
//! Also the game messages that change the prediction world (`Sv_TuneParams` with
//! `UpdateLocalTuning` and the map's tune settings, `Sv_TeamsState`, `Sv_KillMsg`,
//! `Sv_KillMsgTeam`, `Sv_PreInput`).
//!
//! Not ported yet: snapshot entities other than doors, the dummy, antiping smoothing and
//! events.

use std::collections::BTreeMap;

use std::rc::Rc;

use crate::console;
use crate::map::Map;
use crate::net::client::{Client, PredictMsg};
use crate::physics::collision::Collision;
use crate::physics::core::{CharacterCore, WorldCore};
use crate::physics::teams::{MAX_CLIENTS, NUM_DDRACE_TEAMS, TEAM_FLOCK, TeamsCore};
use crate::physics::tuning::{Param, Tuning};
use crate::physics::vmath::{Vec2, mix, vec2};
use crate::physics::world::{
    GameWorld, NUM_TUNE_ZONES, TUNE_ZONE_OVERRIDE_NONE, WorldConfig, core_obj,
};
use crate::protocol::generated::{
    self as g, MsgSvPreInput, ObjCharacter, ObjCharacterCore, ObjDDNetCharacter, ObjDDNetLaser,
    ObjGameInfoEx, ObjSpectatorInfo, ObjSwitchState,
};
use crate::protocol::snapshot::Item;

const TICK_SPEED: i32 = 50;
const TILE_SWITCHTIMEDOPEN: i32 = 22;
const TILE_SWITCHTIMEDCLOSE: i32 = 23;
const TILE_SWITCHOPEN: i32 = 24;
const TILE_SWITCHCLOSE: i32 = 25;
/// C++: `MAPITEMTYPE_INFO`.
const MAPITEMTYPE_INFO: i32 = 1;
/// C++: `VANILLA_MAX_CLIENTS`.
const VANILLA_MAX_CLIENTS: usize = 16;
/// C++: the size of `CClientData::m_aPreInputs`.
const NUM_PRE_INPUTS: usize = 200;

/// C++: `CGameClient::LoadMapSettings` (gameclient.cpp:4208): every tune zone reset, then
/// the map's settings run as console lines; only the commands registered with
/// `CFGFLAG_GAME` run: `tune` and `tune_zone`. ponytail: `mapbug` (CMapBugs) is not
/// ported; add it with the map bug emulation.
pub fn map_tunings(map: &Map) -> Vec<Tuning> {
    let mut list = vec![Tuning::map_default(); NUM_TUNE_ZONES];
    let df = &map.datafile;
    for i in df.get_type(MAPITEMTYPE_INFO) {
        let item = &df.items[i];
        if item.id() != 0 {
            continue;
        }
        // sizeof(CMapItemInfoSettings): version, author, map version, credits, license,
        // settings
        if item.data.len() < 6 || item.data[5] <= -1 {
            break;
        }
        apply_map_settings(&mut list, &map.data(item.data[5]).unwrap_or_default());
        break;
    }
    list
}

/// The settings loop of `LoadMapSettings`: runs the NUL-terminated lines of a map's
/// settings data as console lines from the map (`IConsole::CLIENT_ID_GAME`).
pub fn apply_map_settings(list: &mut [Tuning], settings: &[u8]) {
    for line in settings.split(|&b| b == 0) {
        for (command, args) in console::split_line(&String::from_utf8_lossy(line)) {
            let mut a = Vec::new();
            if command.eq_ignore_ascii_case("tune") {
                // C++: ConTuneParam
                if console::parse_args(&args, "s[tuning] ?f[value]", &mut a) && a.len() == 2 {
                    list[0].set_by_name(&a[0], console::arg_float(&a, 1));
                }
            } else if command.eq_ignore_ascii_case("tune_zone")
                && console::parse_args(&args, "i[zone] s[tuning] f[value]", &mut a)
            {
                // C++: ConTuneZone
                let zone = console::str_toint(&a[0]).unwrap_or(0);
                if (0..NUM_TUNE_ZONES as i32).contains(&zone) {
                    list[zone as usize].set_by_name(&a[1], console::arg_float(&a, 2));
                }
            }
        }
    }
}

/// C++: `m_Snap.m_aCharacters[i]` of an active character (in this and the previous
/// snapshot): `prev` evolved to the previous snapshot's tick, `cur` to the current one,
/// and the DDNet objects of both snapshots.
#[derive(Clone, Copy, Debug)]
struct SnapChar {
    prev: ObjCharacter,
    cur: ObjCharacter,
    ext: Option<ObjDDNetCharacter>,
    prev_ext: Option<ObjDDNetCharacter>,
}

/// A character as the renderer sees it this frame. C++: `m_aClients[i]`'s `m_RenderPrev`,
/// `m_RenderCur`, `m_RenderPos` and `m_IsPredicted` after `UpdateRenderedCharacters`,
/// plus the snapshots' DDNet objects.
#[derive(Clone, Copy, Debug)]
pub struct Rendered {
    pub id: i32,
    pub prev: ObjCharacter,
    pub cur: ObjCharacter,
    pub pos: Vec2,
    /// Drawn between the predicted ticks with `PredIntraGameTick` (the local tee).
    pub predicted: bool,
    pub ext: Option<ObjDDNetCharacter>,
    pub prev_ext: Option<ObjDDNetCharacter>,
}

/// How well prediction matched the server.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    /// Predictions of a tick that was already predicted before (C++ `OnPredict` with
    /// `m_PredictedTick == PredGameTick`).
    pub repredictions: u32,
    /// C++: the "prediction error" debug log: such a re-prediction gave a different local
    /// character core than the prediction before it.
    pub prediction_errors: u32,
    /// Snapshots whose local character was compared with the prediction of its tick.
    pub compared: u32,
    /// Compared snapshots whose (evolved) local character core differed.
    pub mismatched: u32,
    /// The last mismatch: (tick, predicted, server).
    pub last_mismatch: Option<(i32, ObjCharacterCore, ObjCharacterCore)>,
}

/// C++: `CGameClient`'s `m_GameWorld` (server state); `m_PredictedWorld` is rebuilt
/// from it by `predict`.
pub struct Predictor {
    col: Rc<Collision>,
    world: GameWorld,
    teams: TeamsCore,
    highest_switch: i32,
    /// C++: `CCharOrder::m_Ids`.
    char_order: Vec<usize>,
    chars: Vec<Option<SnapChar>>,
    local_id: i32,
    /// C++: `m_PredictedChar` written as a net object, and `m_PredictedTick`.
    predicted_char: Option<ObjCharacterCore>,
    /// C++: `m_aClients[local].m_PrevPredicted` and `m_Predicted`: the local core at
    /// the start and the end of the last predicted tick.
    prev_predicted_core: Option<CharacterCore>,
    predicted_core: Option<CharacterCore>,
    /// C++: `Predict()`: a local character to predict, not spectating, world not paused.
    predicting: bool,
    /// C++: `m_aSwitchStateTeam`: the team of the only switch state object, else -1.
    switch_state_team: i32,
    predicted_tick: i32,
    /// C++ `m_Snap.m_SpecInfo.m_Active`, `IsWorldPaused()` and `m_GameInfo.m_AllowZoom`
    /// of the current snapshot.
    pub spectating: bool,
    pub world_paused: bool,
    pub allow_zoom: bool,
    /// C++ `m_GameInfo.m_HudDDRace`: the server wants the DDRace HUD (GameInfoEx 7+).
    pub hud_ddrace: bool,
    /// The latest prediction of the local character for each tick.
    history: BTreeMap<i32, ObjCharacterCore>,
    pub stats: Stats,
    /// C++: `m_aTuning[0]` (the last `Sv_TuneParams`), `m_aReceivedTuning`,
    /// `m_aLocalTuneZone`, `m_aExpectingTuningForZone`, `m_aExpectingTuningSince`.
    tuning: Tuning,
    received_tuning: bool,
    local_tune_zone: i32,
    expecting_tuning_for_zone: i32,
    expecting_tuning_since: i32,
    /// C++: `m_aClients[i].m_aPreInputs`, indexed by intended tick % 200.
    pre_inputs: Vec<Vec<MsgSvPreInput>>,
}

/// C++: `CCharacterCore::Write` into a `CNetObj_Character` (all but the tick).
fn write_core(c: &mut ObjCharacter, core: &CharacterCore) {
    let o = core.write();
    (c.x, c.y, c.vel_x, c.vel_y, c.angle, c.direction, c.jumped) =
        (o.x, o.y, o.vel_x, o.vel_y, o.angle, o.direction, o.jumped);
    (c.hooked_player, c.hook_state, c.hook_tick) = (o.hooked_player, o.hook_state, o.hook_tick);
    (c.hook_x, c.hook_y, c.hook_dx, c.hook_dy) = (o.hook_x, o.hook_y, o.hook_dx, o.hook_dy);
}

/// Pads a snapshot item to `n` ints (shorter items come from older servers).
fn ints(item: &Item, n: usize) -> Vec<i32> {
    crate::items::pad(&item.data, n)
}

fn find(items: &[Item], type_id: i32, id: i32) -> Option<&Item> {
    items.iter().find(|it| it.type_id == type_id && it.id == id)
}

impl Predictor {
    /// C++: `CGameClient::OnConnected` for a loaded map (incl. `LoadMapSettings`).
    pub fn new(col: Rc<Collision>, map: &Map) -> Self {
        // C++: CGameClient::OnConnected / OnReset defaults
        let config = WorldConfig {
            infinite_ammo: true,
            predict_freeze: true,
            freeze_delay: 3,
            antiping_players: 1,
            ..Default::default()
        };
        let mut world = GameWorld::new(Rc::clone(&col), config);
        world.tunings = map_tunings(map);
        let no_pre_input = MsgSvPreInput {
            intended_tick: -1,
            ..Default::default()
        };
        Self {
            highest_switch: col.highest_switch_number,
            world,
            teams: TeamsCore::default(),
            col,
            char_order: (0..MAX_CLIENTS).collect(),
            chars: vec![None; MAX_CLIENTS],
            local_id: -1,
            predicted_char: None,
            prev_predicted_core: None,
            predicted_core: None,
            predicting: false,
            switch_state_team: -1,
            predicted_tick: -1,
            spectating: false,
            world_paused: false,
            hud_ddrace: false,
            allow_zoom: false,
            history: BTreeMap::new(),
            stats: Stats::default(),
            tuning: Tuning::default(),
            received_tuning: false,
            local_tune_zone: -1,
            expecting_tuning_for_zone: -1,
            expecting_tuning_since: 0,
            pre_inputs: vec![vec![no_pre_input; NUM_PRE_INPUTS]; MAX_CLIENTS],
        }
    }

    /// Handles everything the client did since the last call: each snapshot switch
    /// (`OnNewSnapshot` + `UpdatePrediction`), then `OnPredict` if a repredict is due.
    pub fn update(&mut self, client: &mut Client) {
        // C++ handles messages while pumping the network, before CClient::Update
        // switches snapshots
        for msg in std::mem::take(&mut client.predict_msgs) {
            self.on_message(msg);
        }
        for (prev, cur) in std::mem::take(&mut client.snapshot_switches) {
            self.on_new_snapshot(client, prev, cur);
        }
        if std::mem::take(&mut client.repredict) {
            self.predict(client);
        }
    }

    /// C++: the branches of `CGameClient::OnMessage` that change the prediction world.
    pub fn on_message(&mut self, msg: PredictMsg) {
        match msg {
            // gameclient.cpp:1053
            PredictMsg::TuneParams(ints) => {
                self.received_tuning = true;
                self.tuning = Tuning::from_net(&ints);
            }
            // gameclient.cpp:1168
            PredictMsg::TeamsState(ints) => {
                let mut i = 0;
                while i < MAX_CLIENTS {
                    match ints.get(i) {
                        Some(&team) if (TEAM_FLOCK..NUM_DDRACE_TEAMS as i32).contains(&team) => {
                            self.teams.set_team(i as i32, team);
                        }
                        _ => {
                            self.teams.set_team(i as i32, 0);
                            break;
                        }
                    }
                    i += 1;
                }
                if i <= VANILLA_MAX_CLIENTS {
                    self.teams.num_ddrace_teams = VANILLA_MAX_CLIENTS as i32 + 1;
                }
            }
            // gameclient.cpp:1192
            PredictMsg::KillMsg(m) => {
                if !(self.world.config.is_fng && m.weapon == g::WEAPON_LASER) {
                    self.give_weak(m.victim);
                    if let Ok(victim) = usize::try_from(m.victim) {
                        self.world.reset_prediction(victim);
                    }
                    self.world.release_hooked(m.victim);
                }
            }
            // gameclient.cpp:1204
            PredictMsg::KillMsgTeam(m) => {
                let mut sorted = Vec::new();
                for i in 0..MAX_CLIENTS {
                    if self.teams.team(i as i32) == m.team {
                        if let Some((ch, _)) = self.world.character(i) {
                            let key = if m.first == i as i32 {
                                MAX_CLIENTS as i32
                            } else {
                                ch.strong_weak_id
                            };
                            self.world.reset_prediction(i);
                            sorted.push((i as i32, key));
                        }
                        self.world.release_hooked(i as i32);
                    }
                }
                sorted.sort_by_key(|a| std::cmp::Reverse(a.1)); // stable, like std::stable_sort
                for (id, _) in sorted {
                    self.give_weak(id);
                }
            }
            // gameclient.cpp:1236; ponytail: C++ indexes with a negative remainder for a
            // negative intended tick (out of bounds); such pre-inputs are dropped
            PredictMsg::PreInput(m) => {
                if let (Ok(owner), Ok(slot)) = (
                    usize::try_from(m.owner),
                    usize::try_from(m.intended_tick % NUM_PRE_INPUTS as i32),
                ) && owner < MAX_CLIENTS
                {
                    self.pre_inputs[owner][slot] = m;
                }
            }
        }
    }

    /// C++: `CCharOrder::GiveWeak`.
    fn give_weak(&mut self, id: i32) {
        if (0..MAX_CLIENTS as i32).contains(&id) {
            self.char_order.retain(|&i| i != id as usize);
            self.char_order.push(id as usize);
        }
    }

    /// C++: `m_aClients[id].m_aPreInputs[tick % 200]` if it was meant for `tick`
    /// (the test in `ApplyPreInputs`).
    pub fn pre_input(&self, id: usize, tick: i32) -> Option<&MsgSvPreInput> {
        let p = self
            .pre_inputs
            .get(id)?
            .get(usize::try_from(tick % NUM_PRE_INPUTS as i32).ok()?)?;
        (p.intended_tick == tick).then_some(p)
    }

    /// C++: `CGameClient::GetTuning(zone)`: the tuning list the prediction uses.
    pub fn tuning(&self, zone: i32) -> &Tuning {
        &self.world.tunings[zone.clamp(0, NUM_TUNE_ZONES as i32 - 1) as usize]
    }

    /// C++: `m_aTuning[0]`: the last tuning the server sent (default before any).
    pub fn received_tuning(&self) -> &Tuning {
        &self.tuning
    }

    /// C++: `m_aReceivedTuning[0]`, `m_aLocalTuneZone[0]`, `m_aExpectingTuningForZone[0]`
    /// and `m_aExpectingTuningSince[0]` (for the parity tests).
    pub fn tune_state(&self) -> (bool, i32, i32, i32) {
        (
            self.received_tuning,
            self.local_tune_zone,
            self.expecting_tuning_for_zone,
            self.expecting_tuning_since,
        )
    }

    /// C++: `m_GameWorld`, the server-state world (for the parity tests).
    pub fn world_mut(&mut self) -> &mut GameWorld {
        &mut self.world
    }

    /// C++: `m_CharOrder.m_Ids` (for the parity tests).
    pub fn char_order(&self) -> &[usize] {
        &self.char_order
    }

    /// C++: `m_Teams` (for the parity tests).
    pub fn teams_mut(&mut self) -> &mut TeamsCore {
        &mut self.teams
    }

    /// C++: `CGameClient::UpdateLocalTuning` (gameclient.cpp:3409). `local_pos` is the
    /// local character (`m_pLocalCharacter`), else the spectator info position.
    pub fn update_local_tuning(&mut self, local_pos: Option<Vec2>, override_zone: Option<i32>) {
        let use_zones = self.world.config.use_tune_zones;
        // always update default tune zone, even without character
        if !use_zones {
            self.world.tunings[0] = self.tuning;
        }
        let Some(pos) = local_pos else {
            return;
        };
        if !use_zones {
            return;
        }
        let zone = match override_zone {
            Some(z) if z != TUNE_ZONE_OVERRIDE_NONE => z,
            _ => self.col.is_tune(self.col.get_map_index(pos)),
        };
        if zone != self.local_tune_zone {
            // our tunezone changed, expecting tuning message
            self.local_tune_zone = zone;
            self.expecting_tuning_for_zone = zone;
            self.expecting_tuning_since = 0;
        }
        if self.expecting_tuning_for_zone >= 0 {
            if self.received_tuning {
                self.world.tunings[self.expecting_tuning_for_zone as usize] = self.tuning;
                self.received_tuning = false;
                self.expecting_tuning_for_zone = -1;
            } else if self.expecting_tuning_since >= 5 {
                // probably dropped, out of order or for another zone: fall back
                self.expecting_tuning_for_zone = -1;
                self.expecting_tuning_since = 0;
                self.received_tuning = false;
            } else {
                self.expecting_tuning_since += 1;
            }
        } else {
            self.world.tunings[zone as usize] = self.tuning;
            self.expecting_tuning_since = 0;
            self.received_tuning = false;
        }
    }

    /// C++: `CGameClient::Evolve`: ticks a snapped character core without input from its
    /// tick (the server's dead-reckoning tick) to `tick`.
    fn evolve(&self, c: &mut ObjCharacter, tick: i32) {
        let (mut w, teams) = (WorldCore::default(), TeamsCore::default());
        w.characters[0] = Some(CharacterCore::new());
        w.read(0, &core_obj(c));
        w.characters[0].as_mut().unwrap().active_weapon = c.weapon;
        while c.tick < tick {
            c.tick += 1;
            w.tick(0, &self.col, &teams, false, true);
            w.move_(0, &self.col, &teams);
            w.quantize(0);
        }
        write_core(c, w.characters[0].as_ref().unwrap());
    }

    /// C++: the prediction parts of `CGameClient::OnNewSnapshot`.
    fn on_new_snapshot(&mut self, client: &Client, prev: Option<i32>, tick: i32) {
        let Some(items) = client.snapshot_at(tick) else {
            return;
        };
        let prev_items = prev.and_then(|t| client.snapshot_at(t)).unwrap_or(&[]);

        self.local_id = items
            .iter()
            .filter(|it| it.type_id == g::NETOBJTYPE_PLAYERINFO)
            .map(|it| g::ObjPlayerInfo::from_ints(&ints(it, g::ObjPlayerInfo::NUM_INTS)))
            .find(|p| p.local != 0)
            .map_or(-1, |p| p.client_id);

        self.chars = vec![None; MAX_CLIENTS];
        let prev_tick = prev.unwrap_or(0);
        let ext_of = |items: &[Item], id: i32| {
            find(items, g::NETOBJTYPE_DDNETCHARACTER, id)
                .map(|e| ObjDDNetCharacter::from_ints(&ints(e, ObjDDNetCharacter::NUM_INTS)))
        };
        for it in items
            .iter()
            .filter(|it| it.type_id == g::NETOBJTYPE_CHARACTER)
        {
            let id = it.id as usize;
            let Some(old) = find(prev_items, g::NETOBJTYPE_CHARACTER, it.id) else {
                continue;
            };
            if id >= MAX_CLIENTS {
                continue;
            }
            let mut cur = ObjCharacter::from_ints(&ints(it, ObjCharacter::NUM_INTS));
            let mut prev = ObjCharacter::from_ints(&ints(old, ObjCharacter::NUM_INTS));
            // limit evolving to 3 seconds
            // ponytail: C++ reuses its previous evolve when the snapped object did not
            // change; evolving again from the snapped tick gives the same core.
            if prev_tick - prev.tick <= 3 * TICK_SPEED && prev.tick != 0 {
                self.evolve(&mut prev, prev_tick);
            }
            if tick - cur.tick <= 3 * TICK_SPEED && cur.tick != 0 {
                self.evolve(&mut cur, tick);
            }
            let ext = ext_of(items, it.id);
            if let Some(ext) = ext {
                self.teams
                    .set_solo(it.id, ext.flags & g::CHARACTERFLAG_SOLO != 0);
            }
            let prev_ext = ext_of(prev_items, it.id);
            self.chars[id] = Some(SnapChar {
                prev,
                cur,
                ext,
                prev_ext,
            });
        }
        // C++: m_SpecInfo.m_Active and IsWorldPaused
        let spectating = crate::net::client::spectating(items, Some(self.local_id));
        let paused = items
            .iter()
            .find(|it| it.type_id == g::NETOBJTYPE_GAMEINFO)
            .is_some_and(|it| ints(it, 2)[1] & g::GAMESTATEFLAG_PAUSED != 0);
        self.predicting = self.local_char().is_some() && !spectating && !paused;
        (self.spectating, self.world_paused) = (spectating, paused);

        // game info: C++ GetGameInfo, only what prediction reads
        let info = items
            .iter()
            .find(|it| it.type_id == g::NETOBJTYPE_GAMEINFOEX);
        let size = info.map_or(0, |it| it.data.len() * 4);
        let ex = info.map(|it| ObjGameInfoEx::from_ints(&ints(it, ObjGameInfoEx::NUM_INTS)));
        let version = match (size, &ex) {
            (12.., Some(ex)) => ex.version,
            (8.., Some(ex)) => ex.version.min(4),
            (4.., _) => 0,
            _ => -1, // ponytail: no GameInfoEx: C++ guesses from the game type string
        };
        let ex = ex.unwrap_or_default();
        let (flags, flags2) = (
            if version >= 0 { ex.flags } else { 0 },
            if version >= 5 { ex.flags2 } else { 0 },
        );
        let ddrace = flags & (g::GAMEINFOFLAG_GAMETYPE_DDRACE | g::GAMEINFOFLAG_GAMETYPE_DDNET)
            != 0
            || (version >= 6 && flags2 & g::GAMEINFOFLAG2_GAMETYPE_FDDRACE != 0);
        let block_worlds = flags & g::GAMEINFOFLAG_GAMETYPE_BLOCK_WORLDS != 0;
        let (mut predict_ddrace, mut predict_tiles, mut bug_input) =
            (ddrace, ddrace && !block_worlds, ddrace);
        if version >= 2 {
            predict_ddrace = flags & g::GAMEINFOFLAG_PREDICT_DDRACE != 0;
            predict_tiles = flags & g::GAMEINFOFLAG_PREDICT_DDRACE_TILES != 0;
            bug_input = flags & g::GAMEINFOFLAG_BUG_DDRACE_INPUT != 0;
        }
        let fng = flags & g::GAMEINFOFLAG_GAMETYPE_FNG != 0;
        let predict_fng = if version >= 2 {
            flags & g::GAMEINFOFLAG_PREDICT_FNG != 0
        } else {
            fng
        };
        let no_weak = version >= 8 && flags2 & g::GAMEINFOFLAG2_NO_WEAK_HOOK != 0;
        self.hud_ddrace = version >= 7 && flags2 & g::GAMEINFOFLAG2_HUD_DDRACE != 0;
        let race = ddrace
            || flags & (g::GAMEINFOFLAG_GAMETYPE_RACE | g::GAMEINFOFLAG_GAMETYPE_FASTCAP) != 0;
        let city = version >= 5 && flags2 & g::GAMEINFOFLAG2_GAMETYPE_CITY != 0;
        self.allow_zoom = if version >= 2 {
            flags & g::GAMEINFOFLAG_ALLOW_ZOOM != 0
        } else {
            race || block_worlds || city
        };
        self.teams.num_ddrace_teams = if version >= 12 {
            let n = ex.num_dd_race_teams;
            if n > TEAM_FLOCK + 1 && n <= NUM_DDRACE_TEAMS as i32 {
                n
            } else {
                NUM_DDRACE_TEAMS as i32
            }
        } else {
            65
        };

        self.switch_state_team = -1;
        let mut got_switch_state_team = false;
        for it in items
            .iter()
            .filter(|it| it.type_id == g::NETOBJTYPE_SWITCHSTATE)
        {
            if it.data.len() < 9 {
                continue;
            }
            self.read_switch_state(it);
            self.switch_state_team = if got_switch_state_team {
                -1
            } else {
                it.id.clamp(TEAM_FLOCK, NUM_DDRACE_TEAMS as i32 - 1)
            };
            got_switch_state_team = true;
        }

        let c = &mut self.world.config;
        c.predict_ddrace = predict_ddrace;
        c.predict_tiles = predict_ddrace && predict_tiles;
        c.bug_ddrace_input = bug_input;
        c.no_weak_hook_and_bounce = no_weak;
        c.is_fng = predict_fng;
        c.use_tune_zones = predict_tiles; // C++: UpdateLocalTuning

        // C++: m_Snap.m_pLocalCharacter (the evolved snapshot character, not while
        // spectating), else the last spectator info
        let local_pos = match self.local_char() {
            Some(c) if !spectating => Some(vec2(c.cur.x as f32, c.cur.y as f32)),
            _ => items
                .iter()
                .rev()
                .find(|it| it.type_id == g::NETOBJTYPE_SPECTATORINFO)
                .map(|it| ObjSpectatorInfo::from_ints(&ints(it, ObjSpectatorInfo::NUM_INTS)))
                .map(|s| vec2(s.x as f32, s.y as f32)),
        };
        let override_zone = ext_of(items, self.local_id).map(|e| e.tune_zone_override);
        self.update_local_tuning(local_pos, override_zone);

        self.compare_with_server(tick);
        self.update_prediction(client, tick);
    }

    /// C++: the `NETOBJTYPE_SWITCHSTATE` part of `OnNewSnapshot`.
    fn read_switch_state(&mut self, it: &Item) {
        let s = ObjSwitchState::from_ints(&ints(it, ObjSwitchState::NUM_INTS));
        let team = it.id.clamp(TEAM_FLOCK, NUM_DDRACE_TEAMS as i32 - 1) as usize;
        let highest = s
            .highest_switch_number
            .max(self.highest_switch)
            .clamp(0, 255);
        let switchers = &mut self.world.core.switchers;
        if highest != (switchers.len() as i32 - 1).max(0) {
            self.world.core.init_switchers(highest);
            self.highest_switch = highest;
        }
        let switchers = &mut self.world.core.switchers;
        for (j, sw) in switchers.iter_mut().enumerate() {
            sw.status[team] = (s.status[j / 32] >> (j % 32)) & 1 != 0;
        }
        if it.data.len() >= 17 {
            // the end ticks of up to four timed switchers
            for (&number, &end_tick) in s.switch_numbers.iter().zip(&s.end_ticks) {
                if end_tick > 0 && number >= 0 && (number as usize) < switchers.len() {
                    switchers[number as usize].end_tick[team] = end_tick;
                }
            }
        }
        for sw in switchers.iter_mut() {
            let timed = sw.end_tick[team] != 0;
            sw.type_[team] = match (sw.status[team], timed) {
                (true, true) => TILE_SWITCHTIMEDOPEN,
                (true, false) => TILE_SWITCHOPEN,
                (false, true) => TILE_SWITCHTIMEDCLOSE,
                (false, false) => TILE_SWITCHCLOSE,
            };
        }
    }

    /// Compares the server's local character at `tick` with its last prediction.
    fn compare_with_server(&mut self, tick: i32) {
        let Some(server) = self.local_char().map(|c| core_obj(&c.cur)) else {
            return;
        };
        let Some(&predicted) = self.history.get(&tick) else {
            return;
        };
        self.stats.compared += 1;
        let differs = |a: &ObjCharacterCore, b: &ObjCharacterCore| {
            ObjCharacterCore { tick: 0, ..*a } != ObjCharacterCore { tick: 0, ..*b }
        };
        if differs(&predicted, &server) {
            self.stats.mismatched += 1;
            self.stats.last_mismatch = Some((tick, predicted, server));
        }
    }

    fn local_char(&self) -> Option<&SnapChar> {
        self.chars
            .get(usize::try_from(self.local_id).ok()?)?
            .as_ref()
    }

    /// C++: `CGameClient::UpdatePrediction`.
    fn update_prediction(&mut self, client: &Client, tick: i32) {
        let local = self.local_id;
        let Some(local_char) = self.local_char().copied() else {
            if local >= 0 {
                self.world.remove_character(local as usize);
            }
            return;
        };
        let local = local as usize;
        if local_char.cur.ammo_count > 0 && local_char.cur.weapon != g::WEAPON_NINJA {
            self.world.config.infinite_ammo = false;
        }
        let collision = self.tuning.0[Param::PlayerCollision as usize] != 0;
        let hooking = self.tuning.0[Param::PlayerHooking as usize] != 0;
        self.world.config.is_solo = local_char.ext.is_none() && !collision && !hooking;

        // update strong and weak hook
        let has_local = self.world.character(local).is_some();
        if has_local && !self.spectating && (collision || hooking) {
            if local_char.ext.is_some() {
                let mut ids = [-1i32; MAX_CLIENTS];
                for i in 0..MAX_CLIENTS {
                    if let Some((ch, _)) = self.world.character(i)
                        && let Some(slot) = ids.get_mut(ch.strong_weak_id as usize)
                    {
                        *slot = i as i32;
                    }
                }
                for id in ids.into_iter().filter(|&id| id >= 0) {
                    // C++: CCharOrder::GiveStrong
                    self.char_order.retain(|&i| i != id as usize);
                    self.char_order.insert(0, id as usize);
                }
            }
            // ponytail: without the extended object C++ detects strong hooks by hand
            // (DetectStrongHook); vanilla servers are later work (PLAN §11).
            self.world.reinsert(self.char_order.iter().copied());
        }

        // advance the server world to the snapshot tick
        if has_local && (self.world.game_tick - tick).abs() < TICK_SPEED {
            for t in self.world.game_tick + 1..=tick {
                let input = client.input_at(t);
                if let Some(input) = &input {
                    self.world.on_direct_input(local, input);
                }
                self.world.game_tick = t;
                if let Some(input) = &input {
                    self.world.on_predicted_input(local, input);
                }
                self.world.tick();
            }
        } else {
            self.world.game_tick = tick;
            if let Some(input) = client.input_at(tick) {
                self.world.set_input(local, &input);
            }
        }

        // update the server world with the new snapshot
        self.world.net_obj_begin(self.teams.clone(), self.local_id);
        let items = client.snapshot_at(tick).unwrap_or(&[]);
        // C++: GameTeam = IsTeamPlay() ? m_aClients[i].m_Team : i. ponytail: C++ keeps a
        // client's team from older snapshots when its PlayerInfo is missing; here it is 0.
        let team_play = items
            .iter()
            .find(|it| it.type_id == g::NETOBJTYPE_GAMEINFO)
            .is_some_and(|it| ints(it, 1)[0] & g::GAMEFLAG_TEAMS != 0);
        let game_team = |i: usize| {
            if !team_play {
                return i as i32;
            }
            items
                .iter()
                .filter(|it| it.type_id == g::NETOBJTYPE_PLAYERINFO)
                .map(|it| g::ObjPlayerInfo::from_ints(&ints(it, g::ObjPlayerInfo::NUM_INTS)))
                .find(|p| p.client_id == i as i32)
                .map_or(0, |p| p.team)
        };
        for (i, c) in self.chars.iter().enumerate() {
            // ponytail: characters without the extended object (vanilla servers) are
            // skipped, see `Character::read`
            if let Some(SnapChar {
                cur,
                ext: Some(ext),
                ..
            }) = c
            {
                self.world
                    .net_char_add(i, cur, ext, game_team(i), i == local);
            }
        }
        // C++: SnapCollectEntities: entities sorted by id with their EntityEx
        for (i, ex) in crate::items::snap_entities(items) {
            let it = &items[i];
            let data = ints(it, ObjDDNetLaser::NUM_INTS); // the largest entity object
            self.world
                .net_obj_add(it.id, it.type_id, &data, ex.as_ref());
        }
        self.world.net_obj_end();
    }

    /// C++: `CGameClient::UpdateRenderedCharacters` (gameclient.cpp:3762) with
    /// `cl_antiping 0`: every active character between the snapshots with
    /// `IntraGameTick`; the local one, when predicting, between its predicted cores with
    /// `PredIntraGameTick`. Ordered by client id.
    pub fn rendered(&self, client: &Client) -> Vec<Rendered> {
        let pos = |c: &ObjCharacter| vec2(c.x as f32, c.y as f32);
        let predicted = self
            .prev_predicted_core
            .as_ref()
            .zip(self.predicted_core.as_ref());
        (0..MAX_CLIENTS)
            .filter_map(|i| {
                let c = self.chars[i]?;
                let mut r = Rendered {
                    id: i as i32,
                    prev: c.prev,
                    cur: c.cur,
                    pos: mix(pos(&c.prev), pos(&c.cur), client.intra_tick),
                    predicted: false,
                    ext: c.ext,
                    prev_ext: c.prev_ext,
                };
                // C++ also needs the character in the predicted world, where it is once
                // a prediction ran (`predicted`)
                if self.predicting
                    && i as i32 == self.local_id
                    && let Some((prev, cur)) = predicted
                {
                    write_core(&mut r.cur, cur);
                    write_core(&mut r.prev, prev);
                    r.predicted = true;
                    r.pos = mix(pos(&r.prev), pos(&r.cur), client.pred_intra_tick);
                }
                Some(r)
            })
            .collect()
    }

    /// C++: `m_aClients[id].m_Predicted` where it is the predicted core: the local tee
    /// while predicting (other tees' cores come from their `DDNetCharacter`, see
    /// `physics::core::read_ddnet`).
    pub fn predicted_core(&self, id: i32) -> Option<&CharacterCore> {
        (self.predicting && id == self.local_id)
            .then_some(self.predicted_core.as_ref())
            .flatten()
    }

    /// The map's collision.
    pub fn collision(&self) -> &Collision {
        &self.col
    }

    /// C++: `m_aClients[local].m_Predicted.m_FreezeEnd`, the predicted freeze of the
    /// local tee (`None` before the first prediction).
    pub fn predicted_freeze_end(&self) -> Option<i32> {
        self.predicted_core.as_ref().map(|c| c.freeze_end)
    }

    /// C++: `m_GameWorld.m_WorldConfig.m_UseTuneZones`.
    pub fn use_tune_zones(&self) -> bool {
        self.world.config.use_tune_zones
    }

    /// C++: the switch test of `CItems::OnRender`: switch `number` exists and is off for
    /// `SwitchStateTeam()` (the switch state object's team, else the local player's).
    pub fn switch_inactive(&self, number: i32) -> bool {
        let team = if self.switch_state_team >= 0 {
            self.switch_state_team
        } else if self.local_id < 0 {
            0
        } else {
            self.teams.team(self.local_id)
        };
        let switchers = &self.world.core.switchers;
        number > 0
            && (number as usize) < switchers.len()
            && !switchers[number as usize].status[team as usize]
    }

    /// C++: `m_GameInfo.m_BugDDRaceInput`.
    pub fn bug_ddrace_input(&self) -> bool {
        self.world.config.bug_ddrace_input
    }

    /// C++: `CGameClient::IsOtherTeam` for a playing (not spectating) local player.
    pub fn is_other_team(&self, id: i32) -> bool {
        let local = self.local_id;
        if local < 0 || id < 0 {
            return false;
        }
        if (self.teams.is_solo(local) || self.teams.is_solo(id)) && id != local {
            return true;
        }
        let t = &self.teams;
        if t.team(id) == t.team_super() || t.team(local) == t.team_super() {
            return false;
        }
        t.team(id) != t.team(local)
    }

    /// C++: `CGameClient::OnPredict` (only called for sane prediction ticks, like
    /// `CClient::Update`).
    fn predict(&mut self, client: &Client) {
        let Some(cur) = client.game_tick() else {
            return;
        };
        let pred = client.pred_tick();
        if !(pred > cur && pred < cur + client.max_latency_ticks()) || self.local_char().is_none() {
            return;
        }
        let local = self.local_id as usize;
        let before = self.predicted_char;

        let mut w = self.world.copy_world();
        // don't predict inactive players, or players from other teams
        for i in 0..MAX_CLIENTS {
            if let Some((ch, _)) = w.character(i)
                && ((self.chars[i].is_none() && ch.snap_ticks > 10) || self.is_other_team(i as i32))
            {
                w.remove_character(i);
            }
        }
        if w.character(local).is_none() {
            return;
        }
        for t in cur + 1..=pred {
            if t == pred {
                self.prev_predicted_core = w.character(local).map(|c| c.1.clone());
            }
            let input = client.input_at(t);
            if let Some(input) = &input {
                w.on_direct_input(local, input);
            }
            w.game_tick = t;
            if let Some(input) = &input {
                w.on_predicted_input(local, input);
            }
            w.tick();
            let core = w.character(local).unwrap().1.write();
            self.history.insert(t, core);
            if t == pred {
                self.predicted_char = Some(core);
                self.predicted_core = w.character(local).map(|c| c.1.clone());
            }
        }
        if self.predicted_tick == pred {
            self.stats.repredictions += 1;
            if before != self.predicted_char {
                self.stats.prediction_errors += 1;
            }
        }
        self.predicted_tick = pred;
        self.history = self.history.split_off(&(cur - TICK_SPEED));
    }
}
