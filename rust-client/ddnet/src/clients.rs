//! C++ `CGameClient::m_aClients` (`CClientData`) as far as chat and the scoreboard read
//! it, kept across snapshots like C++, and the player lists `OnNewSnapshot` sorts
//! (gameclient.cpp:1745-2225).

use std::cmp::Ordering;

use crate::map::ints_to_str;
use crate::net::client::valid_filename;
use crate::protocol::generated as g;
use crate::protocol::snapshot::Item;
use crate::text;

/// C++ `MAX_CLIENTS`.
pub const MAX_CLIENTS: usize = 128;
/// C++ `FinishTime::UNSET`, `NOT_FINISHED_TIMESCORE`, `NOT_FINISHED_MILLIS`.
pub const FINISH_UNSET: i32 = -2;
pub const NOT_FINISHED_TIMESCORE: i32 = -9999;
pub const NOT_FINISHED_MILLIS: i32 = -1;

/// C++ `CGameClient::CClientData` (the fields chat and the scoreboard use).
#[derive(Clone, Debug, PartialEq)]
pub struct ClientData {
    pub name: String,
    pub clan: String,
    pub country: i32,
    pub skin: String,
    pub use_custom_color: i32,
    pub color_body: i32,
    pub color_feet: i32,
    pub team: i32,
    pub active: bool,
    pub chat_ignore: bool,
    pub friend: bool,
    pub foe: bool,
    pub auth_level: i32,
    pub afk: bool,
    pub paused: bool,
    pub spec: bool,
    pub finish_time_seconds: i32,
    pub finish_time_millis: i32,
}

impl Default for ClientData {
    /// C++: `CClientData::Reset` (gameclient.cpp:2989); finish times keep the values a
    /// value-initialised client starts with (Reset does not touch them).
    fn default() -> Self {
        Self {
            name: String::new(),
            clan: String::new(),
            country: -1,
            skin: "default".into(),
            use_custom_color: 0,
            color_body: 0,
            color_feet: 0,
            team: 0,
            active: false,
            chat_ignore: false,
            friend: false,
            foe: false,
            auth_level: 0,
            afk: false,
            paused: false,
            spec: false,
            finish_time_seconds: 0,
            finish_time_millis: 0,
        }
    }
}

/// One `PlayerInfo` of the current snapshot (C++ `m_Snap.m_apPlayerInfos[id]`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerInfo {
    pub local: bool,
    pub client_id: i32,
    pub team: i32,
    pub score: i32,
    pub latency: i32,
}

/// C++ `CFriends`: `(name, clan)` entries from `add_friend` / `add_foe`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Friends {
    pub list: Vec<(String, String)>,
}

impl Friends {
    /// C++ `MAX_FRIENDS`.
    const MAX: usize = 1024;

    /// C++: `CFriends::IsFriend` (friends.cpp:94). The hashes only speed up the string
    /// comparisons, so they are left out.
    pub fn is_friend(&self, name: &str, clan: &str, players_only: bool, ignore_clan: bool) -> bool {
        self.list.iter().any(|(n, c)| {
            ((ignore_clan && !n.is_empty()) || c == clan)
                && ((!players_only && n.is_empty()) || n == name)
        })
    }

    /// C++: `CFriends::AddFriend` (friends.cpp:107): name and clan cut to their
    /// buffers (16 and 12 bytes).
    pub fn add(&mut self, name: &str, clan: &str) {
        if self.list.len() == Self::MAX || (name.is_empty() && clan.is_empty()) {
            return;
        }
        let cut = |s: &str, size| {
            String::from_utf8_lossy(&text::truncate(s.as_bytes(), size)).into_owned()
        };
        let (name, clan) = (cut(name, 16), cut(clan, 12));
        if !self.list.iter().any(|(n, c)| *n == name && *c == clan) {
            self.list.push((name, clan));
        }
    }

    /// C++: `CFriends::RemoveFriend`.
    pub fn remove(&mut self, name: &str, clan: &str) {
        self.list.retain(|(n, c)| !(n == name && c == clan));
    }
}

