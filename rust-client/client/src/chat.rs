//! Chat (PLAN §13.1). `Chat` is a port of the logic of C++ `CChat`
//! (game/client/components/chat.cpp): the 64-line ring with its filters, repeat folding
//! and highlights (`AddLine`), the input with name and command completion and history
//! (`OnInput`), and the send queue (`SendChatQueued`). Text is edited by an egui text
//! field; the drawing is ours (`show`). Checked against copies of the C++ code by
//! `rs_parity chat` (tests/parity.rs `chat_matches_cpp`).

use std::collections::VecDeque;

use ddnet::clients::Clients;
use ddnet::protocol::generated::GameMsg;
use ddnet::protocol::packer::{utf8_decode, utf8_isspace, utf8_skip_whitespaces};
use ddnet::text::{self, at};

/// C++ `CChat::MAX_LINES`, `MAX_CHAT_LENGTH`.
pub const MAX_LINES: usize = 64;
pub const MAX_CHAT_LENGTH: usize = 256;
/// C++ client ids of special lines.
pub const SERVER_MSG: i32 = -1;
pub const CLIENT_MSG: i32 = -2;
/// C++ `TEAM_*` and the whisper "teams" of `Sv_Chat`.
pub const TEAM_SPECTATORS: i32 = -1;
pub const TEAM_RED: i32 = 0;
pub const TEAM_BLUE: i32 = 1;
pub const TEAM_WHISPER_SEND: i32 = 2;
pub const TEAM_WHISPER_RECV: i32 = 3;
/// C++ `time_freq()`: `now` is in nanoseconds.
const FREQ: i64 = 1_000_000_000;
/// C++: `CStaticRingBuffer<CHistoryEntry, 64 * 1024>`.
const HISTORY_BYTES: usize = 64 * 1024;

/// C++ `CChat::CLine`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Line {
    pub initialized: bool,
    pub time: i64,
    pub client_id: i32,
    pub team_number: i32,
    pub team: bool,
    pub whisper: bool,
    /// C++ `m_NameColor`: -2 none, or `TEAM_SPECTATORS/RED/BLUE`.
    pub name_color: i32,
    pub name: String,
    pub text: String,
    pub friend: bool,
    pub highlighted: bool,
    /// C++ `m_CustomColor`: the packed `cl_message_client_color` for client lines.
    pub custom_color: Option<u32>,
    pub times_repeated: i32,
    /// The author's tee (C++ `m_pManagedTeeRenderInfo`), when the author was active.
    pub tee: Option<crate::teehead::TeeLook>,
}

/// C++ `MODE_NONE`, `MODE_ALL`, `MODE_TEAM`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    None,
    All,
    Team,
}

/// The C++ chat sounds (`CHAT_SERVER`, `CHAT_HIGHLIGHT`, `CHAT_CLIENT`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    Server,
    Highlight,
    Client,
}

/// The keys `CChat::OnInput` looks at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Escape,
    Enter,
    Tab,
    Up,
    Down,
    /// `KEY_LSHIFT` / `KEY_RSHIFT`.
    Shift,
    Other,
}

/// C++ `CChat::CCommand` (a server chat command from `Sv_CommandInfo`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub name: String,
    pub params: String,
    pub help: String,
}

/// The input line (C++ `CLineInputBuffered<MAX_CHAT_LENGTH>`: at most 255 bytes, byte
/// cursor). `changed` tells the text field to take text and cursor from here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineInput {
    pub text: String,
    pub cursor: usize,
    pub changed: bool,
}

impl LineInput {
    /// C++: `CLineInput::Set`: the text (cut like `str_copy`), cursor at the end.
    pub fn set(&mut self, s: &str) {
        self.text = c_str(s.as_bytes(), MAX_CHAT_LENGTH);
        self.set_cursor(self.text.len());
    }

    /// C++: `CLineInput::SetCursorOffset` (clamped to the text).
    pub fn set_cursor(&mut self, offset: usize) {
        self.cursor = offset.min(self.text.len());
        self.changed = true;
    }

    pub fn clear(&mut self) {
        self.set("");
    }
}

/// The config values `CChat` reads.
#[derive(Clone, Copy, Debug)]
pub struct ChatConfig {
    pub show_chat_system: bool,
    pub show_chat_friends: bool,
    pub show_chat_team_members_only: bool,
    pub chat_reset: bool,
    pub snd_server_message: bool,
    pub snd_highlight: bool,
    pub snd_team_chat: bool,
    pub snd_chat: bool,
    /// Packed HSL `cl_message_client_color`.
    pub message_client_color: u32,
}

impl ChatConfig {
    pub fn new(c: &crate::config::Config) -> Self {
        Self {
            show_chat_system: c.cl_show_chat_system != 0,
            show_chat_friends: c.cl_show_chat_friends != 0,
            show_chat_team_members_only: c.cl_show_chat_team_members_only != 0,
            chat_reset: c.cl_chat_reset != 0,
            snd_server_message: c.snd_servermessage != 0,
            snd_highlight: c.snd_highlight != 0,
            snd_team_chat: c.snd_team_chat != 0,
            snd_chat: c.snd_chat != 0,
            message_client_color: c.cl_message_client_color,
        }
    }
}

