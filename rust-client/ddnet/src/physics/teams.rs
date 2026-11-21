//! Team membership as the physics sees it. C++: `CTeamsCore` (`src/game/teamscore.cpp`).

pub const MAX_CLIENTS: usize = 128;
pub const TEAM_FLOCK: i32 = 0;
pub const TEAM_SUPER: i32 = MAX_CLIENTS as i32;
pub const NUM_DDRACE_TEAMS: usize = TEAM_SUPER as usize + 1;

/// C++: `CTeamsCore`.
#[derive(Clone, Debug)]
pub struct TeamsCore {
    team: [i32; MAX_CLIENTS],
    is_solo: [bool; MAX_CLIENTS],
    pub num_ddrace_teams: i32,
}

impl Default for TeamsCore {
    /// C++: `CTeamsCore::Reset` with `sv_team` not forced-solo (client default).
    fn default() -> Self {
        Self {
            team: [TEAM_FLOCK; MAX_CLIENTS],
            is_solo: [false; MAX_CLIENTS],
            num_ddrace_teams: NUM_DDRACE_TEAMS as i32,
        }
    }
}

impl TeamsCore {
    pub fn team_super(&self) -> i32 {
        self.num_ddrace_teams - 1
    }

    pub fn team(&self, id: i32) -> i32 {
        self.team[id as usize]
    }

    pub fn set_team(&mut self, id: i32, team: i32) {
        assert!(
            (TEAM_FLOCK..NUM_DDRACE_TEAMS as i32).contains(&team),
            "Invalid Team: {team}"
        );
        self.team[id as usize] = team;
    }

    pub fn set_solo(&mut self, id: i32, value: bool) {
        self.is_solo[id as usize] = value;
    }

    /// C++: `CTeamsCore::GetSolo`.
    pub fn is_solo(&self, id: i32) -> bool {
        self.is_solo[id as usize]
    }

    pub fn same_team(&self, a: i32, b: i32) -> bool {
        let (ta, tb) = (self.team(a), self.team(b));
        ta == self.team_super() || tb == self.team_super() || ta == tb
    }

    pub fn can_keep_hook(&self, a: i32, b: i32) -> bool {
        let (ta, tb) = (self.team(a), self.team(b));
        if ta == self.team_super() || tb == self.team_super() || a == b {
            return true;
        }
        ta == tb
    }

    pub fn can_collide(&self, a: i32, b: i32) -> bool {
        let (ta, tb) = (self.team(a), self.team(b));
        if ta == self.team_super() || tb == self.team_super() || a == b {
            return true;
        }
        if self.is_solo[a as usize] || self.is_solo[b as usize] {
            return false;
        }
        ta == tb
    }
}
