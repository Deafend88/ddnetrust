//! Settings and key binds in the C++ console syntax. C++: the line parsing of
//! `CConsole` (engine/shared/console.cpp), the config variables DDNet-rs uses
//! (engine/shared/config_variables.h, config.cpp) and `CBinds`
//! (game/client/components/binds.cpp).
//!
//! The C++ client's `settings_ddnet.cfg` is read once, read only (PLAN §9.7); unknown
//! commands in it (other variables, other clients' settings) are ignored.

use std::collections::BTreeMap;

use crate::keynames::KEY_NAMES;

/// C++: `KEY_LAST`.
pub const KEY_LAST: usize = 512;
/// C++: `KEY_ESCAPE`, `KEY_MOUSE_1`, `KEY_MOUSE_WHEEL_UP`, ... (engine/keys.h).
pub const KEY_ESCAPE: usize = 41;
pub const KEY_LCTRL: usize = 224;
pub const KEY_LSHIFT: usize = 225;
pub const KEY_LALT: usize = 226;
pub const KEY_LGUI: usize = 227;
pub const KEY_RCTRL: usize = 228;
pub const KEY_RSHIFT: usize = 229;
pub const KEY_RALT: usize = 230;
pub const KEY_RGUI: usize = 231;
pub const KEY_MOUSE_1: usize = 291;
pub const KEY_MOUSE_WHEEL_UP: usize = 300;
pub const KEY_MOUSE_WHEEL_DOWN: usize = 301;
pub const KEY_MOUSE_WHEEL_LEFT: usize = 302;
pub const KEY_MOUSE_WHEEL_RIGHT: usize = 303;

/// C++: `KeyModifier` (binds.h).
pub mod modifier {
    pub const NONE: usize = 0;
    pub const CTRL: usize = 1;
    pub const ALT: usize = 2;
    pub const SHIFT: usize = 3;
    pub const GUI: usize = 4;
}

/// C++: `CONSOLE_MAX_STR_LENGTH`: a command is cut to this many bytes.
const MAX_STR_LENGTH: usize = 8192;

/// C++: `str_isspace`.
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\n' | b'\r' | b'\t')
}

fn skip_whitespaces(s: &str) -> &str {
    s.trim_start_matches(|c: char| c.is_ascii() && is_space(c as u8))
}

/// C++: `str_skip_to_whitespace`, as the index of the first whitespace.
fn to_whitespace(s: &str) -> usize {
    s.bytes().position(is_space).unwrap_or(s.len())
}

/// C++: the command loop of `CConsole::ExecuteLineStroked`: splits a line at `;`
/// outside quotes (a `#` outside quotes ends it) and does `ParseStart` on each part.
/// Returns (command, arguments) pairs; parts without a command are skipped.
pub fn split_line(line: &str) -> Vec<(String, String)> {
    // "mc;" only forces semicolon interpretation, which is already on (C++ default)
    let mut s = line.strip_prefix("mc;").unwrap_or(line).as_bytes();
    let mut out = Vec::new();
    while !s.is_empty() {
        let (mut end, mut next) = (0, None);
        let (mut in_string, mut escaping) = (false, false);
        while end < s.len() {
            let c = s[end];
            if escaping {
                escaping = false;
            } else if c == b'"' {
                in_string = !in_string;
            } else if in_string && c == b'\\' {
                escaping = true;
            }
            if !in_string {
                if c == b';' {
                    next = Some(end + 1);
                    break;
                } else if c == b'#' {
                    break;
                }
            }
            end += 1;
        }
        // C++ ParseStart: str_copy into CONSOLE_MAX_STR_LENGTH + 1 bytes
        let mut part = s[..end.min(MAX_STR_LENGTH)].to_vec();
        ddnet::protocol::packer::utf8_fix_truncation(&mut part);
        let part = String::from_utf8_lossy(&part);
        let part = skip_whitespaces(&part);
        let cmd_end = to_whitespace(part);
        let command = &part[..cmd_end];
        let args = part.get(cmd_end + 1..).unwrap_or("");
        if !command.is_empty() {
            out.push((command.to_string(), args.to_string()));
        }
        match next {
            Some(n) => s = &s[n..],
            None => break,
        }
    }
    out
}

/// C++: `CConsole::NextParam`.
fn next_param(format: &[u8], pos: &mut usize) -> u8 {
    let at = |p: usize| format.get(p).copied().unwrap_or(0);
    if at(*pos) != 0 {
        *pos += 1;
        if at(*pos) == b'[' {
            while at(*pos) != b']' {
                if at(*pos) == 0 {
                    return 0;
                }
                *pos += 1;
            }
            *pos += 1;
            if at(*pos) == b' ' {
                *pos += 1;
            }
        }
    }
    at(*pos)
}

