//! Votes (PLAN §13.7). C++: `CVoting` (game/client/components/voting.cpp): the running
//! vote and the server's option list from its messages. The vote notice and the Vote
//! tab draw it.

use ddnet::net::conn::FREQ;
use ddnet::protocol::generated::GameMsg;

use crate::config::str_copy;

/// C++ `VOTE_DESC_LENGTH`, `VOTE_REASON_LENGTH`, `MAX_VOTE_OPTIONS` (game/voting.h).
const DESC_LENGTH: usize = 64;
const REASON_LENGTH: usize = 16;
const MAX_OPTIONS: usize = 8192;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Voting {
    /// C++ `m_Opentime`, `m_Closetime` (`time_get()` units; 0: no vote).
    pub open_time: i64,
    pub close_time: i64,
    pub description: String,
    pub reason: String,
    /// C++ `m_Voted`: our vote (1 yes, -1 no, 0 none).
    pub voted: i32,
    pub yes: i32,
    pub no: i32,
    pub pass: i32,
    pub total: i32,
    pub receiving_options: bool,
    /// The server's vote options in list order.
    pub options: Vec<String>,
}

impl Voting {
    /// C++: `CVoting::OnReset` (the option list stays).
    pub fn on_reset(&mut self) {
        (self.open_time, self.close_time) = (0, 0);
        self.description.clear();
        self.reason.clear();
        (self.yes, self.no, self.pass, self.total) = (0, 0, 0, 0);
        self.voted = 0;
        self.receiving_options = false;
    }

    /// C++: `CVoting::IsVoting`.
    pub fn is_voting(&self) -> bool {
        self.close_time != 0
    }

    /// C++: `CVoting::SecondsLeft`.
    pub fn seconds_left(&self, now: i64) -> i32 {
        ((self.close_time - now) / FREQ) as i32
    }

    /// C++: `CVoting::AddOption`.
    fn add_option(&mut self, description: &str) {
        if self.options.len() == MAX_OPTIONS {
            return;
        }
        self.options.push(str_copy(description, DESC_LENGTH));
    }

    /// C++: `CVoting::RemoveOption`: the first option equal to `description` (the stored
    /// one is cut to 63 bytes, the message's is not).
    fn remove_option(&mut self, description: &str) {
        if let Some(i) = self.options.iter().position(|o| o == description) {
            self.options.remove(i);
        }
    }

    /// C++: `CVoting::OnMessage` (voting.cpp:250) at `now`. ponytail: the rcon-authed
    /// notification and sound for a new vote wait for rcon (13.9).
    pub fn on_message(&mut self, msg: &GameMsg, now: i64) {
        match msg {
            GameMsg::SvVoteSet(m) => {
                self.on_reset();
                if m.timeout != 0 {
                    self.description = str_copy(&m.description, DESC_LENGTH);
                    self.reason = str_copy(&m.reason, REASON_LENGTH);
                    self.open_time = now;
                    self.close_time = now + FREQ * m.timeout as i64;
                }
            }
            GameMsg::SvVoteStatus(m) => {
                (self.yes, self.no, self.pass, self.total) = (m.yes, m.no, m.pass, m.total);
            }
            GameMsg::SvVoteClearOptions(_) => self.options.clear(),
            GameMsg::SvVoteOptionListAdd(m) => {
                let all = [
                    &m.description0,
                    &m.description1,
                    &m.description2,
                    &m.description3,
                    &m.description4,
                    &m.description5,
                    &m.description6,
                    &m.description7,
                    &m.description8,
                    &m.description9,
                    &m.description10,
                    &m.description11,
                    &m.description12,
                    &m.description13,
                    &m.description14,
                ];
                // C++ loops to m_NumOptions; a switch adds only cases 0-14
                let n = m.num_options.clamp(0, all.len() as i32) as usize;
                for d in &all[..n] {
                    self.add_option(d);
                }
            }
            GameMsg::SvVoteOptionAdd(m) => self.add_option(&m.description),
            GameMsg::SvVoteOptionRemove(m) => self.remove_option(&m.description),
            GameMsg::SvYourVote(m) => self.voted = m.voted,
            GameMsg::SvVoteOptionGroupStart(_) => self.receiving_options = true,
            GameMsg::SvVoteOptionGroupEnd(_) => self.receiving_options = false,
            _ => {}
        }
    }
}