/// What `AddLine` reads from the game client.
pub struct Ctx<'a> {
    /// C++ `m_aClients`, `m_Snap.m_LocalClientId`, `m_Snap.m_apInfoByName`.
    pub clients: &'a Clients,
    /// C++ `m_aLocalIds` (main, dummy).
    pub local_ids: [i32; 2],
    /// C++ `m_Teams.Team(id)` and `IsOtherTeam(id)`.
    pub team: &'a dyn Fn(i32) -> i32,
    pub is_other_team: &'a dyn Fn(i32) -> bool,
    /// C++ `IsTeamPlay()`.
    pub team_play: bool,
    /// C++ `Client()->State() == STATE_DEMOPLAYBACK`.
    pub demo: bool,
    pub config: ChatConfig,
}

/// C++ `CChat` (logic).
#[derive(Clone, Debug)]
pub struct Chat {
    pub lines: Vec<Line>,
    pub current: usize,
    pub mode: Mode,
    /// C++ `m_Show` (`+show_chat`).
    pub show: bool,
    pub input: LineInput,
    completion_used: bool,
    completion_chosen: i32,
    completion_buffer: Vec<u8>,
    placeholder_offset: usize,
    placeholder_length: usize,
    /// (client id, score) of the name completion (C++ `m_aPlayerCompletionList`).
    player_completion: Vec<(i32, i32)>,
    pub commands: Vec<Command>,
    commands_need_sorting: bool,
    supports_command_info: bool,
    /// C++ `m_History`: (team, text), oldest first.
    history: VecDeque<(i32, String)>,
    history_bytes: usize,
    /// C++ `m_pHistoryEntry`: an index into `history`.
    history_pos: Option<usize>,
    pending: i32,
    last_send: i64,
    last_sound: [i64; 3],
    current_input_text: String,
    editing_new_line: bool,
    /// Chat lines to send (team, text), drained by the caller (C++ `Cl_Say`).
    pub sends: Vec<(i32, String)>,
    /// Sounds to play (sound comes with PLAN §13.3), drained by the caller.
    pub sounds: Vec<Sound>,
}

impl Default for Chat {
    /// C++: `CChat::Reset`.
    fn default() -> Self {
        Self {
            lines: vec![Line::default(); MAX_LINES],
            current: 0,
            mode: Mode::None,
            show: false,
            input: LineInput::default(),
            completion_used: false,
            completion_chosen: -1,
            completion_buffer: Vec::new(),
            placeholder_offset: 0,
            placeholder_length: 0,
            player_completion: Vec::new(),
            commands: Vec::new(),
            commands_need_sorting: false,
            supports_command_info: false,
            history: VecDeque::new(),
            history_bytes: 0,
            history_pos: None,
            pending: 0,
            last_send: 0,
            last_sound: [0; 3],
            current_input_text: String::new(),
            editing_new_line: true,
            sends: Vec::new(),
            sounds: Vec::new(),
        }
    }
}

/// C++: `str_copy` into a `size`-byte buffer, as text.
fn c_str(s: &[u8], size: usize) -> String {
    String::from_utf8_lossy(&text::truncate(s, size)).into_owned()
}

/// C++: `str_append(dst, src, MAX_CHAT_LENGTH)`.
fn append(dst: &mut Vec<u8>, src: &[u8]) {
    dst.extend_from_slice(src);
    *dst = text::truncate(dst, MAX_CHAT_LENGTH);
}

impl Chat {
    pub fn is_active(&self) -> bool {
        self.mode != Mode::None
    }

    /// C++: `CChat::Reset` (on connecting): everything but the input text and the sent
    /// history.
    pub fn reset(&mut self) {
        let (input, history, history_bytes) = (
            std::mem::take(&mut self.input),
            std::mem::take(&mut self.history),
            self.history_bytes,
        );
        *self = Self {
            input,
            history,
            history_bytes,
            ..Self::default()
        };
    }

    /// The completion and queue state the parity test compares: completion used,
    /// chosen, placeholder offset and length, pending sends.
    pub fn state(&self) -> (bool, i32, usize, usize, i32) {
        (
            self.completion_used,
            self.completion_chosen,
            self.placeholder_offset,
            self.placeholder_length,
            self.pending,
        )
    }

    /// C++: `CChat::RegisterCommand` (chat.cpp:88).
    fn register_command(&mut self, name: &str, params: &str, help: &str) {
        if self.commands.iter().any(|c| c.name == name) {
            return;
        }
        self.commands.push(Command {
            name: name.into(),
            params: params.into(),
            help: help.into(),
        });
        self.commands_need_sorting = true;
    }

    /// C++: `CChat::EnableMode` (chat.cpp:541).
    pub fn enable_mode(&mut self, team: i32, demo: bool) {
        if demo {
            return;
        }
        if self.mode == Mode::None {
            self.mode = if team != 0 { Mode::Team } else { Mode::All };
            self.completion_chosen = -1;
            self.completion_used = false;
        }
    }

    /// C++: `CChat::DisableMode` (chat.cpp:559).
    pub fn disable_mode(&mut self) {
        self.mode = Mode::None;
    }

    /// C++: the `chat` console command (chat.cpp:176): `all`/`team` and an optional text.
    pub fn con_chat(&mut self, args: &[String], chat_reset: bool, demo: bool) {
        match args.first().map(String::as_str) {
            Some("all") => self.enable_mode(0, demo),
            Some("team") => self.enable_mode(1, demo),
            _ => eprintln!("chat: expected all or team as mode"),
        }
        let text = args.get(1).map_or("", String::as_str);
        if !text.is_empty() || chat_reset {
            self.input.set(text);
        }
    }

