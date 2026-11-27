//! Client prediction driven by server snapshots. C++: the parts of
//! `CGameClient::OnNewSnapshot`, `UpdatePrediction` and `OnPredict` (gameclient.cpp)
//! with the prediction and antiping settings (`PredictConfig`).
//!
//! `rendered` ports `UpdateRenderedCharacters`: what the renderer draws between ticks.
//!
//! Also the game messages that change the prediction world (`Sv_TuneParams` with
//! `UpdateLocalTuning` and the map's tune settings, `Sv_TeamsState`, `Sv_KillMsg`,
//! `Sv_KillMsgTeam`, `Sv_PreInput`).
//!
//! Not ported: the dummy (`PredictDummy()` branches), predicted events.

use std::collections::BTreeMap;

use std::rc::Rc;

use crate::console;
use crate::map::Map;
use crate::net::client::{self, Client, PredictMsg, Timing};
use crate::net::conn::FREQ;
use crate::physics::collision::Collision;
use crate::physics::core::{
    COREEVENT_AIR_JUMP, COREEVENT_GROUND_JUMP, COREEVENT_HOOK_ATTACH_GROUND,
    COREEVENT_HOOK_ATTACH_PLAYER, COREEVENT_HOOK_HIT_NOHOOK, CharacterCore, PHYSICAL_SIZE,
    WorldCore, read_ddnet,
};
use crate::physics::teams::{MAX_CLIENTS, NUM_DDRACE_TEAMS, TEAM_FLOCK, TeamsCore};
use crate::physics::tuning::{Param, Tuning};
use crate::physics::vmath::{Vec2, length, mix, vec2};
use crate::physics::world::{
    GameWorld, NUM_TUNE_ZONES, PredictedEvent, TUNE_ZONE_OVERRIDE_NONE, WorldConfig, core_obj,
    create_predicted_sound,
};
use crate::protocol::generated::{
    self as g, MsgSvPreInput, ObjCharacter, ObjCharacterCore, ObjDDNetCharacter, ObjDDNetLaser,
    ObjGameInfoEx, ObjPlayerInput, ObjSpectatorInfo, ObjSwitchState,
};
use crate::protocol::snapshot::Item;
use crate::protocol::sounds::*;

const TICK_SPEED: i32 = 50;
const TILE_SWITCHTIMEDOPEN: i32 = 22;
const TILE_SWITCHTIMEDCLOSE: i32 = 23;
const TILE_SWITCHOPEN: i32 = 24;
const TILE_SWITCHCLOSE: i32 = 25;
/// C++: `MAPITEMTYPE_INFO`.
const MAPITEMTYPE_INFO: i32 = 1;
/// C++: `VANILLA_MAX_CLIENTS`.
const VANILLA_MAX_CLIENTS: usize = 16;
/// C++: the size of `CClientData::m_aPreInputs`, `m_aPredPos` and `m_aPredTick`.
const NUM_PRE_INPUTS: usize = 200;
const NUM_PRED_POS: usize = 200;

/// C++: the prediction and antiping config variables (config_variables.h:15-27, 730),
/// all but `cl_prediction_margin` (`Client::prediction_margin`). Defaults as in C++.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PredictConfig {
    pub predict: bool,
    pub antiping: bool,
    /// 0 off, 1 all, 2 only interfering players, 3 = 2 and frozen players.
    pub antiping_players: i32,
    pub antiping_grenade: bool,
    pub antiping_weapons: bool,
    pub antiping_smooth: bool,
    pub antiping_gunfire: bool,
    pub antiping_preinput: bool,
    /// ms; others are predicted only that far ahead when set.
    pub antiping_limit: i32,
    pub antiping_percent: i32,
    /// 0 off, 1 on, 2 partial (the local tee may move in freeze the last 1-2 ticks).
    pub predict_freeze: i32,
    /// C++: `cl_predict_events` (default 0).
    pub predict_events: bool,
    /// C++: `snd_enable`, `snd_game`, `snd_gun`, `snd_long_pain`.
    pub snd_enable: bool,
    pub snd_game: bool,
    pub snd_gun: bool,
    pub snd_long_pain: bool,
}

impl Default for PredictConfig {
    fn default() -> Self {
        Self {
            predict: true,
            antiping: false,
            antiping_players: 1,
            antiping_grenade: true,
            antiping_weapons: true,
            antiping_smooth: false,
            antiping_gunfire: true,
            antiping_preinput: true,
            antiping_limit: 0,
            antiping_percent: 100,
            predict_freeze: 1,
            predict_events: false,
            snd_enable: true,
            snd_game: true,
            snd_gun: true,
            snd_long_pain: true,
        }
    }
}