/// C `isspace` (what `strtol`/`strtod` skip).
fn is_c_space(c: u8) -> bool {
    matches!(c, b' ' | 0x09..=0x0d)
}

/// C++ `strtol(s, &end, 10)` with `long` = `i32` (Windows): the value (clamped) and how
/// many bytes it consumed (0 if there are no digits).
fn strtol(s: &str) -> (i32, usize) {
    let b = s.as_bytes();
    let mut i = b.iter().take_while(|&&c| is_c_space(c)).count();
    let neg = b.get(i) == Some(&b'-');
    if matches!(b.get(i), Some(b'-' | b'+')) {
        i += 1;
    }
    let digits = b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return (0, 0);
    }
    let v = b[i..i + digits]
        .iter()
        .fold(0i64, |v, c| (v * 10 + (c - b'0') as i64).min(1 << 40));
    let v = if neg { -v } else { v };
    (v.clamp(i32::MIN as i64, i32::MAX as i64) as i32, i + digits)
}

/// C++: `str_toint(str, &out)`: `strtol` that must consume the whole string.
pub fn str_toint(s: &str) -> Option<i32> {
    let (v, end) = strtol(s);
    (end == s.len()).then_some(v)
}

/// C++: `str_tofloat`: `strtod` (to double, then float) that must consume the whole
/// string. ponytail: Rust's parser, not strtod: no hex floats.
fn str_tofloat(s: &str) -> Option<f32> {
    s.trim_start_matches(|c: char| c.is_ascii() && is_c_space(c as u8))
        .parse::<f64>()
        .ok()
        .map(|v| v as f32)
}

/// C++: the packed-number branch of `CConsole::ColorParse` (`str_isallnum`, optionally
/// signed, then `strtoul`), the only form the C++ client saves. Hex (`$RRGGBB`) and
/// colour names are not supported. ponytail: add them if a config ever uses them.
fn color_parse(s: &str) -> Option<u32> {
    let digits = s.strip_prefix(['-', '+']).unwrap_or(s);
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // strtoul: negation wraps, overflow gives ULONG_MAX, which ColorParse rejects
    let parsed = digits.bytes().try_fold(0u64, |v, b| {
        Some(v * 10 + (b - b'0') as u64).filter(|&v| v <= u32::MAX as u64)
    });
    let v = match parsed {
        None => u32::MAX,
        Some(v) if s.starts_with('-') => (v as u32).wrapping_neg(),
        Some(v) => v as u32,
    };
    (v != u32::MAX).then_some(v)
}

/// C++: `CConsole::ParseArgs` for the parameter kinds used here (`s`, `r`, `i`, `c`, `f`;
/// others are strings). `args` may already hold the stroke argument.
pub fn parse_args(mut s: &str, format: &str, args: &mut Vec<String>) -> bool {
    let format = format.as_bytes();
    let mut pos = 0;
    let mut optional = false;
    let mut command = format.first().copied().unwrap_or(0);
    while command != 0 {
        if command == b'?' {
            optional = true;
            command = next_param(format, &mut pos);
            continue;
        }
        s = skip_whitespaces(s);
        if s.is_empty() {
            return optional;
        }
        if let Some(rest) = s.strip_prefix('"') {
            let (mut arg, mut chars) = (String::new(), rest.char_indices());
            loop {
                match chars.next() {
                    None => return false,
                    Some((i, '"')) => {
                        s = &rest[i + 1..];
                        break;
                    }
                    Some((_, '\\')) if rest[chars.offset()..].starts_with(['\\', '"']) => {
                        arg.push(chars.next().unwrap().1);
                    }
                    Some((_, c)) => arg.push(c),
                }
            }
            args.push(arg);
        } else {
            if command == b'r' {
                args.push(s.to_string());
                return true;
            }
            let end = to_whitespace(s);
            args.push(s[..end].to_string());
            s = s.get(end + 1..).unwrap_or("");
        }
        let last = args.last().unwrap();
        let valid = match command {
            b'i' => str_toint(last).is_some_and(|v| v != i32::MAX && v != i32::MIN),
            b'c' => color_parse(last).is_some(),
            b'f' => str_tofloat(last).is_some_and(|v| v != f32::MAX && v != f32::MIN_POSITIVE),
            _ => true,
        };
        if !valid {
            return false;
        }
        command = next_param(format, &mut pos);
    }
    true
}

/// C++: `CResult::GetFloat` (`str_tofloat` result, 0 when invalid).
pub fn arg_float(args: &[String], i: usize) -> f32 {
    args.get(i).and_then(|a| str_tofloat(a)).unwrap_or(0.0)
}