    /// C++: `CChat::OnMessage` (chat.cpp:568). Returns the server line `StoreSave` gets.
    pub fn on_message(&mut self, msg: &GameMsg, ctx: &Ctx, now: i64) -> Option<String> {
        match msg {
            GameMsg::SvChat(m) => {
                let mut line = m.message.clone().into_bytes();
                self.add_line(m.client_id, m.team, &mut line, ctx, now);
                (!ctx.demo && m.client_id == SERVER_MSG)
                    .then(|| String::from_utf8_lossy(&line).into_owned())
            }
            GameMsg::SvCommandInfo(m) => {
                if !self.supports_command_info {
                    self.commands.clear();
                    self.supports_command_info = true;
                }
                self.register_command(&m.name, &m.args_format, &m.help_text);
                None
            }
            GameMsg::SvCommandInfoRemove(m) => {
                self.commands.retain(|c| c.name != m.name);
                None
            }
            _ => None,
        }
    }

    /// C++: `CChat::Echo`: a client line.
    pub fn echo(&mut self, s: &str, ctx: &Ctx, now: i64) {
        self.add_line(CLIENT_MSG, 0, &mut s.as_bytes().to_vec(), ctx, now);
    }

    /// C++: `CChat::ClearLines`.
    pub fn clear_lines(&mut self) {
        self.lines.fill(Line::default());
    }

    /// C++: `CChat::LineShouldHighlight` (chat.cpp:614).
    pub fn line_should_highlight(line: &[u8], name: &[u8]) -> bool {
        let mut hit = text::utf8_find_nocase(line, name);
        while let Some(h) = hit {
            let len = name.len();
            let after = at(line, h + len);
            if len > 0
                && (h == 0 || line[h - 1] == b' ')
                && matches!(after, 0 | b' ' | b'.' | b'!' | b',' | b'?' | b':')
            {
                return true;
            }
            hit = text::utf8_find_nocase(&line[h + 1..], name).map(|o| o + h + 1);
        }
        false
    }

    /// C++: `CChat::AddLine` (chat.cpp:680). `line` is trimmed in place, like C++ does
    /// to the message it is given.
    pub fn add_line(&mut self, client_id: i32, team: i32, line: &mut Vec<u8>, ctx: &Ctx, now: i64) {
        let cfg = &ctx.config;
        let clients = &ctx.clients.clients;
        let local = ctx.clients.local_id;
        if at(line, 0) == 0
            || (client_id == SERVER_MSG && !cfg.show_chat_system)
            || (client_id >= 0 && {
                let c = &clients[client_id as usize];
                c.name.is_empty()
                    || c.chat_ignore
                    || (local != client_id && cfg.show_chat_friends && !c.friend)
                    || (local != client_id
                        && cfg.show_chat_team_members_only
                        && (ctx.is_other_team)(client_id)
                        && (ctx.team)(local) != 0)
                    || (local != client_id && c.foe)
            })
        {
            return;
        }

        // trim right and set maximum length to 256 utf8-characters
        let (mut length, mut pos, mut end) = (0, 0, None);
        while at(line, pos) != 0 {
            let old = pos;
            let code = utf8_decode(line, &mut pos);
            if !utf8_isspace(code) {
                end = None;
            } else if end.is_none() {
                end = Some(old);
            }
            length += 1;
            if length >= MAX_CHAT_LENGTH {
                line.truncate(pos);
                break;
            }
        }
        if let Some(e) = end {
            line.truncate(e);
        }
        if line.is_empty() {
            return;
        }

        let mut highlighted = false;
        let custom_color = (client_id == CLIENT_MSG).then_some(cfg.message_client_color);
        let prev = &mut self.lines[self.current];
        if prev.initialized
            && prev.team_number == team
            && prev.client_id == client_id
            && prev.text.as_bytes() == line.as_slice()
            && prev.custom_color == custom_color
        {
            prev.times_repeated += 1;
            prev.time = now;
            log_line(prev);
            return;
        }

        self.current = (self.current + 1) % MAX_LINES;
        let mut cur = Line {
            initialized: true,
            time: now,
            client_id,
            team_number: team,
            team: team == 1,
            whisper: team >= 2,
            name_color: -2,
            custom_color,
            ..Line::default()
        };

        // check for highlighted name
        if !ctx.demo {
            if client_id >= 0 && client_id != ctx.local_ids[0] && client_id != ctx.local_ids[1] {
                for id in ctx.local_ids {
                    highlighted |= id >= 0
                        && Self::line_should_highlight(line, clients[id as usize].name.as_bytes());
                }
            }
        } else {
            highlighted |= local >= 0
                && Self::line_should_highlight(line, clients[local as usize].name.as_bytes());
        }
        cur.highlighted = highlighted;
        cur.text = c_str(line, MAX_CHAT_LENGTH);

        if client_id == SERVER_MSG {
            cur.name = "*** ".into();
        } else if client_id == CLIENT_MSG {
            cur.name = "— ".into();
        } else {
            let author = &clients[client_id as usize];
            if author.active {
                if author.team == TEAM_SPECTATORS {
                    cur.name_color = TEAM_SPECTATORS;
                }
                if ctx.team_play {
                    if author.team == TEAM_RED {
                        cur.name_color = TEAM_RED;
                    } else if author.team == TEAM_BLUE {
                        cur.name_color = TEAM_BLUE;
                    }
                }
            }
            if team == TEAM_WHISPER_SEND {
                cur.name = "→".into();
                if author.active {
                    cur.name = c_str(format!("→ {}", author.name).as_bytes(), 64);
                }
                cur.name_color = TEAM_BLUE;
                cur.highlighted = false;
                highlighted = false;
            } else if team == TEAM_WHISPER_RECV {
                cur.name = "←".into();
                if author.active {
                    cur.name = c_str(format!("← {}", author.name).as_bytes(), 64);
                }
                cur.name_color = TEAM_RED;
                cur.highlighted = true;
                highlighted = true;
            } else {
                cur.name = author.name.clone();
            }
            if author.active {
                cur.friend = author.friend;
                cur.tee = Some(crate::teehead::TeeLook {
                    skin: author.skin.clone(),
                    custom_color: author.use_custom_color != 0,
                    color_body: author.color_body,
                });
            }
        }
        log_line(&cur);
        let cur_team = cur.team;
        self.lines[self.current] = cur;

        // play sound (C++ also shows a desktop notification on a highlight: not ported)
        let mut play = |kind: Sound, on: bool, last: &mut [i64; 3]| {
            if now - last[kind as usize] >= FREQ * 3 / 10 && on {
                self.sounds.push(kind);
                last[kind as usize] = now;
            }
        };
        if client_id == SERVER_MSG {
            play(Sound::Server, cfg.snd_server_message, &mut self.last_sound);
        } else if client_id == CLIENT_MSG {
            // No sound yet
        } else if highlighted && !ctx.demo {
            play(Sound::Highlight, cfg.snd_highlight, &mut self.last_sound);
        } else if team != TEAM_WHISPER_SEND {
            let on = if cur_team {
                cfg.snd_team_chat
            } else {
                cfg.snd_chat
            };
            play(Sound::Client, on, &mut self.last_sound);
        }
    }