/// A sound the game wants played: C++'s `m_Sounds.PlayAt` (CHN_WORLD, volume 1) and
/// `m_MapSounds.PlayAt` calls of `OnNewSnapshot` and `OnPredict`, in call order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SoundEvent {
    /// A sound set (`protocol::sounds::SOUND_*`) at a world position.
    World(i32, Vec2),
    /// A map sound (`NETEVENTTYPE_MAPSOUNDWORLD`) at a world position.
    MapWorld(i32, Vec2),
}

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
    /// C++ `m_IsPredicted`: drawn between the predicted ticks with `PredIntraGameTick`.
    pub predicted: bool,
    /// C++ `m_IsPredictedLocal`: the local tee, predicted (weapons drawn predicted).
    pub predicted_local: bool,
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
    /// C++: `m_aClients[i].m_PrevPredicted` and `m_Predicted`: each character's core
    /// before and after the prediction tick (the local one's: the predicted tick).
    pub prev_predicted: Vec<CharacterCore>,
    pub predicted: Vec<CharacterCore>,
    /// C++: `m_aClients[i].m_aPredPos` and `m_aPredTick`, indexed by tick % 200.
    pub pred_pos: Vec<[Vec2; NUM_PRED_POS]>,
    pub pred_pos_tick: Vec<[i32; NUM_PRED_POS]>,
    /// C++: `m_PredictedWorld` and `m_PrevPredictedWorld` (the world at the prediction
    /// tick, items are drawn from it).
    pub predicted_world: Option<GameWorld>,
    pub prev_predicted_world: Option<GameWorld>,
    /// C++: `m_aClients[i].m_aSmoothStart`, `m_aSmoothLen` (per axis, `FREQ` units),
    /// `m_aLastPos`, `m_aLastActive`: the antiping smoothing state.
    pub smooth_start: Vec<[i64; 2]>,
    pub smooth_len: Vec<[i64; 2]>,
    pub last_pos: Vec<Vec2>,
    pub last_active: Vec<bool>,
    /// C++: `m_NewTick`: a snapshot arrived this frame.
    pub new_tick: bool,
    /// C++: the `m_Snap.m_pLocalCharacter`, spectator and pause parts of `Predict()`.
    predicting: bool,
    pub config: PredictConfig,
    /// C++: `m_aSwitchStateTeam`: the team of the only switch state object, else -1.
    switch_state_team: i32,
    predicted_tick: i32,
    /// C++: the pause test of `OnPredict` (`GAMESTATEFLAG_PAUSED` only).
    game_paused: bool,
    /// C++ `m_Snap.m_SpecInfo.m_Active`, `IsWorldPaused()` (paused or game over) and
    /// `m_GameInfo.m_AllowZoom` of the current snapshot.
    pub spectating: bool,
    pub world_paused: bool,
    pub allow_zoom: bool,
    /// C++ `m_GameInfo.m_HudDDRace`: the server wants the DDRace HUD (GameInfoEx 7+).
    pub hud_ddrace: bool,
    /// C++ `m_GameInfo.m_TimeScore` and `m_AllowXSkins` (for the client list).
    pub time_score: bool,
    pub allow_x_skins: bool,
    /// C++ `CGameClient::MaxTeamSize()`: the game info's, else `sv_max_team_size` (128).
    pub max_team_size: i32,
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
    /// C++ `m_GameInfo.m_RaceSounds` and `m_PredictEvents`.
    race_sounds: bool,
    predict_events: bool,
    /// C++: `m_aLastNewPredictedTick[Dummy]`.
    last_new_predicted_tick: i32,
    /// `IntraGameTick` of the frame, for the positions of `OnNewSnapshot`'s effects.
    intra_tick: f32,
    /// This frame's sounds, drained by the client.
    pub sound_events: Vec<SoundEvent>,
}

