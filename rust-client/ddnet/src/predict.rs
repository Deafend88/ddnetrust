//! Client prediction driven by server snapshots. C++: the parts of
//! `CGameClient::OnNewSnapshot`, `UpdatePrediction` and `OnPredict` (gameclient.cpp)
//! that move characters, with the default client config (`cl_predict 1`,
//! `cl_predict_freeze 1`, `cl_antiping 0`). Weapons are not predicted.
//!
//! Not ported yet: `Sv_TeamsState` (team membership: everyone is in team 0 unless a
//! snapshot says solo/super), `Sv_TuneParams` (default tuning), snapshot entities other
//! than doors, the dummy, and the rendering-only parts (smoothing, events).

use std::collections::BTreeMap;

use crate::net::client::Client;
use crate::physics::collision::Collision;
use crate::physics::core::{CharacterCore, WorldCore};
use crate::physics::teams::{MAX_CLIENTS, NUM_DDRACE_TEAMS, TEAM_FLOCK, TeamsCore};
use crate::physics::world::{GameWorld, WorldConfig, core_obj};
use crate::protocol::generated::{
    self as g, ObjCharacter, ObjCharacterCore, ObjDDNetCharacter, ObjDDNetLaser, ObjEntityEx,
    ObjGameInfoEx, ObjSwitchState,
};
use crate::protocol::snapshot::Item;

const TICK_SPEED: i32 = 50;
const TILE_SWITCHTIMEDOPEN: i32 = 22;
const TILE_SWITCHTIMEDCLOSE: i32 = 23;
const TILE_SWITCHOPEN: i32 = 24;
const TILE_SWITCHCLOSE: i32 = 25;

/// C++: `m_Snap.m_aCharacters[i]` of an active character (in this and the previous
/// snapshot), its core evolved to the snapshot tick.
#[derive(Clone, Copy, Debug)]
struct SnapChar {
    cur: ObjCharacter,
    ext: Option<ObjDDNetCharacter>,
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
pub struct Predictor<'a> {
    col: &'a Collision,
    world: GameWorld<'a>,
    teams: TeamsCore,
    highest_switch: i32,
    /// C++: `CCharOrder::m_Ids`.
    char_order: Vec<usize>,
    chars: Vec<Option<SnapChar>>,
    local_id: i32,
    /// C++: `m_PredictedChar` written as a net object, and `m_PredictedTick`.
    predicted_char: Option<ObjCharacterCore>,
    predicted_tick: i32,
    /// The latest prediction of the local character for each tick.
    history: BTreeMap<i32, ObjCharacterCore>,
    pub stats: Stats,
}

/// Pads a snapshot item to `n` ints (shorter items come from older servers).
fn ints(item: &Item, n: usize) -> Vec<i32> {
    let mut v = item.data.clone();
    v.resize(v.len().max(n), 0);
    v
}

fn find(items: &[Item], type_id: i32, id: i32) -> Option<&Item> {
    items.iter().find(|it| it.type_id == type_id && it.id == id)
}

impl<'a> Predictor<'a> {
    pub fn new(col: &'a Collision) -> Self {
        // C++: CGameClient::OnConnected / OnReset defaults
        let config = WorldConfig {
            infinite_ammo: true,
            predict_freeze: true,
            freeze_delay: 3,
            antiping_players: 1,
            ..Default::default()
        };
        Self {
            col,
            world: GameWorld::new(col, config),
            teams: TeamsCore::default(),
            highest_switch: col.highest_switch_number,
            char_order: (0..MAX_CLIENTS).collect(),
            chars: vec![None; MAX_CLIENTS],
            local_id: -1,
            predicted_char: None,
            predicted_tick: -1,
            history: BTreeMap::new(),
            stats: Stats::default(),
        }
    }