    /// C++: `CChat::SendChat` (chat.cpp:1363).
    pub fn send_chat(&mut self, team: i32, line: &str, now: i64) {
        if utf8_skip_whitespaces(line.as_bytes()) == line.len() {
            return;
        }
        self.last_send = now;
        self.sends.push((team, line.into()));
    }

    /// C++: `CChat::SendChatQueued` (chat.cpp:1388).
    fn send_chat_queued(&mut self, line: String, now: i64) {
        if line.is_empty() {
            return;
        }
        let team = i32::from(self.mode != Mode::All);
        let add = if self.last_send + FREQ < now {
            self.send_chat(team, &line, now);
            true
        } else if self.pending < 3 {
            self.pending += 1;
            true
        } else {
            false
        };
        if add {
            // ponytail: recycles by a byte budget like the 64 KiB ring (entry + its
            // 24-byte block header, rounded to 24), not the ring's exact block layout;
            // they differ only after ~64 KiB of sent chat (which old lines Up reaches).
            let cost = |s: &str| (8 + s.len() + 24).div_ceil(24) * 24;
            self.history_bytes += cost(&line);
            self.history.push_back((team, line));
            while self.history_bytes > HISTORY_BYTES / 24 * 24 {
                if let Some((_, old)) = self.history.pop_front() {
                    self.history_bytes -= cost(&old);
                    self.history_pos = self.history_pos.and_then(|p| p.checked_sub(1));
                }
            }
        }
    }

    /// C++: the queued send at the start of `CChat::OnRender` (chat.cpp:1179).
    pub fn on_render(&mut self, now: i64) {
        if self.pending > 0 && self.last_send + FREQ < now {
            let n = self.history.len();
            let i = self.pending as usize - 1;
            if i < n {
                let (team, text) = self.history[n - 1 - i].clone();
                self.send_chat(team, &text, now);
            }
            self.pending -= 1;
        }
    }

    /// C++: `CChat::OnInput` (chat.cpp:271) for a key press while the chat is active.
    /// Typing itself goes to the text field. Returns true when C++ calls
    /// `GameClient()->OnRelease()` (the chat closed).
    pub fn on_key(&mut self, key: Key, shift: bool, ctx: &Ctx, chat_reset: bool, now: i64) -> bool {
        if self.mode == Mode::None {
            return false;
        }
        let mut released = false;
        match key {
            Key::Escape => {
                self.disable_mode();
                released = true;
                if chat_reset {
                    self.input.clear();
                    self.history_pos = None;
                    self.editing_new_line = true;
                }
            }
            Key::Enter => {
                if self.commands_need_sorting {
                    self.commands
                        .sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
                    self.commands_need_sorting = false;
                }
                let line = self.input.text.clone();
                self.send_chat_queued(line, now);
                self.history_pos = None;
                self.editing_new_line = true;
                self.disable_mode();
                released = true;
                self.input.clear();
            }
            _ => {}
        }
        if released {
            self.show = false; // C++ CChat::OnRelease
        }
        if key == Key::Tab {
            self.complete(shift, ctx);
        } else if key != Key::Shift {
            // reset name completion process
            self.completion_chosen = -1;
            self.completion_used = false;
        }

        if key == Key::Up {
            if self.editing_new_line {
                self.current_input_text = self.input.text.clone();
                self.editing_new_line = false;
            }
            match self.history_pos {
                Some(p) => {
                    if p > 0 {
                        self.history_pos = Some(p - 1);
                    }
                }
                None => self.history_pos = self.history.len().checked_sub(1),
            }
            if let Some(p) = self.history_pos {
                let text = self.history[p].1.clone();
                self.input.set(&text);
            }
        } else if key == Key::Down {
            if let Some(p) = self.history_pos {
                self.history_pos = (p + 1 < self.history.len()).then_some(p + 1);
            }
            if let Some(p) = self.history_pos {
                let text = self.history[p].1.clone();
                self.input.set(&text);
            } else if !self.editing_new_line {
                let text = self.current_input_text.clone();
                self.input.set(&text);
                self.editing_new_line = true;
            }
        }
        released
    }