/// The C++ config variables DDNet-rs reads, with their C++ names, defaults and ranges.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub player_name: String,
    pub player_skin: String,
    pub player_use_custom_color: i32,
    pub player_color_body: u32,
    pub player_color_feet: u32,
    pub inp_mousesens: i32,
    pub cl_dyncam: i32,
    pub cl_dyncam_max_distance: i32,
    pub cl_dyncam_min_distance: i32,
    pub cl_dyncam_mousesens: i32,
    pub cl_dyncam_deadzone: i32,
    pub cl_dyncam_follow_factor: i32,
    pub cl_dyncam_smoothness: i32,
    pub cl_dyncam_stabilizing: i32,
    pub cl_mouse_deadzone: i32,
    pub cl_mouse_followfactor: i32,
    pub cl_mouse_max_distance: i32,
    pub cl_mouse_min_distance: i32,
    pub cl_default_zoom: i32,
    pub cl_smooth_zoom_time: i32,
    pub cl_limit_max_zoom_level: i32,
    pub cl_race_binds_set: i32,
    pub gfx_vsync: i32,
    /// C++ HUD switches, imported as the HUD items' first values (`HudConfig::import_cpp`).
    pub cl_showhud_player_position: i32,
    pub cl_showhud_player_speed: i32,
    pub cl_showhud_player_angle: i32,
    pub cl_showhud_jumps_indicator: i32,
    pub cl_showhud_ddrace: i32,
    pub cl_showkillmessages: i32,
    pub cl_show_finish_messages: i32,
    pub cl_showhud_timer: i32,
    pub cl_showhud_time_cp_diff: i32,
    pub cl_show_broadcasts: i32,
    pub cl_showhud_spectator: i32,
    /// Ours (not a C++ variable): the DDNet data dir, used when neither `--data-dir`
    /// nor `DDNET_DATA_DIR` is given.
    pub rs_data_dir: String,
    /// The media player's chosen source (an SMTC app id; empty: automatic).
    pub rs_media_source: String,
    /// Ours: the media player (now playing, lyrics, volume) on or off.
    pub rs_media: i32,
    /// Ours: glass strength, 0 off, 1 subtle, 2 full (PLAN §10.2).
    pub rs_glass: i32,
    /// Ours: UI scale in percent on top of the window's DPI.
    pub rs_ui_scale: i32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            player_name: String::new(),
            player_skin: "default".into(),
            player_use_custom_color: 0,
            player_color_body: 65408,
            player_color_feet: 65408,
            inp_mousesens: 200,
            cl_dyncam: 0,
            cl_dyncam_max_distance: 1000,
            cl_dyncam_min_distance: 0,
            cl_dyncam_mousesens: 0,
            cl_dyncam_deadzone: 300,
            cl_dyncam_follow_factor: 60,
            cl_dyncam_smoothness: 0,
            cl_dyncam_stabilizing: 0,
            cl_mouse_deadzone: 0,
            cl_mouse_followfactor: 0,
            cl_mouse_max_distance: 400,
            cl_mouse_min_distance: 0,
            cl_default_zoom: 10,
            cl_smooth_zoom_time: 250,
            cl_limit_max_zoom_level: 1,
            cl_race_binds_set: 0,
            gfx_vsync: 0,
            cl_showhud_player_position: 0,
            cl_showhud_player_speed: 0,
            cl_showhud_player_angle: 0,
            cl_showhud_jumps_indicator: 1,
            cl_showhud_ddrace: 1,
            cl_showkillmessages: 1,
            cl_show_finish_messages: 1,
            cl_showhud_timer: 1,
            cl_showhud_time_cp_diff: 1,
            cl_show_broadcasts: 1,
            cl_showhud_spectator: 1,
            rs_data_dir: String::new(),
            rs_media_source: String::new(),
            rs_media: 1,
            rs_glass: 1,
            rs_ui_scale: 100,
        }
    }
}

