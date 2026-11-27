//! Settings and key binds in the C++ console syntax. C++: the line parsing of
//! `CConsole` (engine/shared/console.cpp), the config variables DDNet-rs uses
//! (engine/shared/config_variables.h, config.cpp) and `CBinds`
//! (game/client/components/binds.cpp).
//!
//! The C++ client's `settings_ddnet.cfg` is read once, read only (PLAN §9.7); unknown
//! commands in it (other variables, other clients' settings) are ignored.

use std::collections::BTreeMap;

use crate::keynames::KEY_NAMES;
pub use ddnet::console::{arg_float, color_parse, parse_args, split_line, str_toint, strtol};

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
    /// C++ prediction and antiping (config_variables.h:15-27, 730).
    pub cl_predict: i32,
    pub cl_antiping_limit: i32,
    pub cl_antiping_percent: i32,
    pub cl_antiping: i32,
    pub cl_antiping_players: i32,
    pub cl_antiping_grenade: i32,
    pub cl_antiping_weapons: i32,
    pub cl_antiping_smooth: i32,
    pub cl_antiping_gunfire: i32,
    pub cl_antiping_preinput: i32,
    pub cl_prediction_margin: i32,
    pub cl_predict_freeze: i32,
    /// C++ chat (config_variables.h:90-93, 357, 409-414, 628-653, 743-747).
    pub cl_showchat: i32,
    pub cl_show_chat_friends: i32,
    pub cl_show_chat_team_members_only: i32,
    pub cl_show_chat_system: i32,
    pub cl_chat_reset: i32,
    pub cl_chat_teamcolors: i32,
    pub cl_chat_width: i32,
    pub cl_show_ids: i32,
    pub cl_message_friend: i32,
    pub cl_friends_ignore_clan: i32,
    pub snd_servermessage: i32,
    pub snd_highlight: i32,
    pub snd_team_chat: i32,
    pub snd_chat: i32,
    pub snd_enable: i32,
    pub snd_rate: i32,
    pub snd_buffer_size: i32,
    pub snd_enable_music: i32,
    pub snd_volume: i32,
    pub snd_chat_volume: i32,
    pub snd_game_volume: i32,
    pub snd_ambient_volume: i32,
    pub snd_background_music_volume: i32,
    pub snd_nonactive_mute: i32,
    pub snd_game: i32,
    pub snd_gun: i32,
    pub snd_long_pain: i32,
    pub cl_predict_events: i32,
    pub gfx_high_detail: i32,
    pub cl_assets_entities: String,
    pub cl_asset_game: String,
    pub cl_asset_emoticons: String,
    pub cl_asset_particles: String,
    pub cl_asset_hud: String,
    pub cl_asset_extras: String,
    pub cl_spec_cursor: i32,
    pub cl_overlay_entities: i32,
    pub cl_text_entities: i32,
    pub cl_text_entities_size: i32,
    pub cl_message_system_color: u32,
    pub cl_message_client_color: u32,
    pub cl_message_highlight_color: u32,
    pub cl_message_team_color: u32,
    pub cl_message_color: u32,
    pub cl_message_friend_color: u32,
    /// C++ name plates, freeze bars and scoreboard (config_variables.h:43-57, 85-86, 654,
    /// 670, 750-751).
    pub cl_nameplates: i32,
    pub cl_nameplates_always: i32,
    pub cl_nameplates_teamcolors: i32,
    pub cl_nameplates_size: i32,
    pub cl_nameplates_clan: i32,
    pub cl_nameplates_clan_size: i32,
    pub cl_nameplates_ids: i32,
    pub cl_nameplates_ids_size: i32,
    pub cl_nameplates_ids_seperate_line: i32,
    pub cl_nameplates_offset: i32,
    pub cl_nameplates_own: i32,
    pub cl_nameplates_friendmark: i32,
    pub cl_nameplates_strong: i32,
    pub cl_nameplates_strong_size: i32,
    pub cl_show_direction: i32,
    pub cl_direction_size: i32,
    pub cl_show_others_alpha: i32,
    pub cl_show_freeze_bars: i32,
    pub cl_freezebars_alpha_inside_freeze: i32,
    pub cl_scoreboard_on_death: i32,
    pub cl_enable_ping_color: i32,
    /// Ours (not a C++ variable): the DDNet data dir, used when neither `--data-dir`
    /// nor `DDNET_DATA_DIR` is given.
    pub rs_data_dir: String,
    /// The media player's chosen source (an SMTC app id; empty: automatic).
    pub rs_media_source: String,
    /// Ours: the media player (now playing, lyrics, volume) on or off.
    pub rs_media: i32,
    /// Ours: glass strength, 0 off, 1 subtle, 2 full (PLAN §10.2).
    pub rs_glass: i32,
    /// Ours: UI scale in percent on top of the window-height scale (`style::ui_scale`).
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
            cl_predict: 1,
            cl_antiping_limit: 0,
            cl_antiping_percent: 100,
            cl_antiping: 0,
            cl_antiping_players: 1,
            cl_antiping_grenade: 1,
            cl_antiping_weapons: 1,
            cl_antiping_smooth: 0,
            cl_antiping_gunfire: 1,
            cl_antiping_preinput: 1,
            cl_prediction_margin: 10,
            cl_predict_freeze: 1,
            cl_showchat: 1,
            cl_show_chat_friends: 0,
            cl_show_chat_team_members_only: 0,
            cl_show_chat_system: 1,
            cl_chat_reset: 1,
            cl_chat_teamcolors: 1,
            cl_chat_width: 200,
            cl_show_ids: 0,
            cl_message_friend: 1,
            cl_friends_ignore_clan: 1,
            snd_servermessage: 1,
            snd_highlight: 1,
            snd_team_chat: 1,
            snd_chat: 1,
            snd_enable: 1,
            snd_rate: 48000,
            snd_buffer_size: 512,
            snd_enable_music: 0,
            snd_volume: 30,
            snd_chat_volume: 30,
            snd_game_volume: 30,
            snd_ambient_volume: 30,
            snd_background_music_volume: 30,
            snd_nonactive_mute: 0,
            snd_game: 1,
            snd_gun: 1,
            snd_long_pain: 1,
            cl_predict_events: 0,
            gfx_high_detail: 1,
            cl_assets_entities: "default".into(),
            cl_asset_game: "default".into(),
            cl_asset_emoticons: "default".into(),
            cl_asset_particles: "default".into(),
            cl_asset_hud: "default".into(),
            cl_asset_extras: "default".into(),
            cl_spec_cursor: 1,
            cl_overlay_entities: 0,
            cl_text_entities: 1,
            cl_text_entities_size: 100,
            cl_message_system_color: 2817983,
            cl_message_client_color: 9633471,
            cl_message_highlight_color: 65471,
            cl_message_team_color: 5636050,
            cl_message_color: 255,
            cl_message_friend_color: 65425,
            cl_nameplates: 1,
            cl_nameplates_always: 1,
            cl_nameplates_teamcolors: 1,
            cl_nameplates_size: 50,
            cl_nameplates_clan: 0,
            cl_nameplates_clan_size: 30,
            cl_nameplates_ids: 0,
            cl_nameplates_ids_size: 50,
            cl_nameplates_ids_seperate_line: 0,
            cl_nameplates_offset: 30,
            cl_nameplates_own: 0,
            cl_nameplates_friendmark: 0,
            cl_nameplates_strong: 0,
            cl_nameplates_strong_size: 30,
            cl_show_direction: 1,
            cl_direction_size: 30,
            cl_show_others_alpha: 40,
            cl_show_freeze_bars: 1,
            cl_freezebars_alpha_inside_freeze: 0,
            cl_scoreboard_on_death: 1,
            cl_enable_ping_color: 1,
            rs_data_dir: String::new(),
            rs_media_source: String::new(),
            rs_media: 1,
            rs_glass: 1,
            rs_ui_scale: 100,
        }
    }
}