    /// C++: the Tab branch of `CChat::OnInput` (chat.cpp:302-486).
    fn complete(&mut self, shift: bool, ctx: &Ctx) {
        let s = self.input.text.clone().into_bytes();
        // fill the completion buffer
        if !self.completion_used {
            let mut p = self.input.cursor.min(s.len());
            while p > 0 && s[p - 1] != b' ' {
                p -= 1;
            }
            self.placeholder_offset = p;
            self.placeholder_length = s[p..].iter().take_while(|&&c| c != b' ').count();
            let n = self.placeholder_length;
            self.completion_buffer = text::truncate(&s[p..p + n], (n + 1).min(MAX_CHAT_LENGTH));
        }

        if !self.completion_used && at(&self.completion_buffer, 0) != b'/' {
            // Create the completion list of player names through which the player can iterate
            self.player_completion.clear();
            for &id in &ctx.clients.by_name {
                let name = ctx.clients.clients[id as usize].name.as_bytes();
                if let Some(found) = text::utf8_find_nocase(name, &self.completion_buffer) {
                    self.player_completion.push((id, found as i32));
                }
            }
            self.player_completion.sort_by_key(|&(_, score)| score);
        }

        let (offset, length) = (self.placeholder_offset, self.placeholder_length);
        let after = &s[(offset + length).min(s.len())..];
        if at(&self.completion_buffer, 0) == b'/' && !self.commands.is_empty() {
            let n = self.commands.len() as u64;
            if shift && self.completion_used {
                self.completion_chosen -= 1;
            } else if !shift {
                self.completion_chosen += 1;
            }
            // C++ int + size_t arithmetic: unsigned 64-bit, wrapping
            let chosen = self.completion_chosen as i64 as u64;
            self.completion_chosen = (chosen.wrapping_add(2 * n) % (2 * n)) as i32;
            self.completion_used = true;

            let start = self.completion_buffer[1..].to_vec();
            let chosen = self.completion_chosen as i64 as u64;
            let mut found = None;
            for i in 0..2 * n {
                let (search_type, index) = if shift {
                    (
                        (chosen.wrapping_sub(i).wrapping_add(2 * n) % (2 * n)) / n,
                        chosen.wrapping_sub(i).wrapping_add(n) % n,
                    )
                } else {
                    (
                        (chosen.wrapping_add(i) % (2 * n)) / n,
                        chosen.wrapping_add(i) % n,
                    )
                };
                let (search_type, index) = (search_type as i32, index as i32);
                if text::startswith_nocase(self.commands[index as usize].name.as_bytes(), &start) {
                    found = Some(index as usize);
                    self.completion_chosen = index + search_type * n as i32;
                    break;
                }
            }

            // insert the command
            if let Some(c) = found {
                let c = &self.commands[c];
                let mut buf = text::truncate(&s, (offset + 1).min(MAX_CHAT_LENGTH));
                append(&mut buf, b"/");
                append(&mut buf, c.name.as_bytes());
                let separator = if c.params.is_empty() { "" } else { " " };
                append(&mut buf, separator.as_bytes());
                append(&mut buf, after);
                self.placeholder_length = separator.len() + c.name.len() + 1;
                self.input.set(&String::from_utf8_lossy(&buf));
                self.input.set_cursor(offset + self.placeholder_length);
            }
        } else {
            // find next possible name
            let mut completion = None;
            let len = self.player_completion.len() as i32;
            for _ in 0..len {
                if shift && self.completion_used {
                    self.completion_chosen -= 1;
                } else if !shift {
                    self.completion_chosen += 1;
                }
                if self.completion_chosen < 0 {
                    self.completion_chosen += len;
                }
                self.completion_chosen %= len;
                self.completion_used = true;
                let id = self.player_completion[self.completion_chosen as usize].0;
                let client = &ctx.clients.clients[id as usize];
                if !client.active {
                    continue;
                }
                completion = Some(client.name.clone());
                break;
            }

            // insert the name
            if let Some(mut name) = completion {
                let mut buf = text::truncate(&s, (offset + 1).min(MAX_CHAT_LENGTH));
                // quote the name
                if at(&s, 0) == b'/' && (name.contains(' ') || name.contains('"')) {
                    name = c_str(
                        format!("\"{}\"", crate::config::escape(&name)).as_bytes(),
                        128,
                    );
                }
                append(&mut buf, name.as_bytes());
                let separator = if at(&s, offset + length) != b' ' {
                    if offset == 0 { ": " } else { " " }
                } else if offset == 0 {
                    ":"
                } else {
                    ""
                };
                append(&mut buf, separator.as_bytes());
                append(&mut buf, after);
                self.placeholder_length = separator.len() + name.len();
                self.input.set(&String::from_utf8_lossy(&buf));
                self.input.set_cursor(offset + self.placeholder_length);
            }
        }
    }