/// One config variable: C++ `SIntConfigVariable`, `SColorConfigVariable` (all ours
/// use `CFGFLAG_COLLIGHT`, no alpha) or `SStringConfigVariable`.
enum Var<'a> {
    Int(&'a mut i32, i32, i32),
    Color(&'a mut u32),
    Str(&'a mut String, usize),
}

impl Config {
    /// An integer variable by its C++ name.
    pub fn int(&self, name: &str) -> Option<i32> {
        match self.clone().var(name)? {
            Var::Int(v, ..) => Some(*v),
            _ => None,
        }
    }

    fn var(&mut self, name: &str) -> Option<Var<'_>> {
        use Var::*;
        Some(match name.to_ascii_lowercase().as_str() {
            "player_name" => Str(&mut self.player_name, 16),
            "player_skin" => Str(&mut self.player_skin, 24),
            "player_use_custom_color" => Int(&mut self.player_use_custom_color, 0, 1),
            "player_color_body" => Color(&mut self.player_color_body),
            "player_color_feet" => Color(&mut self.player_color_feet),
            "inp_mousesens" => Int(&mut self.inp_mousesens, 1, 100000),
            "cl_dyncam" => Int(&mut self.cl_dyncam, 0, 1),
            "cl_dyncam_max_distance" => Int(&mut self.cl_dyncam_max_distance, 0, 2000),
            "cl_dyncam_min_distance" => Int(&mut self.cl_dyncam_min_distance, 0, 2000),
            "cl_dyncam_mousesens" => Int(&mut self.cl_dyncam_mousesens, 0, 100000),
            "cl_dyncam_deadzone" => Int(&mut self.cl_dyncam_deadzone, 1, 1300),
            "cl_dyncam_follow_factor" => Int(&mut self.cl_dyncam_follow_factor, 0, 200),
            "cl_dyncam_smoothness" => Int(&mut self.cl_dyncam_smoothness, 0, 100),
            "cl_dyncam_stabilizing" => Int(&mut self.cl_dyncam_stabilizing, 0, 100),
            "cl_mouse_deadzone" => Int(&mut self.cl_mouse_deadzone, 0, 3000),
            "cl_mouse_followfactor" => Int(&mut self.cl_mouse_followfactor, 0, 200),
            "cl_mouse_max_distance" => Int(&mut self.cl_mouse_max_distance, 0, 5000),
            "cl_mouse_min_distance" => Int(&mut self.cl_mouse_min_distance, 0, 5000),
            "cl_default_zoom" => Int(&mut self.cl_default_zoom, 0, 20),
            "cl_smooth_zoom_time" => Int(&mut self.cl_smooth_zoom_time, 0, 5000),
            "cl_limit_max_zoom_level" => Int(&mut self.cl_limit_max_zoom_level, 0, 1),
            "cl_race_binds_set" => Int(&mut self.cl_race_binds_set, 0, 3),
            "gfx_vsync" => Int(&mut self.gfx_vsync, 0, 1),
            "cl_showhud_player_position" => Int(&mut self.cl_showhud_player_position, 0, 1),
            "cl_showhud_player_speed" => Int(&mut self.cl_showhud_player_speed, 0, 1),
            "cl_showhud_player_angle" => Int(&mut self.cl_showhud_player_angle, 0, 1),
            "cl_showhud_jumps_indicator" => Int(&mut self.cl_showhud_jumps_indicator, 0, 1),
            "cl_showhud_ddrace" => Int(&mut self.cl_showhud_ddrace, 0, 1),
            "cl_showkillmessages" => Int(&mut self.cl_showkillmessages, 0, 1),
            "cl_show_finish_messages" => Int(&mut self.cl_show_finish_messages, 0, 1),
            "cl_showhud_timer" => Int(&mut self.cl_showhud_timer, 0, 1),
            "cl_showhud_time_cp_diff" => Int(&mut self.cl_showhud_time_cp_diff, 0, 1),
            "cl_show_broadcasts" => Int(&mut self.cl_show_broadcasts, 0, 1),
            "cl_showhud_spectator" => Int(&mut self.cl_showhud_spectator, 0, 1),
            "rs_data_dir" => Str(&mut self.rs_data_dir, 512),
            "rs_media_source" => Str(&mut self.rs_media_source, 512),
            "rs_media" => Int(&mut self.rs_media, 0, 1),
            "rs_glass" => Int(&mut self.rs_glass, 0, 2),
            "rs_ui_scale" => Int(&mut self.rs_ui_scale, 50, 200),
            _ => return None,
        })
    }

    /// C++: the `CommandCallback` of each variable kind (config.cpp:40, :116, :206).
    /// Returns false if `name` is not one of ours.
    fn set(&mut self, name: &str, args: &str) -> bool {
        let Some(var) = self.var(name) else {
            return false;
        };
        let mut a = Vec::new();
        let format = match var {
            Var::Int(..) => "?i",
            Var::Color(_) => "?c",
            Var::Str(..) => "?r",
        };
        if !parse_args(args, format, &mut a) || a.is_empty() {
            return true; // invalid arguments or a query: nothing changes
        }
        match var {
            Var::Int(v, min, max) => {
                let mut value = str_toint(&a[0]).unwrap_or(0);
                if min != max {
                    if value < min {
                        value = min;
                    }
                    if max != 0 && value > max {
                        value = max;
                    }
                }
                *v = value;
            }
            Var::Color(v) => *v = color_pack_light(color_parse(&a[0]).unwrap_or(0)),
            Var::Str(v, size) => *v = str_copy(&a[0], size),
        }
        true
    }

    /// The variables as the C++ `CConfigManager::Save` writes them: the non-default
    /// ones, or with `all` every one (our settings file must override the C++ one).
    pub fn serialize(&self, all: bool) -> Vec<String> {
        let mut clone = self.clone();
        let mut def = Self::default();
        let mut out = Vec::new();
        for name in VAR_NAMES {
            let line = match (clone.var(name).unwrap(), def.var(name).unwrap()) {
                (Var::Int(v, ..), Var::Int(dv, ..)) if all || v != dv => format!("{name} {v}"),
                (Var::Color(v), Var::Color(dv)) if all || v != dv => format!("{name} {v}"),
                (Var::Str(v, _), Var::Str(dv, _)) if all || v != dv => {
                    format!("{name} \"{}\"", escape(v))
                }
                _ => continue,
            };
            out.push(line);
        }
        out
    }
}

/// Every variable name `Config::var` knows.
pub const VAR_NAMES: [&str; 39] = [
    "player_name",
    "player_skin",
    "player_use_custom_color",
    "player_color_body",
    "player_color_feet",
    "inp_mousesens",
    "cl_dyncam",
    "cl_dyncam_max_distance",
    "cl_dyncam_min_distance",
    "cl_dyncam_mousesens",
    "cl_dyncam_deadzone",
    "cl_dyncam_follow_factor",
    "cl_dyncam_smoothness",
    "cl_dyncam_stabilizing",
    "cl_mouse_deadzone",
    "cl_mouse_followfactor",
    "cl_mouse_max_distance",
    "cl_mouse_min_distance",
    "cl_default_zoom",
    "cl_smooth_zoom_time",
    "cl_limit_max_zoom_level",
    "cl_race_binds_set",
    "gfx_vsync",
    "cl_showhud_player_position",
    "cl_showhud_player_speed",
    "cl_showhud_player_angle",
    "cl_showhud_jumps_indicator",
    "cl_showhud_ddrace",
    "cl_showkillmessages",
    "cl_show_finish_messages",
    "cl_showhud_timer",
    "cl_showhud_time_cp_diff",
    "cl_show_broadcasts",
    "cl_showhud_spectator",
    "rs_data_dir",
    "rs_media_source",
    "rs_media",
    "rs_glass",
    "rs_ui_scale",
];

/// C++: `str_copy` into a buffer of `size` bytes (cut at `size - 1` bytes, then
/// `str_utf8_fix_truncation`).
fn str_copy(s: &str, size: usize) -> String {
    let mut b = s.as_bytes()[..s.len().min(size - 1)].to_vec();
    ddnet::protocol::packer::utf8_fix_truncation(&mut b);
    String::from_utf8_lossy(&b).into_owned()
}

/// C++: `ColorHSLA(Value, true).UnclampLighting(DARKEST_LGT)` then
/// `Pack(DARKEST_LGT, false)`: what a colour variable stores for a packed number.
fn color_pack_light(v: u32) -> u32 {
    const DARKEST: f32 = 0.5;
    let ch = |shift: u32| ((v >> shift) & 0xFF) as f32 / 255.0;
    let (h, s) = (ch(16), ch(8));
    let l = DARKEST + ch(0) * (1.0 - DARKEST);
    let l = ((l - DARKEST) / (1.0 - DARKEST)).clamp(0.0, 1.0);
    let b = |f: f32| ddnet::physics::vmath::round_to_int(f * 255.0) as u32;
    (b(h) << 16).wrapping_add(b(s) << 8).wrapping_add(b(l))
}

/// C++: `str_escape`: `\` before `"` and `\`.
pub fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// C++: `CInput::FindKeyByName`: `&<number>` or a key name, case-insensitive; 0 if none.
pub fn find_key_by_name(name: &str) -> usize {
    if let Some(n) = name.strip_prefix('&') {
        // C++ str_toint(const char *): strtol of the prefix
        let k = strtol(n).0;
        if k > 0 && (k as usize) < KEY_LAST {
            return k as usize;
        }
    }
    KEY_NAMES
        .iter()
        .position(|k| k.eq_ignore_ascii_case(name))
        .unwrap_or(0)
}

/// C++: `CBinds` (one client, no dummy).
#[derive(Clone, Debug, Default)]
pub struct Binds {
    /// C++ `m_aapKeyBindings[modifier mask][key]`, ordered like the saved config.
    pub map: BTreeMap<(usize, usize), String>,
    /// C++ `m_vActiveBinds`: (key, modifier mask) of pressed binds.
    active: Vec<(usize, usize)>,
}

impl Binds {
    /// C++: `CBinds::Bind`.
    pub fn bind(&mut self, key: usize, command: &str, free_only: bool, mask: usize) {
        if free_only && self.map.contains_key(&(mask, key)) {
            return;
        }
        self.map.remove(&(mask, key));
        if !command.is_empty() {
            self.map.insert((mask, key), command.to_string());
        }
    }

    /// C++: `CBinds::GetKey`: the first (modifier, key) bound to exactly `command`.
    pub fn slot_of(&self, command: &str) -> Option<(usize, usize)> {
        self.map
            .iter()
            .find(|(_, c)| c.as_str() == command)
            .map(|(&(mask, key), _)| (key, mask))
    }

    /// C++: the settings key reader (menus_settings_controls.cpp:543): unbind the
    /// command's first slot, bind the new key (no modifiers) to it.
    pub fn rebind(&mut self, command: &str, key: usize) {
        if let Some((old, mask)) = self.slot_of(command) {
            self.bind(old, "", false, mask);
        }
        self.bind(key, command, false, 0);
    }

    fn get(&self, key: usize, mask: usize) -> &str {
        self.map.get(&(mask, key)).map_or("", |s| s)
    }

    fn bind_name(&mut self, name: &str, command: &str, free_only: bool) {
        self.bind(find_key_by_name(name), command, free_only, modifier::NONE);
    }

    /// C++: `CBinds::SetDefaults` (binds.cpp:269), which also sets the DDRace binds.
    pub fn set_defaults(&mut self, config: &mut Config) {
        self.map.clear();
        for (key, command) in [
            ("f1", "toggle_local_console"),
            ("f2", "toggle_remote_console"),
            ("tab", "+scoreboard"),
            ("equals", "+statboard"),
            ("f10", "screenshot"),
            ("a", "+left"),
            ("d", "+right"),
            ("space", "+jump"),
            ("mouse1", "+fire"),
            ("mouse2", "+hook"),
            ("lshift", "+emote"),
            ("return", "+show_chat; chat all"),
            ("right", "spectate_next"),
            ("left", "spectate_previous"),
            ("rshift", "+spectate"),
            ("1", "+weapon1"),
            ("2", "+weapon2"),
            ("3", "+weapon3"),
            ("4", "+weapon4"),
            ("5", "+weapon5"),
            ("mousewheelup", "+prevweapon"),
            ("mousewheeldown", "+nextweapon"),
            ("t", "+show_chat; chat all"),
            ("y", "+show_chat; chat team"),
            ("u", "+show_chat"),
            ("i", "+show_chat; chat all /c "),
            ("f3", "vote yes"),
            ("f4", "vote no"),
            ("k", "kill"),
            ("q", "say /spec"),
            ("p", "say /pause"),
        ] {
            self.bind_name(key, command, false);
        }
        config.cl_race_binds_set = 0;
        self.set_ddrace_binds(config, false);
    }

    /// C++: `CBinds::SetDDRaceBinds` (binds.cpp:514).
    pub fn set_ddrace_binds(&mut self, config: &mut Config, free_only: bool) {
        if config.cl_race_binds_set < 1 {
            for (key, command) in [
                ("kp_plus", "zoom+"),
                ("kp_minus", "zoom-"),
                ("kp_multiply", "zoom"),
                ("pause", "say /pause"),
                ("up", "+jump"),
                ("left", "+left"),
                ("right", "+right"),
                ("leftbracket", "+prevweapon"),
                ("rightbracket", "+nextweapon"),
                ("c", "say /rank"),
                ("v", "say /info"),
                ("b", "say /top5"),
                ("s", "+showhookcoll"),
                ("x", "toggle cl_dummy 0 1"),
                ("h", "toggle cl_dummy_hammer 0 1"),
                ("slash", "+show_chat; chat all /"),
                ("kp_0", "say /emote normal 999999"),
                ("kp_1", "say /emote happy 999999"),
                ("kp_2", "say /emote angry 999999"),
                ("kp_3", "say /emote pain 999999"),
                ("kp_4", "say /emote surprise 999999"),
                ("kp_5", "say /emote blink 999999"),
                ("minus", "spectate_previous"),
                ("equals", "spectate_next"),
            ] {
                self.bind_name(key, command, free_only);
            }
        }
        if config.cl_race_binds_set < 2 {
            let mouse3 = find_key_by_name("mouse3");
            let dont_modify_spectate = free_only && self.get(mouse3, 0) != "+spectate";
            self.bind(
                mouse3,
                "toggle_scoreboard_cursor; +spectate",
                dont_modify_spectate,
                0,
            );
            self.bind_name("lalt", "toggle_scoreboard_cursor", free_only);
        }
        if config.cl_race_binds_set < 3 {
            self.bind_name("w", "+jump", free_only);
        }
        config.cl_race_binds_set = 3;
    }

    /// C++: `CBinds::GetBindSlot`: (key, modifier mask) from `shift+ctrl+a`-style names;
    /// key 0 if invalid. Escape cannot be bound.
    pub fn bind_slot(name: &str) -> (usize, usize) {
        let b = name.as_bytes();
        // C++ str_next_token(&b[from], "+", aMod, 32): skips one leading '+' (C++
        // str_token_get), copies at most 31 bytes; returns (token, index after it)
        let next_token = |from: usize| -> Option<(&[u8], usize)> {
            let p = from + usize::from(b.get(from) == Some(&b'+'));
            if p >= b.len() {
                return None;
            }
            let len = b[p..].iter().take_while(|&&c| c != b'+').count().min(31);
            Some((&b[p..p + len], p + len))
        };
        let Some((mut m, mut key)) = next_token(0) else {
            return (0, 0);
        };
        let mut mask = 0;
        while !m.is_empty() && key < b.len() {
            match m.to_ascii_lowercase().as_slice() {
                b"shift" => mask |= 1 << modifier::SHIFT,
                b"ctrl" => mask |= 1 << modifier::CTRL,
                b"alt" => mask |= 1 << modifier::ALT,
                b"gui" => mask |= 1 << modifier::GUI,
                _ => return (0, 0),
            }
            if !b[key + 1..].contains(&b'+') {
                break;
            }
            match next_token(key + 1) {
                Some((t, k)) => (m, key) = (t, k),
                None => return (0, 0),
            }
        }
        let key_name = if mask == 0 {
            m
        } else {
            b.get(key + 1..).unwrap_or_default()
        };
        let key = find_key_by_name(&String::from_utf8_lossy(key_name));
        (if key == KEY_ESCAPE { 0 } else { key }, mask)
    }

    /// C++: `CBinds::GetKeyBindName`: modifiers (ctrl, alt, shift, gui order) then key.
    pub fn key_bind_name(key: usize, mask: usize) -> String {
        let mut s = String::new();
        for (m, name) in [
            (modifier::CTRL, "ctrl"),
            (modifier::ALT, "alt"),
            (modifier::SHIFT, "shift"),
            (modifier::GUI, "gui"),
        ] {
            if mask & (1 << m) != 0 {
                s += name;
                s += "+";
            }
        }
        s + KEY_NAMES[key]
    }

    /// The bind lines of the C++ `CBinds::ConfigSaveCallback` (after `unbindall`).
    pub fn serialize(&self) -> Vec<String> {
        self.map
            .iter()
            .map(|(&(mask, key), c)| {
                format!("bind {} \"{}\"", Self::key_bind_name(key, mask), escape(c))
            })
            .collect()
    }

    /// C++: `CBinds::GetModifierMaskOfKey`.
    fn modifier_mask_of_key(key: usize) -> usize {
        match key {
            KEY_LSHIFT | KEY_RSHIFT => 1 << modifier::SHIFT,
            KEY_LCTRL | KEY_RCTRL => 1 << modifier::CTRL,
            KEY_LALT | KEY_RALT => 1 << modifier::ALT,
            KEY_LGUI | KEY_RGUI => 1 << modifier::GUI,
            _ => 0,
        }
    }

    /// C++: `CBinds::OnInput` for one key event (`press`/`release`, both for a wheel
    /// step) with `pressed` the current key states (C++ `Input()->KeyIsPressed`).
    /// Returns the bind lines to run, with their stroke (C++ `ExecuteLineStroked`).
    /// `menus_active`: C++ menus take key presses before binds see them, and releases
    /// end active binds without running them (binds.cpp:180).
    pub fn on_input(
        &mut self,
        key: usize,
        press: bool,
        release: bool,
        menus_active: bool,
        pressed: impl Fn(usize) -> bool,
    ) -> Vec<(bool, String)> {
        let press = press && !menus_active;
        let mut run = Vec::new();
        let key_mask = Self::modifier_mask_of_key(key);
        let mask = [
            KEY_LSHIFT, KEY_RSHIFT, KEY_LCTRL, KEY_RCTRL, KEY_LALT, KEY_RALT, KEY_LGUI, KEY_RGUI,
        ]
        .into_iter()
        .filter(|&k| pressed(k))
        .fold(0, |m, k| m | Self::modifier_mask_of_key(k))
            & !key_mask;
        if press {
            match self.active.iter().find(|b| b.0 == key) {
                None => {
                    let both = |a: usize, b: usize| (1 << a) | (1 << b);
                    let slot = if self.map.contains_key(&(mask, key)) {
                        Some(mask)
                    } else if self.map.contains_key(&(0, key))
                        && mask != both(modifier::CTRL, modifier::SHIFT)
                        && mask != both(modifier::GUI, modifier::SHIFT)
                    {
                        Some(0)
                    } else {
                        None
                    };
                    if let Some(m) = slot {
                        run.push((true, self.get(key, m).to_string()));
                        self.active.push((key, m));
                    }
                }
                // repeat the active bind while the key is held down
                Some(&(k, m)) => {
                    if let Some(c) = self.map.get(&(m, k)) {
                        run.push((true, c.clone()));
                    }
                }
            }
        }
        if release {
            let mut end = |(k, m): (usize, usize)| {
                if let Some(c) = self.map.get(&(m, k)).filter(|_| !menus_active) {
                    run.push((false, c.clone()));
                }
            };
            if let Some(i) = self.active.iter().position(|b| b.0 == key) {
                end(self.active.remove(i));
            }
            if key_mask != 0 {
                while let Some(i) = self.active.iter().position(|b| b.1 & key_mask != 0) {
                    end(self.active.remove(i));
                }
            }
        }
        run
    }
}

/// Settings and binds after the C++ start-up order: `CBinds::SetDefaults`, the config
/// files, then `SetDDRaceBinds(true)` (gameclient.cpp:427).
#[derive(Clone, Debug)]
pub struct Settings {
    pub config: Config,
    pub binds: Binds,
    /// HUD layout and widget settings (`hud_widget`, `hud_set`; ours, not C++).
    pub hud: crate::hud::HudConfig,
    /// Config commands skipped because DDNet-rs does not use them (other stock DDNet
    /// settings, other clients' settings such as TClient's).
    pub skipped: usize,
}

impl Settings {
    pub fn load(files: &[&[u8]]) -> Self {
        let mut s = Self {
            config: Config::default(),
            binds: Binds::default(),
            hud: Default::default(),
            skipped: 0,
        };
        s.binds.set_defaults(&mut s.config);
        for f in files {
            for line in crate::storage::lines(f) {
                s.execute_line(line);
            }
        }
        s.binds.set_ddrace_binds(&mut s.config, true);
        s
    }

    /// Our binds (PLAN §10.6), added after `load` like `SetDDRaceBinds(true)`: only
    /// while the key is free. Kept out of `load`, which mirrors C++.
    pub fn add_our_binds(&mut self) {
        self.binds.bind_name("f6", "hud_editor", true);
        self.binds.bind_name("f7", "media_player", true);
    }

    /// Our settings file: every variable and every bind (it overrides the C++ file).
    pub fn serialize(&self) -> String {
        let mut text = String::from(
            "# DDNet-rs settings (C++ console syntax)
",
        );
        for line in self.config.serialize(true) {
            text += &format!(
                "{line}
"
            );
        }
        for line in self.hud.serialize() {
            text += &line;
            text.push('\n');
        }
        text += "unbindall
";
        for line in self.binds.serialize() {
            text += &format!(
                "{line}
"
            );
        }
        text
    }

    /// C++: `CConsole::ExecuteLine` for the config file commands: our variables, `bind`,
    /// `unbind`, `unbindall`, and our HUD commands. Everything else is skipped (counted in `skipped`); binds to
    /// commands DDNet-rs lacks are kept, like stock DDNet keeps binds to unknown ones.
    pub fn execute_line(&mut self, line: &str) {
        for (command, args) in split_line(line) {
            if self.config.set(&command, &args) || self.hud.execute(&command, &args) {
                continue;
            }
            let mut a = Vec::new();
            match command.to_ascii_lowercase().as_str() {
                "bind" if parse_args(&args, "s[key] ?r[command]", &mut a) => {
                    let (key, mask) = Binds::bind_slot(&a[0]);
                    if key != 0 && a.len() > 1 {
                        self.binds.bind(key, &a[1], false, mask);
                    }
                }
                "unbind" if parse_args(&args, "s[key]", &mut a) => {
                    let (key, mask) = Binds::bind_slot(&a[0]);
                    if key != 0 {
                        self.binds.bind(key, "", false, mask);
                    }
                }
                "unbindall" => self.binds.map.clear(),
                "bind" | "unbind" => {}
                _ => self.skipped += 1,
            }
        }
    }
}