/// One config variable: C++ `SIntConfigVariable`, `SColorConfigVariable` (no alpha;
/// with the darkest lighting: 0.5 for `CFGFLAG_COLLIGHT`, else 0) or
/// `SStringConfigVariable`.
enum Var<'a> {
    Int(&'a mut i32, i32, i32),
    Color(&'a mut u32, f32),
    Str(&'a mut String, usize),
}

impl Config {
    /// The prediction and antiping variables for the predictor.
    pub fn predict_config(&self) -> ddnet::predict::PredictConfig {
        ddnet::predict::PredictConfig {
            predict: self.cl_predict != 0,
            antiping: self.cl_antiping != 0,
            antiping_players: self.cl_antiping_players,
            antiping_grenade: self.cl_antiping_grenade != 0,
            antiping_weapons: self.cl_antiping_weapons != 0,
            antiping_smooth: self.cl_antiping_smooth != 0,
            antiping_gunfire: self.cl_antiping_gunfire != 0,
            antiping_preinput: self.cl_antiping_preinput != 0,
            antiping_limit: self.cl_antiping_limit,
            antiping_percent: self.cl_antiping_percent,
            predict_freeze: self.cl_predict_freeze,
            predict_events: self.cl_predict_events != 0,
            snd_enable: self.snd_enable != 0,
            snd_game: self.snd_game != 0,
            snd_gun: self.snd_gun != 0,
            snd_long_pain: self.snd_long_pain != 0,
        }
    }

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
            "player_color_body" => Color(&mut self.player_color_body, 0.5),
            "player_color_feet" => Color(&mut self.player_color_feet, 0.5),
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
            "cl_predict" => Int(&mut self.cl_predict, 0, 1),
            "cl_antiping_limit" => Int(&mut self.cl_antiping_limit, 0, 500),
            "cl_antiping_percent" => Int(&mut self.cl_antiping_percent, 0, 100),
            "cl_antiping" => Int(&mut self.cl_antiping, 0, 1),
            "cl_antiping_players" => Int(&mut self.cl_antiping_players, 0, 3),
            "cl_antiping_grenade" => Int(&mut self.cl_antiping_grenade, 0, 1),
            "cl_antiping_weapons" => Int(&mut self.cl_antiping_weapons, 0, 1),
            "cl_antiping_smooth" => Int(&mut self.cl_antiping_smooth, 0, 1),
            "cl_antiping_gunfire" => Int(&mut self.cl_antiping_gunfire, 0, 1),
            "cl_antiping_preinput" => Int(&mut self.cl_antiping_preinput, 0, 1),
            "cl_prediction_margin" => Int(&mut self.cl_prediction_margin, 1, 300),
            "cl_predict_freeze" => Int(&mut self.cl_predict_freeze, 0, 2),
            "cl_showchat" => Int(&mut self.cl_showchat, 0, 2),
            "cl_show_chat_friends" => Int(&mut self.cl_show_chat_friends, 0, 1),
            "cl_show_chat_team_members_only" => Int(&mut self.cl_show_chat_team_members_only, 0, 1),
            "cl_show_chat_system" => Int(&mut self.cl_show_chat_system, 0, 1),
            "cl_chat_reset" => Int(&mut self.cl_chat_reset, 0, 1),
            "cl_chat_teamcolors" => Int(&mut self.cl_chat_teamcolors, 0, 1),
            "cl_chat_width" => Int(&mut self.cl_chat_width, 140, 400),
            "cl_show_ids" => Int(&mut self.cl_show_ids, 0, 1),
            "cl_message_friend" => Int(&mut self.cl_message_friend, 0, 1),
            "cl_friends_ignore_clan" => Int(&mut self.cl_friends_ignore_clan, 0, 1),
            "snd_servermessage" => Int(&mut self.snd_servermessage, 0, 1),
            "snd_highlight" => Int(&mut self.snd_highlight, 0, 1),
            "snd_team_chat" => Int(&mut self.snd_team_chat, 0, 1),
            "snd_chat" => Int(&mut self.snd_chat, 0, 1),
            "snd_enable" => Int(&mut self.snd_enable, 0, 1),
            "snd_rate" => Int(&mut self.snd_rate, 5512, 384000),
            "snd_buffer_size" => Int(&mut self.snd_buffer_size, 128, 32768),
            "snd_enable_music" => Int(&mut self.snd_enable_music, 0, 1),
            "snd_volume" => Int(&mut self.snd_volume, 0, 100),
            "snd_chat_volume" => Int(&mut self.snd_chat_volume, 0, 100),
            "snd_game_volume" => Int(&mut self.snd_game_volume, 0, 100),
            "snd_ambient_volume" => Int(&mut self.snd_ambient_volume, 0, 100),
            "snd_background_music_volume" => Int(&mut self.snd_background_music_volume, 0, 100),
            "snd_nonactive_mute" => Int(&mut self.snd_nonactive_mute, 0, 1),
            "snd_game" => Int(&mut self.snd_game, 0, 1),
            "snd_gun" => Int(&mut self.snd_gun, 0, 1),
            "snd_long_pain" => Int(&mut self.snd_long_pain, 0, 1),
            "cl_predict_events" => Int(&mut self.cl_predict_events, 0, 1),
            "gfx_high_detail" => Int(&mut self.gfx_high_detail, 0, 1),
            "cl_assets_entities" => Str(&mut self.cl_assets_entities, 50),
            "cl_asset_game" => Str(&mut self.cl_asset_game, 50),
            "cl_asset_emoticons" => Str(&mut self.cl_asset_emoticons, 50),
            "cl_asset_particles" => Str(&mut self.cl_asset_particles, 50),
            "cl_asset_hud" => Str(&mut self.cl_asset_hud, 50),
            "cl_asset_extras" => Str(&mut self.cl_asset_extras, 50),
            "cl_spec_cursor" => Int(&mut self.cl_spec_cursor, 0, 1),
            "cl_overlay_entities" => Int(&mut self.cl_overlay_entities, 0, 100),
            "cl_text_entities" => Int(&mut self.cl_text_entities, 0, 1),
            "cl_text_entities_size" => Int(&mut self.cl_text_entities_size, 20, 100),
            "cl_message_system_color" => Color(&mut self.cl_message_system_color, 0.0),
            "cl_message_client_color" => Color(&mut self.cl_message_client_color, 0.0),
            "cl_message_highlight_color" => Color(&mut self.cl_message_highlight_color, 0.0),
            "cl_message_team_color" => Color(&mut self.cl_message_team_color, 0.0),
            "cl_message_color" => Color(&mut self.cl_message_color, 0.0),
            "cl_message_friend_color" => Color(&mut self.cl_message_friend_color, 0.0),
            "cl_nameplates" => Int(&mut self.cl_nameplates, 0, 1),
            "cl_nameplates_always" => Int(&mut self.cl_nameplates_always, 0, 1),
            "cl_nameplates_teamcolors" => Int(&mut self.cl_nameplates_teamcolors, 0, 1),
            "cl_nameplates_size" => Int(&mut self.cl_nameplates_size, -50, 100),
            "cl_nameplates_clan" => Int(&mut self.cl_nameplates_clan, 0, 1),
            "cl_nameplates_clan_size" => Int(&mut self.cl_nameplates_clan_size, -50, 100),
            "cl_nameplates_ids" => Int(&mut self.cl_nameplates_ids, 0, 1),
            "cl_nameplates_ids_size" => Int(&mut self.cl_nameplates_ids_size, -50, 100),
            "cl_nameplates_ids_seperate_line" => {
                Int(&mut self.cl_nameplates_ids_seperate_line, 0, 1)
            }
            "cl_nameplates_offset" => Int(&mut self.cl_nameplates_offset, 10, 50),
            "cl_nameplates_own" => Int(&mut self.cl_nameplates_own, 0, 1),
            "cl_nameplates_friendmark" => Int(&mut self.cl_nameplates_friendmark, 0, 1),
            "cl_nameplates_strong" => Int(&mut self.cl_nameplates_strong, 0, 2),
            "cl_nameplates_strong_size" => Int(&mut self.cl_nameplates_strong_size, -50, 100),
            "cl_show_direction" => Int(&mut self.cl_show_direction, 0, 3),
            "cl_direction_size" => Int(&mut self.cl_direction_size, -50, 100),
            "cl_show_others_alpha" => Int(&mut self.cl_show_others_alpha, 0, 100),
            "cl_show_freeze_bars" => Int(&mut self.cl_show_freeze_bars, 0, 1),
            "cl_freezebars_alpha_inside_freeze" => {
                Int(&mut self.cl_freezebars_alpha_inside_freeze, 0, 100)
            }
            "cl_scoreboard_on_death" => Int(&mut self.cl_scoreboard_on_death, 0, 1),
            "cl_enable_ping_color" => Int(&mut self.cl_enable_ping_color, 0, 1),
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
            Var::Color(..) => "?c",
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
            Var::Color(v, darkest) => *v = color_pack(color_parse(&a[0]).unwrap_or(0), darkest),
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
                (Var::Color(v, _), Var::Color(dv, _)) if all || v != dv => format!("{name} {v}"),
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
pub const VAR_NAMES: [&str; 117] = [
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
    "cl_predict",
    "cl_antiping_limit",
    "cl_antiping_percent",
    "cl_antiping",
    "cl_antiping_players",
    "cl_antiping_grenade",
    "cl_antiping_weapons",
    "cl_antiping_smooth",
    "cl_antiping_gunfire",
    "cl_antiping_preinput",
    "cl_prediction_margin",
    "cl_predict_freeze",
    "cl_showchat",
    "cl_show_chat_friends",
    "cl_show_chat_team_members_only",
    "cl_show_chat_system",
    "cl_chat_reset",
    "cl_chat_teamcolors",
    "cl_chat_width",
    "cl_show_ids",
    "cl_message_friend",
    "cl_friends_ignore_clan",
    "snd_servermessage",
    "snd_highlight",
    "snd_team_chat",
    "snd_chat",
    "snd_enable",
    "snd_rate",
    "snd_buffer_size",
    "snd_enable_music",
    "snd_volume",
    "snd_chat_volume",
    "snd_game_volume",
    "snd_ambient_volume",
    "snd_background_music_volume",
    "snd_nonactive_mute",
    "snd_game",
    "snd_gun",
    "snd_long_pain",
    "cl_predict_events",
    "gfx_high_detail",
    "cl_assets_entities",
    "cl_asset_game",
    "cl_asset_emoticons",
    "cl_asset_particles",
    "cl_asset_hud",
    "cl_asset_extras",
    "cl_spec_cursor",
    "cl_overlay_entities",
    "cl_text_entities",
    "cl_text_entities_size",
    "cl_message_system_color",
    "cl_message_client_color",
    "cl_message_highlight_color",
    "cl_message_team_color",
    "cl_message_color",
    "cl_message_friend_color",
    "cl_nameplates",
    "cl_nameplates_always",
    "cl_nameplates_teamcolors",
    "cl_nameplates_size",
    "cl_nameplates_clan",
    "cl_nameplates_clan_size",
    "cl_nameplates_ids",
    "cl_nameplates_ids_size",
    "cl_nameplates_ids_seperate_line",
    "cl_nameplates_offset",
    "cl_nameplates_own",
    "cl_nameplates_friendmark",
    "cl_nameplates_strong",
    "cl_nameplates_strong_size",
    "cl_show_direction",
    "cl_direction_size",
    "cl_show_others_alpha",
    "cl_show_freeze_bars",
    "cl_freezebars_alpha_inside_freeze",
    "cl_scoreboard_on_death",
    "cl_enable_ping_color",
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

/// C++: `ColorHSLA(Value, true).UnclampLighting(Darkest)` then `Pack(Darkest, false)`:
/// what a colour variable stores for a packed number.
fn color_pack(v: u32, darkest: f32) -> u32 {
    let ch = |shift: u32| ((v >> shift) & 0xFF) as f32 / 255.0;
    let (h, s) = (ch(16), ch(8));
    let l = darkest + ch(0) * (1.0 - darkest);
    let l = ((l - darkest) / (1.0 - darkest)).clamp(0.0, 1.0);
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
    /// C++ `CFriends` of friends and foes (`add_friend`, `add_foe`).
    pub friends: ddnet::clients::Friends,
    pub foes: ddnet::clients::Friends,
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
            friends: Default::default(),
            foes: Default::default(),
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
        // C++: CFriends::ConfigSaveCallback
        for (command, list) in [("add_friend", &self.friends), ("add_foe", &self.foes)] {
            for (name, clan) in &list.list {
                text += &format!(
                    "{command} \"{}\" \"{}\"
",
                    escape(name),
                    escape(clan)
                );
            }
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
                c @ ("add_friend" | "add_foe" | "remove_friend" | "remove_foe")
                    if parse_args(&args, "s?s", &mut a) =>
                {
                    let list = if c.ends_with("friend") {
                        &mut self.friends
                    } else {
                        &mut self.foes
                    };
                    let clan = a.get(1).map_or("", String::as_str);
                    if c.starts_with("add") {
                        list.add(&a[0], clan);
                    } else {
                        list.remove(&a[0], clan);
                    }
                }
                "bind" | "unbind" => {}
                _ => self.skipped += 1,
            }
        }
    }
}