    /// The command hint after the input (C++ `OnRender`, chat.cpp:1249): the rest of the
    /// first server command the typed `/prefix` starts.
    pub fn command_hint(&self) -> Option<&str> {
        let s = self.input.text.as_bytes();
        if at(s, 0) != b'/' || at(s, 1) == 0 {
            return None;
        }
        let typed = &s[1..];
        (self.commands.iter())
            .find(|c| text::startswith_nocase(c.name.as_bytes(), typed))
            .and_then(|c| c.name.get(typed.len()..))
    }

    /// The shown lines, newest first, as C++ `OnRender` walks them: initialized ones,
    /// stopping at the first older than 16 s unless the chat is open (`show_all`).
    pub fn visible(&self, now: i64, show_all: bool) -> impl Iterator<Item = (&Line, f32)> {
        (0..MAX_LINES)
            .map(move |i| &self.lines[(self.current + MAX_LINES - i) % MAX_LINES])
            .take_while(move |l| l.initialized && (show_all || now <= l.time + 16 * FREQ))
            .map(move |l| {
                // C++: fade over the last 2 of 16 s
                let blend = if now > l.time + 14 * FREQ && !show_all {
                    1.0 - (now - l.time - 14 * FREQ) as f32 / (2 * FREQ) as f32
                } else {
                    1.0
                };
                (l, blend)
            })
    }
}

/// C++: the chat log line of `AddLine` (`log_info_color`, "chat/all" etc.).
fn log_line(l: &Line) {
    let from = if l.whisper {
        "chat/whisper"
    } else if l.team {
        "chat/team"
    } else if l.client_id == SERVER_MSG {
        "chat/server"
    } else if l.client_id == CLIENT_MSG {
        "chat/client"
    } else {
        "chat/all"
    };
    let sep = if l.client_id >= 0 { ": " } else { "" };
    println!("{from}: {}{sep}{}", l.name, l.text);
}

/// C++: `CChat::StoreSave` (chat.cpp:640): the name and save code of a "Team
/// successfully saved" server line.
pub fn parse_save(text: &str) -> Option<(String, String)> {
    let start = text.find("Team successfully saved by ")?;
    let mid = text.find(". Use '/load ")?;
    let on = text.find("' on ");
    let end = text.find(if on.is_some() {
        " to continue"
    } else {
        "' to continue"
    })?;
    if mid < start || end < mid || on.is_some_and(|on| on < mid || end < on) {
        return None;
    }
    let b = text.as_bytes();
    let cut = |from: usize, len: usize, size: usize| {
        let len = len.min(size - 1).min(b.len().saturating_sub(from));
        c_str(&b[from..from + len], size)
    };
    let name = cut(start + 27, (mid - start).saturating_sub(27), 16);
    let code = cut(mid + 13, on.unwrap_or(end).saturating_sub(mid + 13), 64);
    Some((name, code))
}

/// C++: `CsvWrite` (engine/shared/csv.cpp): one line, columns with `"` or `,` quoted
/// with doubled quotes.
pub fn csv_line(columns: &[&str]) -> String {
    let mut out = String::new();
    for (i, c) in columns.iter().enumerate() {
        if i != 0 {
            out.push(',');
        }
        if c.contains('"') || c.contains(',') {
            out.push('"');
            out.push_str(&c.replace('"', "\"\""));
            out.push('"');
        } else {
            out.push_str(c);
        }
    }
    out.push('\n');
    out
}

/// What the chat view reads besides the chat.
pub struct View<'a> {
    pub glass: crate::style::Glass,
    pub now: i64,
    pub config: &'a crate::config::Config,
    pub clients: &'a Clients,
    /// C++ `m_Teams.Team(id)`.
    pub team: &'a dyn Fn(i32) -> i32,
    /// C++: the scoreboard is open on a wide screen (the chat moves aside).
    pub scoreboard_open: bool,
}

mod view {
    use egui::text::{LayoutJob, TextFormat};
    use egui::{Align, Color32, Layout, Margin, Shadow, Ui};

    use super::*;
    use crate::render::map::hsl_packed_to_rgb;
    use crate::style::{Weight, color, font, radius, size, space};
    use crate::teehead::TeeHeads;
    use crate::widgets as w;

    /// The text field's egui id.
    pub const INPUT_ID: &str = "chat_input";

    fn rgb(c: [f32; 3]) -> Color32 {
        let b = |f: f32| (f.clamp(0.0, 1.0) * 255.0).round() as u8;
        Color32::from_rgb(b(c[0]), b(c[1]), b(c[2]))
    }

    /// A subtle tint: `amount` of `c` over the text colour (user decision 2026-11-27:
    /// the C++ kind colours, toned down, so kinds read at a glance).
    fn tint(c: Color32, amount: f32) -> Color32 {
        color::TEXT.lerp_to_gamma(c, amount)
    }