/// C++: a `CClientData` core after `CCharacterCore::Reset` (`CClientData::Reset`).
fn reset_core() -> CharacterCore {
    let mut c = CharacterCore::new();
    c.reset_fields();
    c
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
            prev_predicted: vec![reset_core(); MAX_CLIENTS],
            predicted: vec![reset_core(); MAX_CLIENTS],
            pred_pos: vec![[Vec2::default(); NUM_PRED_POS]; MAX_CLIENTS],
            pred_pos_tick: vec![[0; NUM_PRED_POS]; MAX_CLIENTS],
            predicted_world: None,
            prev_predicted_world: None,
            smooth_start: vec![[0; 2]; MAX_CLIENTS],
            smooth_len: vec![[0; 2]; MAX_CLIENTS],
            last_pos: vec![Vec2::default(); MAX_CLIENTS],
            last_active: vec![false; MAX_CLIENTS],
            new_tick: false,
            predicting: false,
            config: PredictConfig::default(),
            switch_state_team: -1,
            predicted_tick: -1,
            game_paused: false,
            spectating: false,
            world_paused: false,
            hud_ddrace: false,
            time_score: false,
            allow_x_skins: false,
            max_team_size: 128,
            allow_zoom: false,
            history: BTreeMap::new(),
            stats: Stats::default(),
            race_sounds: false,
            predict_events: false,
            last_new_predicted_tick: -1,
            intra_tick: 0.0,
            sound_events: Vec::new(),
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
    /// `now` is C++ `time_get()`.
    pub fn update(&mut self, client: &mut Client, now: i64) {
        // C++ handles messages while pumping the network, before CClient::Update
        // switches snapshots
        for msg in std::mem::take(&mut client.predict_msgs) {
            self.on_message(msg);
        }
        self.new_tick = false; // C++: cleared at the end of the previous frame's OnRender
        self.sound_events.clear();
        let switches = std::mem::take(&mut client.snapshot_switches);
        let repredict = std::mem::take(&mut client.repredict);
        let client = &*client;
        let input_at = |t| client.input_at(t);
        self.intra_tick = client.timing(now).intra_tick;
        for (prev, cur) in switches {
            let Some(items) = client.snapshot_at(cur) else {
                continue;
            };
            let prev_items = prev.and_then(|t| client.snapshot_at(t)).unwrap_or(&[]);
            self.on_new_snapshot(items, prev_items, prev, cur, &input_at);
        }
        // C++ CClient::Update: only sane predictions
        let t = client.timing(now);
        if repredict
            && client.game_tick().is_some()
            && t.pred_tick > t.game_tick
            && t.pred_tick < t.game_tick + client.max_latency_ticks()
        {
            self.predict(&t, &input_at);
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
        pre_input(&self.pre_inputs, id, tick)
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

    /// C++: `m_GameWorld`, the server-state world.
    pub fn world(&self) -> &GameWorld {
        &self.world
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

    /// C++: the prediction parts of `CGameClient::OnNewSnapshot` for a switch to the
    /// snapshot `items` of `tick` from `prev_items` of `prev`. `input_at` is C++
    /// `CClient::GetInput`.
    pub fn on_new_snapshot(
        &mut self,
        items: &[Item],
        prev_items: &[Item],
        prev: Option<i32>,
        tick: i32,
        input_at: &dyn Fn(i32) -> Option<ObjPlayerInput>,
    ) {
        self.local_id = items
            .iter()
            .filter(|it| it.type_id == g::NETOBJTYPE_PLAYERINFO)
            .map(|it| g::ObjPlayerInfo::from_ints(&ints(it, g::ObjPlayerInfo::NUM_INTS)))
            .find(|p| p.local != 0)
            .map_or(-1, |p| p.client_id);

        self.chars = vec![None; MAX_CLIENTS];
        // C++: every DDNetCharacter of the snapshot, active or not (gameclient.cpp:1887);
        // ponytail: C++ indexes out of bounds with a negative id, those are skipped
        for it in items
            .iter()
            .filter(|it| it.type_id == g::NETOBJTYPE_DDNETCHARACTER)
        {
            let Ok(id) = usize::try_from(it.id) else {
                continue;
            };
            if id < MAX_CLIENTS {
                let ext = ObjDDNetCharacter::from_ints(&ints(it, ObjDDNetCharacter::NUM_INTS));
                read_ddnet(&mut self.predicted[id], &ext);
                self.teams
                    .set_solo(it.id, ext.flags & g::CHARACTERFLAG_SOLO != 0);
            }
        }
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
        let state = items
            .iter()
            .find(|it| it.type_id == g::NETOBJTYPE_GAMEINFO)
            .map_or(0, |it| ints(it, 2)[1]);
        let paused = state & (g::GAMESTATEFLAG_GAMEOVER | g::GAMESTATEFLAG_PAUSED) != 0;
        self.game_paused = state & g::GAMESTATEFLAG_PAUSED != 0;
        self.predicting = self.local_char().is_some() && !spectating && !paused;
        (self.spectating, self.world_paused) = (spectating, paused);
        self.process_events(items, tick);

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
        let fastcap = flags & g::GAMEINFOFLAG_GAMETYPE_FASTCAP != 0;
        let predict_vanilla = if version >= 2 {
            flags & g::GAMEINFOFLAG_PREDICT_VANILLA != 0
        } else {
            flags & g::GAMEINFOFLAG_GAMETYPE_VANILLA != 0 || fastcap
        };
        let old_laser = version >= 12 && flags2 & g::GAMEINFOFLAG2_OLD_LASER != 0;
        let fng = flags & g::GAMEINFOFLAG_GAMETYPE_FNG != 0;
        let predict_fng = if version >= 2 {
            flags & g::GAMEINFOFLAG_PREDICT_FNG != 0
        } else {
            fng
        };
        let no_weak = version >= 8 && flags2 & g::GAMEINFOFLAG2_NO_WEAK_HOOK != 0;
        self.hud_ddrace = version >= 7 && flags2 & g::GAMEINFOFLAG2_HUD_DDRACE != 0;
        self.race_sounds = ddrace || fng || block_worlds;
        self.predict_events = if version >= 11 {
            flags2 & g::GAMEINFOFLAG2_PREDICT_EVENTS != 0
        } else {
            flags & g::GAMEINFOFLAG_GAMETYPE_VANILLA != 0
        };
        let race = ddrace || fastcap || flags & g::GAMEINFOFLAG_GAMETYPE_RACE != 0;
        self.time_score = if version >= 0 {
            flags & g::GAMEINFOFLAG_TIMESCORE != 0
        } else {
            race
        };
        self.allow_x_skins = version >= 5 && flags2 & g::GAMEINFOFLAG2_ALLOW_X_SKINS != 0;
        self.max_team_size = if version >= 12 && ex.max_team_size != 0 {
            ex.max_team_size
        } else {
            128
        };
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

        // C++: the world config part of UpdatePrediction
        let c = &mut self.world.config;
        c.is_vanilla = predict_vanilla;
        c.is_ddrace = predict_ddrace;
        c.old_laser = old_laser;
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

        self.detect_air_jumps();
        self.new_tick = true;
        self.compare_with_server(tick);
        self.update_prediction(items, tick, input_at);
    }

    /// C++: the sounds of `CGameClient::ProcessEvents` (gameclient.cpp:1404): hammer hits and
    /// spawns (through `CEffects`), world sounds the prediction did not already play, map
    /// sounds. The other events only make particles.
    fn process_events(&mut self, items: &[Item], tick: i32) {
        let c = self.config;
        for it in items {
            let d = ints(it, 3);
            let pos = vec2(d[0] as f32, d[1] as f32);
            match it.type_id {
                g::NETEVENTTYPE_HAMMERHIT => {
                    let e = PredictedEvent::new(it.type_id, pos, -1, tick, -1);
                    if !self.check_predicted_event_handled(e) && c.snd_game {
                        self.sound_events
                            .push(SoundEvent::World(SOUND_HAMMER_HIT, pos));
                    }
                }
                g::NETEVENTTYPE_SPAWN if c.snd_game => {
                    self.sound_events
                        .push(SoundEvent::World(SOUND_PLAYER_SPAWN, pos));
                }
                g::NETEVENTTYPE_SOUNDWORLD => {
                    let id = d[2];
                    if !c.snd_game || self.race_sound_off(id) {
                        continue;
                    }
                    let e = PredictedEvent::new(it.type_id, pos, -1, tick, id);
                    if !self.check_predicted_event_handled(e) {
                        self.sound_events.push(SoundEvent::World(id, pos));
                    }
                }
                g::NETEVENTTYPE_MAPSOUNDWORLD => {
                    self.sound_events.push(SoundEvent::MapWorld(d[2], pos));
                }
                _ => {}
            }
        }
    }

    fn race_sound_off(&self, id: i32) -> bool {
        race_sound_off(&self.config, self.race_sounds, id)
    }

    /// C++: `CGameWorld::CheckPredictedEventHandled` on `m_PredictedWorld`: a server event
    /// the prediction already played (any tick up to the event's), which it forgets then.
    fn check_predicted_event_handled(&mut self, check: PredictedEvent) -> bool {
        let Some(w) = &mut self.predicted_world else {
            return false;
        };
        let Some(i) = w.predicted_events.iter().position(|e| {
            e.handled
                && e.event_id == check.event_id
                && e.pos == check.pos
                && e.tick <= check.tick
                && e.extra_info == check.extra_info
        }) else {
            return false;
        };
        w.predicted_events.remove(i);
        true
    }

    /// C++: "detect air jump for other players" of `OnNewSnapshot` (gameclient.cpp:2405)
    /// without a dummy: the local tee only when not predicting.
    fn detect_air_jumps(&mut self) {
        let predict = self.predict_on();
        for i in 0..MAX_CLIENTS {
            let Some(ch) = &self.chars[i] else {
                continue;
            };
            if ch.cur.jumped & 2 == 0 || ch.prev.jumped & 2 != 0 {
                continue;
            }
            if predict && i as i32 == self.local_id {
                continue;
            }
            let prev = vec2(ch.prev.x as f32, ch.prev.y as f32);
            let pos = mix(
                prev,
                vec2(ch.cur.x as f32, ch.cur.y as f32),
                self.intra_tick,
            );
            if !self.col.is_on_ground(prev, PHYSICAL_SIZE) && self.config.snd_game {
                self.sound_events
                    .push(SoundEvent::World(SOUND_PLAYER_AIRJUMP, pos));
            }
        }
    }

    /// C++: `CGameClient::HandlePredictedEvents` (gameclient.cpp:3819), its sounds.
    fn handle_predicted_events(
        c: &PredictConfig,
        race_sounds: bool,
        out: &mut Vec<SoundEvent>,
        w: &mut GameWorld,
        tick: i32,
    ) {
        let mut i = 0;
        while i < w.predicted_events.len() {
            let e = w.predicted_events[i];
            if !e.handled && e.tick <= tick {
                if e.event_id == g::NETEVENTTYPE_SOUNDWORLD {
                    if race_sound_off(c, race_sounds, e.extra_info) {
                        w.predicted_events.remove(i);
                        continue;
                    }
                    out.push(SoundEvent::World(e.extra_info, e.pos));
                } else if e.event_id == g::NETEVENTTYPE_HAMMERHIT && c.snd_game {
                    out.push(SoundEvent::World(SOUND_HAMMER_HIT, e.pos));
                }
                w.predicted_events[i].handled = true;
                i += 1;
            } else if tick - e.tick > 3 * TICK_SPEED {
                // remove too old events
                w.predicted_events.remove(i);
            } else {
                i += 1;
            }
        }
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
    fn update_prediction(
        &mut self,
        items: &[Item],
        tick: i32,
        input_at: &dyn Fn(i32) -> Option<ObjPlayerInput>,
    ) {
        self.world.config.predict_freeze = self.config.predict_freeze != 0;
        self.world.config.predict_weapons = self.antiping_weapons();
        // C++ reads g_Config.m_ClAntiPingPlayers itself (interference)
        self.world.config.antiping_players = self.config.antiping_players;
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
        let pre = PreInputs {
            enabled: self.config.antiping_preinput,
            local: self.local_id,
            ring: &self.pre_inputs,
        };
        if has_local && (self.world.game_tick - tick).abs() < TICK_SPEED {
            for t in self.world.game_tick + 1..=tick {
                let input = input_at(t);
                if let Some(input) = &input {
                    self.world.on_direct_input(local, input);
                }
                pre.apply(t, true, &mut self.world);
                self.world.game_tick = t;
                if let Some(input) = &input {
                    self.world.on_predicted_input(local, input);
                }
                pre.apply(t, false, &mut self.world);
                self.world.tick();
                store_pred_pos(&mut self.pred_pos, &mut self.pred_pos_tick, &self.world, t);
            }
        } else {
            self.world.game_tick = tick;
            if let Some(input) = input_at(tick) {
                self.world.set_input(local, &input);
            }
        }
        store_pred_pos(
            &mut self.pred_pos,
            &mut self.pred_pos_tick,
            &self.world,
            tick,
        );

        // update the server world with the new snapshot
        self.world.net_obj_begin(self.teams.clone(), self.local_id);
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

    /// C++: `CGameClient::UpdateRenderedCharacters` (gameclient.cpp:3762): every active
    /// character between the snapshots with `IntraGameTick`; predicted ones (the local
    /// tee, and others per `cl_antiping_players` unless in another team) between their
    /// predicted cores with `PredIntraGameTick`, others with `GetSmoothPos` when
    /// smoothing. Ordered by client id.
    pub fn rendered(&self, t: &Timing) -> Vec<Rendered> {
        let pos = |c: &ObjCharacter| vec2(c.x as f32, c.y as f32);
        let antiping_players = self.antiping_players();
        (0..MAX_CLIENTS)
            .filter_map(|i| {
                let c = self.chars[i]?;
                let mut r = Rendered {
                    id: i as i32,
                    prev: c.prev,
                    cur: c.cur,
                    pos: mix(pos(&c.prev), pos(&c.cur), t.intra_tick),
                    predicted: false,
                    predicted_local: false,
                    ext: c.ext,
                    prev_ext: c.prev_ext,
                };
                let ch = (self.predicted_world.as_ref()).and_then(|w| w.character(i));
                // C++ dummy branch not ported
                let antiping_player = antiping_players == 1
                    || (antiping_players >= 2 && ch.is_some_and(|(ch, _)| ch.interfering));
                let local = i as i32 == self.local_id;
                let Some((ch, _)) = ch else {
                    return Some(r);
                };
                if !(self.predict_on()
                    && (local || (antiping_player && !self.is_other_team(i as i32))))
                {
                    return Some(r);
                }
                write_core(&mut r.cur, &self.predicted[i]);
                write_core(&mut r.prev, &self.prev_predicted[i]);
                r.predicted = true;
                r.pos = mix(pos(&r.prev), pos(&r.cur), t.pred_intra_tick);
                if local {
                    r.predicted_local = true;
                    // C++ m_NinjaJetpack is only set by the vanilla Read path (not ported)
                    let predicted_weapon = self.predicted[i].active_weapon;
                    if self.antiping_gunfire()
                        && (c.cur.weapon != g::WEAPON_NINJA || c.cur.weapon == predicted_weapon)
                    {
                        r.cur.attack_tick = ch.attack_tick;
                        if c.cur.weapon != g::WEAPON_NINJA {
                            r.cur.weapon = predicted_weapon;
                        }
                    }
                } else {
                    // use unpredicted values for other players
                    r.prev.angle = c.prev.angle;
                    r.cur.angle = c.cur.angle;
                    if self.config.antiping_smooth {
                        r.pos = self.smooth_pos(i, t);
                    }
                }
                Some(r)
            })
            .collect()
    }

    /// C++: `m_aClients[id].m_Predicted` where it is the predicted core: the local tee
    /// while predicting (other tees' cores come from their `DDNetCharacter`, see
    /// `physics::core::read_ddnet`).
    pub fn predicted_core(&self, id: i32) -> Option<&CharacterCore> {
        (self.predict_on() && id == self.local_id && self.predicted_world.is_some())
            .then(|| self.predicted.get(id as usize))
            .flatten()
    }

    /// C++: `CClient::GetPredictionTick` with this predictor's `cl_antiping_limit` and
    /// `cl_antiping_percent`.
    pub fn prediction_tick(&self, t: &Timing) -> i32 {
        client::prediction_tick(
            client::prediction_time(t.game_now, t.pred_now),
            self.config.antiping_limit,
            self.config.antiping_percent,
            t.game_tick,
            t.pred_tick,
        )
    }

    /// C++: the flight time `Ct` of `CItems::RenderProjectile` (items.cpp:55): at the
    /// prediction tick with antiping grenades (unless the owner is in another team),
    /// else since the previous snapshot. `last_tick_time` is C++ `s_LastGameTickTime`.
    pub fn projectile_time(
        &self,
        d: &crate::items::ProjectileData,
        t: &Timing,
        local_in_game: bool,
        last_tick_time: f32,
    ) -> f32 {
        let is_other_team = d.extra_info && d.owner >= 0 && self.is_other_team(d.owner);
        if self.predict_on() && self.antiping_grenade() && local_in_game && !is_other_team {
            ((self.prediction_tick(t) - 1 - d.start_tick) as f32 + t.pred_intra_tick)
                / TICK_SPEED as f32
        } else {
            (t.prev_game_tick - d.start_tick) as f32 / TICK_SPEED as f32 + last_tick_time
        }
    }

    /// C++: `Ticks` and `TicksHead` of `CItems::RenderLaser` (items.cpp:313) for a laser
    /// of (clamped) type `ty`, drawn `predicted` or from the snapshot.
    pub fn laser_ticks(
        &self,
        d: &crate::items::LaserData,
        ty: i32,
        predicted: bool,
        t: &Timing,
    ) -> (f32, f32) {
        let head = t.game_tick as f32;
        if ty == g::LASERTYPE_DOOR {
            (1.0, head)
        } else if predicted {
            let ticks = (self.prediction_tick(t) - d.start_tick) as f32 + t.pred_intra_tick;
            (ticks, head + t.pred_intra_tick)
        } else {
            let ticks = (t.game_tick - d.start_tick) as f32 + t.intra_tick;
            (ticks, head + t.intra_tick)
        }
    }

    /// C++: `m_PredictedTick`: the predicted tick of the last `OnPredict`.
    pub fn predicted_tick(&self) -> i32 {
        self.predicted_tick
    }

    /// C++: `CGameClient::Predict()`.
    pub fn predict_on(&self) -> bool {
        self.config.predict && self.predicting
    }

    /// C++: `CGameClient::AntiPingPlayers()`: the `cl_antiping_players` mode, 0 if off.
    pub fn antiping_players(&self) -> i32 {
        let c = &self.config;
        if c.antiping && c.antiping_players != 0 && !self.spectating {
            c.antiping_players
        } else {
            0
        }
    }

    /// C++: `CGameClient::AntiPingGrenade()`.
    pub fn antiping_grenade(&self) -> bool {
        self.config.antiping && self.config.antiping_grenade && !self.spectating
    }

    /// C++: `CGameClient::AntiPingWeapons()`.
    pub fn antiping_weapons(&self) -> bool {
        self.config.antiping && self.config.antiping_weapons && !self.spectating
    }

    /// C++: `CGameClient::AntiPingGunfire()`.
    pub fn antiping_gunfire(&self) -> bool {
        self.antiping_grenade() && self.antiping_weapons() && self.config.antiping_gunfire
    }

    /// The map's collision.
    pub fn collision(&self) -> &Collision {
        &self.col
    }

    /// C++: `m_aClients[local].m_Predicted.m_FreezeEnd`, the predicted freeze of the
    /// local tee (`None` before the first prediction).
    pub fn predicted_freeze_end(&self) -> Option<i32> {
        let local = usize::try_from(self.local_id).ok()?;
        self.predicted_world.as_ref()?;
        Some(self.predicted.get(local)?.freeze_end)
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

    /// C++: `m_Teams.Team(id)`.
    pub fn team(&self, id: i32) -> i32 {
        self.teams.team(id)
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

    /// C++: `CGameClient::GetSmoothPos` (gameclient.cpp:3951): where other tee `id` is
    /// drawn with `cl_antiping_smooth`: between its predicted cores, per axis mixed into
    /// the `pred_pos` history while a smoothing window runs.
    pub fn smooth_pos(&self, id: usize, t: &Timing) -> Vec2 {
        let mut pos = mix(
            self.prev_predicted[id].pos,
            self.predicted[id].pos,
            t.pred_intra_tick,
        );
        for j in 0..2 {
            let len = self.smooth_len[id][j].clamp(1, FREQ);
            let passed = t.now.wrapping_sub(self.smooth_start[id][j]);
            if !(0..len).contains(&passed) {
                continue;
            }
            let mix_amount = 1.0 - (1.0 - passed as f32 / len as f32).powf(1.2);
            let (tick, intra) = client::smooth_tick(t.game_now, t.pred_now, mix_amount);
            let ring = |tick: i32| tick.rem_euclid(NUM_PRED_POS as i32) as usize;
            let ticks = &self.pred_pos_tick[id];
            if tick > 0
                && ticks[ring(tick - 1)] >= t.prev_game_tick
                && ticks[ring(tick)] <= t.pred_tick
            {
                let (a, b) = (
                    self.pred_pos[id][ring(tick - 1)],
                    self.pred_pos[id][ring(tick)],
                );
                set_axis(&mut pos, j, axis(a, j) + (axis(b, j) - axis(a, j)) * intra);
            }
        }
        pos
    }

    /// C++: the `cl_antiping_smooth` block of `OnPredict` (gameclient.cpp:2754): when
    /// another tee's prediction changed by a small error, start or extend a smoothing
    /// window per axis. `before_render` is `GetSmoothPos` of every client before the
    /// prediction. ponytail: C++ measures the window in µs (`1000 * PredTime` ms) against
    /// `time_get()` in ns, so windows last 1000x shorter than meant; kept (DECISIONS.md).
    fn smooth(&mut self, t: &Timing, before_render: &[Vec2]) {
        if !(self.config.antiping_smooth
            && self.predict_on()
            && self.antiping_players() != 0
            && self.new_tick
            && self.predicted_tick >= 0 // MIN_TICK
            && (self.predicted_tick - t.pred_tick).abs() <= 1
            && (t.game_tick - t.prev_game_tick).abs() <= 2)
        {
            return;
        }
        let prediction_time = client::prediction_time(t.game_now, t.pred_now);
        let pred_time = prediction_time.clamp(0, 800);
        // smoothing pace (a lower value will make the smoothing quicker)
        let pace = 4.0 - 1.5 * pred_time as f32 / 800.0;
        let len = ((1000 * pred_time) as f32 * pace) as i64;
        let freq = FREQ as f32;
        for (i, &before) in before_render.iter().enumerate() {
            let Some(c) = self.chars[i] else {
                continue;
            };
            if i as i32 == self.local_id || !self.last_active[i] {
                continue;
            }
            let (prev, pred) = (self.prev_predicted[i].pos, self.predicted[i].pos);
            let new_pos = if self.predicted_tick == t.pred_tick {
                pred
            } else {
                prev
            };
            let div = prediction_time.min(200) as f32;
            let err = vec2(
                (self.last_pos[i].x - new_pos.x) / div,
                (self.last_pos[i].y - new_pos.y) / div,
            );
            let err_len = length(err);
            if !(0.05..=5.0).contains(&err_len) {
                continue;
            }
            let pred_pos = mix(prev, pred, t.pred_intra_tick);
            let cur_pos = mix(
                vec2(c.prev.x as f32, c.prev.y as f32),
                vec2(c.cur.x as f32, c.cur.y as f32),
                t.intra_tick,
            );
            let render_diff = pred_pos - before;
            let pred_diff = pred_pos - cur_pos;
            let mut amount = [0.0f32; 2];
            for (j, a) in amount.iter_mut().enumerate() {
                *a = 1.0;
                if axis(err, j).abs() > 0.05 {
                    *a = 0.0;
                    if axis(render_diff, j).abs() > 0.01 {
                        *a = 1.0 - (axis(render_diff, j) / axis(pred_diff, j)).clamp(0.0, 1.0);
                        *a = 1.0 - (1.0 - *a).powf(1.0 / 1.2);
                    }
                }
                let passed = t.now.wrapping_sub(self.smooth_start[i][j]);
                if (0..len).contains(&passed) {
                    *a = std_min(*a, (passed as f64 / len as f64) as f32);
                }
            }
            for j in 0..2 {
                if axis(render_diff, j).abs() < 0.01
                    && axis(pred_diff, j).abs() < 0.01
                    && (axis(prev, j) - axis(pred, j)).abs() < 0.01
                    && amount[j] > amount[j ^ 1]
                {
                    amount[j] = amount[j ^ 1];
                }
            }
            for j in 0..2 {
                // don't smooth for longer than 700ms, or more than 300ms longer along one
                // axis than the other axis
                let remaining = std_min(
                    std_min((1.0 - amount[j]) * len as f32, freq * 0.700),
                    (1.0 - amount[j ^ 1]) * len as f32 + freq * 0.300,
                ) as i64;
                let start = t.now.wrapping_sub(len.wrapping_sub(remaining));
                let (s, end) = (self.smooth_start[i][j], start.wrapping_add(len));
                if !(s <= end && end <= s.wrapping_add(len)) {
                    self.smooth_start[i][j] = start;
                    self.smooth_len[i][j] = len;
                }
            }
        }
    }

    /// C++: `CGameClient::OnPredict` (called for sane prediction ticks only, see
    /// `update`).
    pub fn predict(&mut self, t: &Timing, input_at: &dyn Fn(i32) -> Option<ObjPlayerInput>) {
        let (cur, pred) = (t.game_tick, t.pred_tick);
        if self.local_char().is_none() {
            return;
        }
        // C++ only copies the snapshot's local character into m_PredictedChar (debug)
        if self.game_paused {
            return;
        }
        let local = self.local_id as usize;
        let before = self.predicted_char;
        let before_render: Vec<Vec2> = (0..MAX_CLIENTS).map(|i| self.smooth_pos(i, t)).collect();

        // PredictedEvents are only handled in predicted world, so update them here
        if let Some(pw) = &self.predicted_world {
            self.world.predicted_events = pw.predicted_events.clone();
        }
        let mut w = self.world.copy_world();
        // C++ reads g_Config.m_ClPredictEvents and g_Config.m_SndEnable live
        w.config.predict_events = self.config.predict_events && self.predict_events;
        w.config.snd_enable = self.config.snd_enable;
        // don't predict inactive players, or entities from other teams
        for i in 0..MAX_CLIENTS {
            if let Some((ch, _)) = w.character(i)
                && ((self.chars[i].is_none() && ch.snap_ticks > 10) || self.is_other_team(i as i32))
            {
                w.remove_character(i);
            }
        }
        w.projectiles.retain(|p| !self.is_other_team(p.owner));
        if w.character(local).is_none() {
            self.predicted_world = Some(w);
            return;
        }
        let c = self.config;
        let prediction_tick = self.prediction_tick(t);
        let pre = PreInputs {
            enabled: c.antiping_preinput,
            local: self.local_id,
            ring: &self.pre_inputs,
        };
        for tick in cur + 1..=pred {
            // fetch the previous characters
            if tick == prediction_tick {
                for i in 0..MAX_CLIENTS {
                    if let Some((_, core)) = w.character(i) {
                        self.prev_predicted[i] = core.clone();
                    }
                }
            }
            if tick == pred
                && let Some((_, core)) = w.character(local)
            {
                self.prev_predicted[local] = core.clone();
            }
            // optionally allow some movement in freeze by not predicting freeze the last
            // one to two ticks
            if c.predict_freeze == 2
                && pred - 1 - pred % 2 <= tick
                && let Some(ch) = w.character_mut(local)
            {
                ch.can_move_in_freeze = true;
            }
            // apply inputs and tick
            let input = input_at(tick);
            if let Some(input) = &input {
                w.on_direct_input(local, input);
            }
            pre.apply(tick, true, &mut w);
            w.game_tick = tick;
            if let Some(input) = &input {
                w.on_predicted_input(local, input);
            }
            pre.apply(tick, false, &mut w);
            w.tick();

            // fetch the current characters
            if tick == prediction_tick {
                self.prev_predicted_world = Some(w.copy_world());
                for i in 0..MAX_CLIENTS {
                    if let Some((_, core)) = w.character(i) {
                        self.predicted[i] = core.clone();
                    }
                }
            }
            if let Some((_, core)) = w.character(local) {
                let obj = core.write();
                self.history.insert(tick, obj);
                if tick == pred {
                    self.predicted_char = Some(obj);
                    self.predicted[local] = core.clone();
                }
            }
            store_pred_pos(&mut self.pred_pos, &mut self.pred_pos_tick, &w, tick);

            // check if we want to trigger effects
            if tick > self.last_new_predicted_tick {
                self.last_new_predicted_tick = tick;
                if let Some((_, core)) = w.character(local) {
                    let (pos, events) = (core.pos, core.triggered_events);
                    let snd = c.snd_game;
                    if c.predict && snd && events & COREEVENT_AIR_JUMP != 0 {
                        self.sound_events
                            .push(SoundEvent::World(SOUND_PLAYER_AIRJUMP, pos));
                    }
                    if snd {
                        for (ev, sound) in [
                            (COREEVENT_GROUND_JUMP, SOUND_PLAYER_JUMP),
                            (COREEVENT_HOOK_ATTACH_GROUND, SOUND_HOOK_ATTACH_GROUND),
                            (COREEVENT_HOOK_HIT_NOHOOK, SOUND_HOOK_NOATTACH),
                        ] {
                            if events & ev != 0 {
                                self.sound_events.push(SoundEvent::World(sound, pos));
                            }
                        }
                        if events & COREEVENT_HOOK_ATTACH_PLAYER != 0 {
                            let tick = w.game_tick;
                            create_predicted_sound(
                                &w.config,
                                &mut w.predicted_events,
                                tick,
                                pos,
                                SOUND_HOOK_ATTACH_PLAYER,
                                self.local_id,
                            );
                        }
                    }
                }
            }
            let out = &mut self.sound_events;
            Self::handle_predicted_events(&c, self.race_sounds, out, &mut w, tick);
        }
        // detect mispredictions of other players and make corrections smoother
        self.smooth(t, &before_render);
        for i in 0..MAX_CLIENTS {
            self.last_active[i] = self.chars[i].is_some();
            if self.last_active[i] {
                self.last_pos[i] = self.predicted[i].pos;
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
        self.predicted_world = Some(w);
    }
}

/// C++: `m_GameInfo.m_RaceSounds` with `snd_gun`/`snd_long_pain` off.
fn race_sound_off(c: &PredictConfig, race_sounds: bool, id: i32) -> bool {
    race_sounds
        && ((id == SOUND_GUN_FIRE && !c.snd_gun)
            || (id == SOUND_PLAYER_PAIN_LONG && !c.snd_long_pain))
}

/// C++: `vec2[j]`.
fn axis(v: Vec2, j: usize) -> f32 {
    if j == 0 { v.x } else { v.y }
}

fn set_axis(v: &mut Vec2, j: usize, value: f32) {
    if j == 0 {
        v.x = value;
    } else {
        v.y = value;
    }
}

/// C++: `std::min` for floats (`b < a ? b : a`; differs from `f32::min` with NaN).
fn std_min(a: f32, b: f32) -> f32 {
    if b < a { b } else { a }
}

/// C++: `CGameClient::ApplyPreInputs` (gameclient.cpp:2540): the other players'
/// `Sv_PreInput`s meant for a tick, as direct or predicted input.
struct PreInputs<'a> {
    /// C++: `cl_antiping_preinput`.
    enabled: bool,
    local: i32,
    ring: &'a [Vec<MsgSvPreInput>],
}

impl PreInputs<'_> {
    fn apply(&self, tick: i32, direct: bool, world: &mut GameWorld) {
        if !self.enabled {
            return;
        }
        for id in 0..MAX_CLIENTS {
            // C++ dummy branch not ported (it skips the dummy's id too)
            if world.character(id).is_none() || id as i32 == self.local {
                continue;
            }
            let Some(p) = pre_input(self.ring, id, tick) else {
                continue;
            };
            // convert preinput to input
            let input = ObjPlayerInput {
                direction: p.direction,
                target_x: p.target_x,
                target_y: p.target_y,
                jump: p.jump,
                fire: p.fire,
                hook: p.hook,
                wanted_weapon: p.wanted_weapon,
                next_weapon: p.next_weapon,
                prev_weapon: p.prev_weapon,
                ..Default::default()
            };
            if direct {
                world.on_direct_input(id, &input);
            } else {
                world.on_predicted_input(id, &input);
            }
        }
    }
}

/// C++: `m_aClients[id].m_aPreInputs[tick % 200]` if it was meant for `tick`.
fn pre_input(ring: &[Vec<MsgSvPreInput>], id: usize, tick: i32) -> Option<&MsgSvPreInput> {
    let p = ring
        .get(id)?
        .get(usize::try_from(tick % NUM_PRE_INPUTS as i32).ok()?)?;
    (p.intended_tick == tick).then_some(p)
}

/// C++: the `m_aPredPos`/`m_aPredTick` writes after a tick: every character's position.
fn store_pred_pos(
    pos: &mut [[Vec2; NUM_PRED_POS]],
    ticks: &mut [[i32; NUM_PRED_POS]],
    world: &GameWorld,
    tick: i32,
) {
    // ponytail: C++ indexes with a negative remainder for a negative tick (UB); ticks
    // from a server are positive
    let slot = tick.rem_euclid(NUM_PRED_POS as i32) as usize;
    for i in 0..MAX_CLIENTS {
        if let Some((_, core)) = world.character(i) {
            pos[i][slot] = core.pos;
            ticks[i][slot] = tick;
        }
    }
}