/// What `OnNewSnapshot` needs besides the snapshot.
pub struct SnapContext<'a> {
    /// C++ `m_GameInfo.m_AllowXSkins`.
    pub allow_x_skins: bool,
    pub friends: &'a Friends,
    pub foes: &'a Friends,
    /// C++ `cl_friends_ignore_clan`.
    pub friends_ignore_clan: bool,
}

/// C++: `m_aClients`, the `m_Snap` player lists and the finish-time flags.
#[derive(Clone, Debug)]
pub struct Clients {
    pub clients: Vec<ClientData>,
    /// C++ `m_Snap.m_apPlayerInfos`.
    pub infos: Vec<Option<PlayerInfo>>,
    /// C++ `m_Snap.m_LocalClientId`.
    pub local_id: i32,
    /// C++ `m_ReceivedDDNetPlayer` (reset only on a new connection).
    pub received_ddnet_player: bool,
    pub received_finish_times: bool,
    pub received_finish_times_millis: bool,
    /// C++ `m_Snap.m_apInfoByName`, `m_apInfoByScore`: client ids, players first.
    pub by_name: Vec<i32>,
    pub by_score: Vec<i32>,
}

impl Default for Clients {
    fn default() -> Self {
        Self {
            clients: vec![ClientData::default(); MAX_CLIENTS],
            infos: vec![None; MAX_CLIENTS],
            local_id: -1,
            received_ddnet_player: false,
            received_finish_times: false,
            received_finish_times_millis: false,
            by_name: Vec::new(),
            by_score: Vec::new(),
        }
    }
}

/// C++: `CSkin::IsValidName` and `CSkins::IsSpecialSkin`.
fn skin_ok(skin: &str, allow_x_skins: bool) -> bool {
    skin.len() < 24
        && valid_filename(skin)
        && (allow_x_skins || !text::startswith_nocase(skin.as_bytes(), b"x_"))
}

impl Clients {
    /// C++: the client parts of `CGameClient::OnNewSnapshot` (gameclient.cpp:1745-2172).
    /// `time_score`: C++ `m_GameInfo.m_TimeScore`.
    pub fn on_new_snapshot(&mut self, items: &[Item], ctx: &SnapContext, time_score: bool) {
        self.infos.fill(None);
        self.local_id = -1;
        let (mut unset_finish, mut millis) = (false, false);
        for it in items {
            let id = it.id as usize;
            match it.type_id {
                g::NETOBJTYPE_CLIENTINFO if id < MAX_CLIENTS => {
                    let info = g::ObjClientInfo::from_ints(&it.data);
                    let c = &mut self.clients[id];
                    c.name = ints_to_str(&info.name).unwrap_or_else(|| "nameless tee".into());
                    c.clan = ints_to_str(&info.clan).unwrap_or_default();
                    c.country = if (-1..=999).contains(&info.country) {
                        info.country
                    } else {
                        -1
                    };
                    c.skin = ints_to_str(&info.skin).unwrap_or_default();
                    if !skin_ok(&c.skin, ctx.allow_x_skins) {
                        c.skin = "default".into();
                    }
                    c.use_custom_color = info.use_custom_color;
                    c.color_body = info.color_body;
                    c.color_feet = info.color_feet;
                }
                g::NETOBJTYPE_PLAYERINFO => {
                    let p = g::ObjPlayerInfo::from_ints(&it.data);
                    if (0..MAX_CLIENTS as i32).contains(&p.client_id) && p.client_id == it.id {
                        let c = &mut self.clients[id];
                        c.team = p.team;
                        c.active = true;
                        self.infos[id] = Some(PlayerInfo {
                            local: p.local != 0,
                            client_id: p.client_id,
                            team: p.team,
                            score: p.score,
                            latency: p.latency,
                        });
                        if p.local != 0 {
                            self.local_id = p.client_id;
                        }
                    }
                }
                g::NETOBJTYPE_DDNETPLAYER => {
                    self.received_ddnet_player = true;
                    if id < MAX_CLIENTS {
                        let p = g::ObjDDNetPlayer::from_ints(&it.data);
                        let c = &mut self.clients[id];
                        c.auth_level = p.auth_level;
                        c.afk = p.flags & g::EXPLAYERFLAG_AFK != 0;
                        c.paused = p.flags & g::EXPLAYERFLAG_PAUSED != 0;
                        c.spec = p.flags & g::EXPLAYERFLAG_SPEC != 0;
                        c.finish_time_seconds = p.finish_time_seconds;
                        c.finish_time_millis = p.finish_time_millis;
                        if c.finish_time_seconds == FINISH_UNSET {
                            unset_finish = true;
                        } else if c.finish_time_seconds < 60 * 60 && c.finish_time_millis % 10 != 0
                        {
                            millis = true;
                        }
                    }
                }
                _ => {}
            }
        }
        // C++: clear out unneeded client data
        for (c, info) in self.clients.iter_mut().zip(&self.infos) {
            if info.is_none() && c.active {
                *c = ClientData::default();
            }
        }
        for i in 0..MAX_CLIENTS {
            let c = &self.clients[i];
            let other = i as i32 != self.local_id && self.infos[i].is_some();
            let (friend, foe) = (
                other && (ctx.friends).is_friend(&c.name, &c.clan, true, ctx.friends_ignore_clan),
                other && (ctx.foes).is_friend(&c.name, &c.clan, true, ctx.friends_ignore_clan),
            );
            let c = &mut self.clients[i];
            (c.friend, c.foe) = (friend, foe);
        }
        self.received_finish_times = self.received_ddnet_player && !unset_finish;
        self.received_finish_times_millis = self.received_ddnet_player && millis;
        self.sort(time_score);
    }