    /// C++ `ColorHSLA(h, s, l)` to RGB (as `GetDDTeamColor` builds it).
    fn hsl(h: f32, s: f32, l: f32) -> Color32 {
        let b = |f: f32| (f * 255.0).round() as u32;
        rgb(hsl_packed_to_rgb((b(h) << 16) | (b(s) << 8) | b(l)))
    }

    /// The colours of a line (C++ `OnPrepareLines`: name colour and text colour), tinted.
    fn colors(l: &Line, v: &View) -> (Color32, Color32) {
        let c = v.config;
        let cfg = |packed: u32| rgb(hsl_packed_to_rgb(packed));
        let name = if l.client_id == SERVER_MSG {
            cfg(c.cl_message_system_color)
        } else if l.client_id == CLIENT_MSG {
            cfg(c.cl_message_client_color)
        } else if l.team {
            cfg(c.cl_message_team_color)
        } else if l.name_color == TEAM_RED {
            rgb([1.0, 0.5, 0.5])
        } else if l.name_color == TEAM_BLUE {
            rgb([0.7, 0.7, 1.0])
        } else if l.name_color == TEAM_SPECTATORS {
            rgb([0.75, 0.5, 0.75])
        } else if l.client_id >= 0 && c.cl_chat_teamcolors != 0 && (v.team)(l.client_id) != 0 {
            // C++ GetDDTeamColor(team, 0.75): golden-angle hues
            let team = (v.team)(l.client_id);
            hsl(((team - 1) as f32 * 0.381_966).rem_euclid(1.0), 1.0, 0.75)
        } else {
            color::TEXT
        };
        let text = if l.client_id == SERVER_MSG {
            tint(cfg(c.cl_message_system_color), 0.55)
        } else if l.client_id == CLIENT_MSG {
            tint(cfg(c.cl_message_client_color), 0.55)
        } else if l.highlighted {
            tint(color::ACCENT, 0.6)
        } else if l.team {
            tint(cfg(c.cl_message_team_color), 0.35)
        } else {
            color::TEXT
        };
        (tint(name, 0.7), text)
    }