    /// Handles everything the client did since the last call: each snapshot switch
    /// (`OnNewSnapshot` + `UpdatePrediction`), then `OnPredict` if a repredict is due.
    pub fn update(&mut self, client: &mut Client) {
        for (prev, cur) in std::mem::take(&mut client.snapshot_switches) {
            self.on_new_snapshot(client, prev, cur);
        }
        if std::mem::take(&mut client.repredict) {
            self.predict(client);
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
            w.tick(0, self.col, &teams, false, true);
            w.move_(0, self.col, &teams);
            w.quantize(0);
        }
        let o = w.characters[0].as_ref().unwrap().write();
        (c.x, c.y, c.vel_x, c.vel_y, c.angle, c.direction, c.jumped) =
            (o.x, o.y, o.vel_x, o.vel_y, o.angle, o.direction, o.jumped);
        (c.hooked_player, c.hook_state, c.hook_tick) = (o.hooked_player, o.hook_state, o.hook_tick);
        (c.hook_x, c.hook_y, c.hook_dx, c.hook_dy) = (o.hook_x, o.hook_y, o.hook_dx, o.hook_dy);
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
        for it in items
            .iter()
            .filter(|it| it.type_id == g::NETOBJTYPE_CHARACTER)
        {
            let id = it.id as usize;
            if id >= MAX_CLIENTS || find(prev_items, g::NETOBJTYPE_CHARACTER, it.id).is_none() {
                continue;
            }
            let mut cur = ObjCharacter::from_ints(&ints(it, ObjCharacter::NUM_INTS));
            // ponytail: C++ reuses its previous evolve when the snapped object did not
            // change; evolving again from the snapped tick gives the same core.
            if tick - cur.tick <= 3 * TICK_SPEED && cur.tick != 0 {
                self.evolve(&mut cur, tick);
            }
            let ext = find(items, g::NETOBJTYPE_DDNETCHARACTER, it.id)
                .map(|e| ObjDDNetCharacter::from_ints(&ints(e, ObjDDNetCharacter::NUM_INTS)));
            if let Some(ext) = ext {
                self.teams
                    .set_solo(it.id, ext.flags & g::CHARACTERFLAG_SOLO != 0);
            }
            self.chars[id] = Some(SnapChar { cur, ext });
        }

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
        let no_weak = version >= 8 && flags2 & g::GAMEINFOFLAG2_NO_WEAK_HOOK != 0;
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

        for it in items
            .iter()
            .filter(|it| it.type_id == g::NETOBJTYPE_SWITCHSTATE)
        {
            if it.data.len() < 9 {
                continue;
            }
            self.read_switch_state(it);
        }

        let c = &mut self.world.config;
        c.predict_ddrace = predict_ddrace;
        c.predict_tiles = predict_ddrace && predict_tiles;
        c.bug_ddrace_input = bug_input;
        c.no_weak_hook_and_bounce = no_weak;
        c.use_tune_zones = predict_tiles; // C++: UpdateLocalTuning

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

        // strong and weak hook order (player collision and hooking are on by default)
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
            self.world.reinsert(self.char_order.iter().copied());
        }
        // ponytail: without the extended object C++ detects strong hooks by hand
        // (DetectStrongHook); DDNet servers always send it.

        // advance the server world to the snapshot tick
        let has_local = self.world.character(local).is_some();
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
        for (i, c) in self.chars.iter().enumerate() {
            // ponytail: characters without the extended object (vanilla servers) are
            // skipped, see `Character::read`
            if let Some(SnapChar {
                cur,
                ext: Some(ext),
            }) = c
            {
                self.world.net_char_add(i, cur, ext, i == local);
            }
        }
        // C++: SnapCollectEntities: entities sorted by id with their EntityEx; only
        // lasers matter here (doors)
        if let Some(items) = client.snapshot_at(tick) {
            let mut lasers: Vec<&Item> = items
                .iter()
                .filter(|it| [g::NETOBJTYPE_LASER, g::NETOBJTYPE_DDNETLASER].contains(&it.type_id))
                .collect();
            lasers.sort_by_key(|it| it.id);
            for it in lasers {
                let ex = find(items, g::NETOBJTYPE_ENTITYEX, it.id)
                    .map(|e| ObjEntityEx::from_ints(&ints(e, ObjEntityEx::NUM_INTS)));
                let data = ints(it, ObjDDNetLaser::NUM_INTS);
                self.world
                    .net_obj_add_laser(it.id, it.type_id, &data, ex.as_ref());
            }
        }
        self.world.net_obj_end();
    }

    /// C++: `CGameClient::IsOtherTeam` for a playing (not spectating) local player.
    fn is_other_team(&self, id: i32) -> bool {
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

        let mut w = self.world.clone();
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