    /// C++: the sorts at gameclient.cpp:2174-2200 (stable sorts over the 128 slots, empty
    /// slots last; only the players are kept here).
    fn sort(&mut self, time_score: bool) {
        let names: Vec<&[u8]> = self.clients.iter().map(|c| c.name.as_bytes()).collect();
        self.by_name = (0..MAX_CLIENTS as i32)
            .filter(|&i| self.infos[i as usize].is_some())
            .collect();
        self.by_name
            .sort_by(|&a, &b| text::comp_nocase(names[a as usize], names[b as usize]));
        let finish = self.received_finish_times;
        let key = |i: i32| {
            let c = &self.clients[i as usize];
            if finish {
                (c.finish_time_seconds, c.finish_time_millis)
            } else {
                (self.infos[i as usize].map_or(0, |p| p.score), 0)
            }
        };
        let mut by_score = self.by_name.clone();
        by_score.sort_by(|&a, &b| {
            let (ka, kb) = (key(a), key(b));
            if score_less(time_score, finish, ka, kb) {
                Ordering::Less
            } else if score_less(time_score, finish, kb, ka) {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        });
        self.by_score = by_score;
    }

    /// C++ `m_Snap.m_apInfoByDDTeamScore`/`ByDDTeamName`: `list` grouped by DDRace team
    /// (`team(id)`, C++ `m_Teams.Team`), in team order 0..64.
    pub fn by_ddteam(list: &[i32], team: impl Fn(i32) -> i32) -> Vec<i32> {
        let mut out = Vec::with_capacity(list.len());
        for t in 0..crate::physics::teams::NUM_DDRACE_TEAMS as i32 {
            out.extend(list.iter().copied().filter(|&i| team(i) == t));
        }
        out
    }
}

/// C++: `CGameClient::GetScoreComparator(TimeScore, m_ReceivedDDNetPlayerFinishTimes,
/// false)` (gameclient.cpp:2479) applied to (score or finish seconds, finish millis):
/// whether `a` sorts before `b`. As in C++, without `time_score` even received finish
/// seconds are compared as scores (bigger first).
pub fn score_less(time_score: bool, finish_times: bool, a: (i32, i32), b: (i32, i32)) -> bool {
    if !time_score {
        return a.0 > b.0;
    }
    if !finish_times {
        let fix = |t: i32| {
            if t == NOT_FINISHED_TIMESCORE {
                i32::MIN
            } else {
                t
            }
        };
        return fix(a.0) > fix(b.0);
    }
    let fix = |t: i32| {
        if t == NOT_FINISHED_MILLIS {
            i32::MAX
        } else {
            t
        }
    };
    let (s1, s2) = (fix(a.0), fix(b.0));
    if s1 == s2 {
        return a.1 < b.1;
    }
    s1 < s2
}