    /// The kind icon of a line: server, client (echo), whisper, team; none for all-chat.
    fn kind_icon(l: &Line) -> Option<&'static str> {
        if l.client_id == SERVER_MSG {
            Some(crate::icons::SERVER)
        } else if l.client_id == CLIENT_MSG {
            Some(crate::icons::INFO)
        } else if l.whisper {
            Some(crate::icons::WHISPER)
        } else if l.team {
            Some(crate::icons::USERS)
        } else {
            None
        }
    }

    /// C++ `FormatClientId(INDENT_AUTO)`, with figure spaces.
    fn client_id_prefix(id: i32, highest: i32) -> String {
        let mut s = String::new();
        if id < 100 && highest >= 100 {
            s.push('\u{2007}');
        }
        if id < 10 && highest >= 10 {
            s.push('\u{2007}');
        }
        format!("{s}{id}: ")
    }

    /// One line's text: id, name, text and the repeat count, wrapped at `wrap`.
    fn job(l: &Line, v: &View, wrap: f32) -> LayoutJob {
        let (name_color, text_color) = colors(l, v);
        let weak = color::TEXT_SECONDARY;
        let mut job = LayoutJob::default();
        job.wrap.max_width = wrap;
        let fmt = |weight, color| TextFormat {
            font_id: font(size::BODY, weight),
            color,
            ..Default::default()
        };
        let player = l.client_id >= 0 && !l.name.is_empty();
        if player {
            if v.config.cl_show_ids != 0 {
                let highest = (v.clients.infos.iter())
                    .rposition(Option::is_some)
                    .map_or(0, |i| i as i32);
                let id = client_id_prefix(l.client_id, highest);
                job.append(&id, 0.0, fmt(Weight::Regular, weak));
            }
            job.append(&l.name, 0.0, fmt(Weight::Medium, name_color));
        }
        let gap = if player { space::S } else { 0.0 };
        job.append(&l.text, gap, fmt(Weight::Regular, text_color));
        if l.times_repeated > 0 {
            let count = format!("×{}", l.times_repeated + 1);
            job.append(&count, space::S, fmt(Weight::Regular, weak));
        }
        job
    }

    /// One chat line: tee head, friend and kind icons, the text. One row is a pill, a
    /// wrapped line a rounded rectangle (CLAUDE.md shapes).
    fn bubble(ui: &mut Ui, l: &Line, v: &View, heads: &mut TeeHeads, wrap: f32, alpha: f32) {
        let galley = ui.fonts_mut(|f| f.layout_job(job(l, v, wrap)));
        let corner = if galley.rows.len() <= 1 {
            radius::PILL
        } else {
            radius::SMALL
        };
        let margin = Margin::symmetric(space::M as i8, space::XS as i8);
        let (name_color, _) = colors(l, v);
        ui.scope(|ui| {
            ui.set_opacity(alpha);
            w::hud_bar_with(
                ui,
                v.glass,
                corner,
                margin,
                size::ICON_LARGE,
                Shadow::NONE,
                |ui| {
                    ui.spacing_mut().item_spacing.x = space::S;
                    ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                        if let Some(look) = &l.tee {
                            let ctx = ui.ctx().clone();
                            if let Some(t) = heads.get(&ctx, look) {
                                let s = size::ICON_LARGE;
                                ui.add(egui::Image::new((t.id(), egui::vec2(s, s))));
                            }
                        }
                        if l.friend && v.config.cl_message_friend != 0 {
                            let friend = rgb(hsl_packed_to_rgb(v.config.cl_message_friend_color));
                            w::icon(
                                ui,
                                crate::icons::FRIEND,
                                size::ICON_SMALL,
                                tint(friend, 0.7),
                            );
                        }
                        if let Some(glyph) = kind_icon(l) {
                            w::icon(ui, glyph, size::ICON_SMALL, name_color);
                        }
                        ui.label(galley);
                    });
                },
            );
        });
    }

    /// The input pill: chat icon, mode, the text field and the command hint.
    fn input(ui: &mut Ui, chat: &mut Chat, v: &View, width: f32) {
        let margin = Margin::symmetric(space::M as i8, space::S as i8);
        let weak = ui.visuals().weak_text_color();
        w::hud_bar(ui, v.glass, radius::PILL, margin, size::ICON_LARGE, |ui| {
            ui.spacing_mut().item_spacing.x = space::S;
            ui.set_width(width);
            w::icon(ui, crate::icons::CHAT, size::ICON, weak);
            // C++: Localize("All") / Localize("Team")
            let mode = if chat.mode == Mode::Team {
                "Team"
            } else {
                "All"
            };
            w::text(ui, mode, size::BODY, Weight::Medium, weak);
            let id = egui::Id::unique(INPUT_ID);
            if chat.input.changed {
                let chars = chat.input.text[..chat.input.cursor].chars().count();
                let mut state =
                    egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
                let c = egui::text::CCursor::new(chars);
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::one(c)));
                state.store(ui.ctx(), id);
                chat.input.changed = false;
            }
            let out = egui::TextEdit::singleline(&mut chat.input.text)
                .id(id)
                .frame(egui::Frame::NONE)
                .margin(Margin::ZERO)
                .font(font(size::BODY, Weight::Regular))
                .text_color(color::TEXT)
                .desired_width(ui.available_width())
                .show(ui);
            out.response.request_focus();
            // C++: the input holds at most 255 bytes
            if chat.input.text.len() >= MAX_CHAT_LENGTH {
                chat.input.text = c_str(chat.input.text.as_bytes(), MAX_CHAT_LENGTH);
            }
            if let Some(r) = out.cursor_range {
                chat.input.cursor = (chat.input.text.char_indices())
                    .nth(r.primary.index.0)
                    .map_or(chat.input.text.len(), |(b, _)| b);
            }
            if let Some(hint) = chat.command_hint() {
                let at = out.galley_pos + egui::vec2(out.galley.rect.right(), 0.0);
                let font = font(size::BODY, Weight::Regular);
                (ui.painter()).text(at, egui::Align2::LEFT_TOP, hint, font, weak);
            }
        });
    }

    /// Draws the chat bottom left: the lines (C++ `OnRender`: newest at the bottom, up
    /// to a height limit, faded out after 14-16 s unless the chat is open) and, while
    /// typing, the input.
    pub fn show(ctx: &egui::Context, chat: &mut Chat, v: &View, heads: &mut TeeHeads) {
        let c = v.config;
        // C++ sizes are in a 300-unit-high screen
        let unit = ctx.content_rect().height() / 300.0;
        // C++: the large area while typing (cl_showchat 1), on +show_chat or always (2)
        let large = chat.show || (chat.is_active() && c.cl_showchat == 1) || c.cl_showchat == 2;
        let width = if v.scoreboard_open {
            85.0
        } else {
            c.cl_chat_width as f32
        } * unit;
        // C++: lines from y = 300 - 20 * 8/6 up to y = 200 (50 large, 180 next to the
        // scoreboard)
        let limit = if v.scoreboard_open {
            180.0
        } else if large {
            50.0
        } else {
            200.0
        };
        let max_height = (300.0 - 20.0 * 8.0 / 6.0 - limit) * unit;
        let input_height = size::ICON_LARGE + 2.0 * space::S;
        let wrap =
            width - 2.0 * space::M - size::ICON_LARGE - 3.0 * space::S - 2.0 * size::ICON_SMALL;
        // C++ walks the lines newest first and stops at the height limit; they are drawn
        // oldest first, top down, above the input
        let mut lines: Vec<(Line, f32)> = Vec::new();
        if c.cl_showchat != 0 {
            let mut used = 0.0;
            for (l, alpha) in chat.visible(v.now, large) {
                let h = ctx
                    .fonts_mut(|f| f.layout_job(job(l, v, wrap)))
                    .rect
                    .height();
                used += h.max(size::ICON_LARGE) + 2.0 * space::XS + space::XS;
                // C++: cut off if msgs waste too much space
                if used > max_height {
                    break;
                }
                lines.push((l.clone(), alpha));
            }
        }
        egui::Area::new(egui::Id::unique("chat"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(space::L, -space::L))
            .interactable(chat.is_active())
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = space::XS;
                for (l, alpha) in lines.iter().rev() {
                    bubble(ui, l, v, heads, wrap, *alpha);
                }
                if chat.is_active() {
                    input(ui, chat, v, width - 2.0 * space::M);
                } else {
                    ui.add_space(input_height);
                }
            });
    }
}

pub use view::{INPUT_ID, show};
