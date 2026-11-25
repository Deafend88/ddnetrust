//! The Dynamic Island (PLAN §10.3, 7.9): one capsule top centre that shows the race
//! timer and springs open for events (finish/record, broadcasts). What it shows is
//! decided by ported C++ logic: the race timer (`CHud::RenderGameTimer`), the finish
//! time and checkpoint difference (`CHud::OnMessage`, `RenderDDRaceEffects`),
//! broadcasts (`CBroadcast`), the pause text (`RenderPauseNotification`) and the
//! spectator text (`RenderSpectatorHud`). Priority: finish > broadcast > timer.

use std::sync::Arc;

use ddnet::map::ints_to_str;
use ddnet::protocol::generated::{self as g, GameMsg};
use ddnet::protocol::snapshot::Item;
use egui::epaint::TextShape;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, CornerRadius, Galley, Rect, Sense, Ui, pos2, vec2};

use crate::anim::Spring;
use crate::hud::{self, Anchor, HudConfig, HudData, Placement, Widget};
use crate::icons;
use crate::lyrics::{self, Lyrics};
use crate::media::{self, Command, Media, time};
use crate::style::{
    Glass, HUD_SHADOW, Weight, color, display, font, icon_font, motion, radius, size, space,
};
use crate::timefmt::{TimeFormat, str_time, str_time_float};
use crate::widgets as w;

/// C++ `Client()->GameTickSpeed()`.
const TICK_SPEED: i32 = 50;
/// C++: the finish time and checkpoint difference show for 6 s.
pub const EFFECT_TICKS: i32 = TICK_SPEED * 6;
/// C++ `CBroadcast::DoBroadcast`: 10 s.
pub const BROADCAST_TICKS: i32 = TICK_SPEED * 10;
/// C++ `TEAM_SPECTATORS`, `SPEC_FREEVIEW`.
const TEAM_SPECTATORS: i32 = -1;
const SPEC_FREEVIEW: i32 = -1;

/// The race timer. C++: `CHud::RenderGameTimer` (hud.cpp:118).
#[derive(Clone, Debug, PartialEq)]
pub enum Timer {
    /// C++ draws "Sudden Death" instead (`RenderSuddenDeath`).
    SuddenDeath,
    /// C++ keeps the width of the timer's class ("00:00" .. "000d 00:00:00") so it does
    /// not jitter; tabular digits (style.rs) give the same.
    Clock {
        text: String,
        /// The last 60 s of a time limit are red, the last 10 s blink (alpha 0.5).
        warn: bool,
        blink: bool,
    },
}

/// C++ `CHud::RenderGameTimer`; `now` is the wall clock in seconds (C++ `time()`).
pub fn timer(info: &g::ObjGameInfo, tick: i32, now: f64) -> Timer {
    if info.game_state_flags & g::GAMESTATEFLAG_SUDDENDEATH != 0 {
        return Timer::SuddenDeath;
    }
    let limit = info.time_limit != 0 && info.warmup_timer <= 0;
    let time = if limit {
        let t = (info.time_limit.wrapping_mul(60))
            .wrapping_sub(tick.wrapping_sub(info.round_start_tick) / TICK_SPEED);
        if info.game_state_flags & g::GAMESTATEFLAG_GAMEOVER != 0 {
            0
        } else {
            t
        }
    } else if info.game_state_flags & g::GAMESTATEFLAG_RACETIME != 0 {
        // the warmup timer is negative here, so old clients show no warmup
        tick.wrapping_add(info.warmup_timer) / TICK_SPEED
    } else {
        tick.wrapping_sub(info.round_start_tick) / TICK_SPEED
    };
    let warn = limit && time <= 60;
    Timer::Clock {
        text: str_time(time as i64 * 100, TimeFormat::Days),
        warn,
        // C++: (2 * time() / time_freq()) % 2
        blink: warn && time <= 10 && (2.0 * now) as i64 % 2 != 0,
    }
}

/// Which record a finish set (`Sv_RaceFinish` for the local player).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Record {
    #[default]
    None,
    Personal,
    Server,
}

/// What `RenderDDRaceEffects` shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// "Finish time: <time>" and, when not 0, the difference to the previous best
    /// ("-00.36" faster, "+00.36" slower).
    Finish {
        time: String,
        diff: Option<(String, bool)>,
        record: Record,
    },
    /// The checkpoint difference; the sign picks C++'s colour (green, red, white).
    Checkpoint { text: String, sign: i8 },
}

/// The race messages' state. C++: `CHud::m_DDRaceTime` and the fields next to it.
#[derive(Clone, Debug, Default)]
pub struct Race {
    time: i32,
    show_finish: bool,
    cp_diff: f32,
    cp_tick: i32,
    finish_diff: f32,
    finish_tick: i32,
    record: Record,
    record_tick: i32,
}

impl Race {
    /// C++ `CHud::OnMessage` for `Sv_DDRaceTime`(+Legacy); `Sv_RaceFinish` of the local
    /// player adds its record flags. `Sv_RecordLegacy` on old DDRace servers is not
    /// ported (DECISIONS.md).
    pub fn on_message(&mut self, msg: &GameMsg, local: Option<i32>, tick: i32) {
        let (time, check, finish) = match msg {
            GameMsg::SvDDRaceTime(m) => (m.time, m.check, m.finish),
            GameMsg::SvDDRaceTimeLegacy(m) => (m.time, m.check, m.finish),
            GameMsg::SvRaceFinish(m) if Some(m.client_id) == local => {
                self.record = if m.record_server != 0 {
                    Record::Server
                } else if m.record_personal != 0 {
                    Record::Personal
                } else {
                    Record::None
                };
                self.record_tick = tick;
                return;
            }
            _ => return,
        };
        self.time = time;
        self.show_finish = finish != 0;
        if self.show_finish {
            self.finish_diff = check as f32 / 100.0;
            self.finish_tick = tick;
        } else {
            self.cp_diff = check as f32 / 100.0;
            self.cp_tick = tick;
        }
    }

    /// C++ `CHud::RenderDDRaceEffects`' decision (its 2 s fade is the island's own
    /// transition here).
    pub fn effect(&self, tick: i32, show_cp_diff: bool) -> Option<Effect> {
        if self.time == 0 {
            return None;
        }
        let format = TimeFormat::HoursCentisecs;
        let signed = |d: f32| {
            let sign = if d < 0.0 { '-' } else { '+' };
            format!("{sign}{}", str_time_float(d.abs(), format))
        };
        if self.show_finish && self.finish_tick.wrapping_add(EFFECT_TICKS) > tick {
            Some(Effect::Finish {
                time: str_time(self.time as i64, format),
                diff: (self.finish_diff != 0.0)
                    .then(|| (signed(self.finish_diff), self.finish_diff < 0.0)),
                record: if self.record_tick == self.finish_tick {
                    self.record
                } else {
                    Record::None
                },
            })
        } else if show_cp_diff
            && !self.show_finish
            && self.cp_tick.wrapping_add(EFFECT_TICKS) > tick
        {
            let d = self.cp_diff;
            Some(Effect::Checkpoint {
                text: signed(d),
                sign: if d > 0.0 {
                    1
                } else if d < 0.0 {
                    -1
                } else {
                    0
                },
            })
        } else {
            None
        }
    }
}

/// C++ `CBroadcast`: the last server broadcast, shown for 10 s.
#[derive(Clone, Debug, Default)]
pub struct Broadcast {
    text: String,
    until: i32,
}

impl Broadcast {
    pub fn on_message(&mut self, msg: &GameMsg, tick: i32) {
        if let GameMsg::SvBroadcast(m) = msg {
            self.text = m.message.clone();
            self.until = tick.wrapping_add(BROADCAST_TICKS);
        }
    }

    /// The text while it shows. Servers pad broadcasts with blank lines and spaces to
    /// place them on the screen; the island trims that.
    pub fn shown(&self, tick: i32) -> Option<&str> {
        let text = self.text.trim();
        (self.until.wrapping_sub(tick) > 0 && !text.is_empty()).then_some(text)
    }
}

/// The spectator text of `RenderSpectatorHud` when C++'s `m_SpecInfo.m_Active` holds:
/// the local player is in the spectators team, or paused / spectating (DDNetPlayer).
pub fn spectating(items: &[Item]) -> Option<String> {
    let local = items.iter().find_map(|it| {
        (it.type_id == g::NETOBJTYPE_PLAYERINFO)
            .then(|| g::ObjPlayerInfo::from_ints(&it.data))
            .filter(|p| p.local != 0)
    })?;
    let flags = (items.iter())
        .find(|it| it.type_id == g::NETOBJTYPE_DDNETPLAYER && it.id == local.client_id)
        .map_or(0, |it| g::ObjDDNetPlayer::from_ints(&it.data).flags);
    let paused = flags & (g::EXPLAYERFLAG_PAUSED | g::EXPLAYERFLAG_SPEC) != 0;
    if local.team != TEAM_SPECTATORS && !paused {
        return None;
    }
    let target = (items.iter())
        .find(|it| it.type_id == g::NETOBJTYPE_SPECTATORINFO)
        .map_or(SPEC_FREEVIEW, |it| {
            g::ObjSpectatorInfo::from_ints(&it.data).spectator_id
        });
    if target == SPEC_FREEVIEW {
        return Some("Free-View".into());
    }
    let name = (items.iter())
        .find(|it| it.type_id == g::NETOBJTYPE_CLIENTINFO && it.id == target)
        .and_then(|it| ints_to_str(&g::ObjClientInfo::from_ints(&it.data).name))
        .unwrap_or_default();
    Some(format!("Following {name}"))
}

/// Whether C++ shows "Game paused" (`RenderPauseNotification`).
pub fn paused(info: &g::ObjGameInfo) -> bool {
    let flags = info.game_state_flags;
    flags & g::GAMESTATEFLAG_PAUSED != 0 && flags & g::GAMESTATEFLAG_GAMEOVER == 0
}

/// What the island shows.
#[derive(Clone, Debug, PartialEq)]
pub enum View {
    /// Idle: the shown items in their configured order.
    Idle(Vec<Part>),
    /// The finish time (an `Effect::Finish`).
    Finish(Effect),
    /// A server broadcast; the timer stays at its end (C++ shows both).
    Broadcast { text: String, timer: Option<Timer> },
    /// The track changed: cover, title, artist.
    Track(NowPlaying),
    /// The expanded media player (F7).
    Player(Player),
}

/// An item of the idle row.
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    /// "Game paused" or the spectator text, with its icon.
    Status(&'static str, String),
    Timer(Timer),
    /// The checkpoint difference (an `Effect::Checkpoint`).
    Checkpoint(Effect),
    /// The mini player: cover and equalizer.
    Media(NowPlaying),
}

/// A track as the island shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub cover: Option<egui::TextureId>,
}

/// The player's track: the track, its timeline (position, duration in seconds) and
/// whether previous, next and seek work.
pub type PlayerTrack = (NowPlaying, Option<(f64, f64)>, [bool; 3]);

/// The expanded player: the track (none: nothing plays) with its timeline in seconds
/// and controls, and the sessions to choose from.
#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    pub track: Option<PlayerTrack>,
    pub sessions: Vec<String>,
    pub source: Option<String>,
    /// Synced lyrics while they are on.
    pub lyrics: Option<Lyrics>,
    /// The app's volume (0..1), when it has an audio session.
    pub volume: Option<f32>,
}

/// The first active event (they come in priority order), else the idle row if it has
/// anything; `None`: nothing to show.
pub fn view(events: Vec<View>, idle: Vec<Part>) -> Option<View> {
    let idle = (!idle.is_empty()).then_some(View::Idle(idle));
    events.into_iter().next().or(idle)
}

/// The island's items (`hud_set island <key> 0|1`, `hud_set island order "<key>,..."`):
/// the idle row's parts in their order, and the events, whose order is their priority.
/// First values from C++ where it has a switch (`HudConfig::import_cpp`).
pub const ITEMS: [hud::Item; 7] = [
    ("status", true, "Pause and spectator status"),
    ("timer", true, "Race timer"),
    ("checkpoint", true, "Checkpoint difference"),
    ("media", true, "Now playing"),
    ("finish", true, "Finish and record"),
    ("broadcasts", true, "Broadcasts"),
    ("track", true, "Track change"),
];

/// Where synced lyrics show (`hud_set island lyrics_place island|card|off`): as subtitles
/// under the island (default), in the "Now playing" card, or nowhere (then nothing is
/// fetched; otherwise the playing track's title and artist go to lrclib.net).
pub const LYRICS_PLACES: [(&str, &str); 3] = [
    ("island", "Under the island"),
    ("card", "In the card"),
    ("off", "Off"),
];

/// The configured lyrics place (one of `LYRICS_PLACES`' keys).
pub fn lyrics_place(config: &HudConfig) -> &'static str {
    let v = config.value("island", "lyrics_place").unwrap_or("island");
    LYRICS_PLACES
        .iter()
        .find(|p| p.0 == v.trim())
        .map_or("island", |p| p.0)
}

/// How long a track change shows (the concept's 3 s; C++ has no such notice).
pub const TRACK_TICKS: i32 = TICK_SPEED * 3;

/// The island widget: the race state, the broadcast, the track and the springs.
#[derive(Default)]
pub struct Island {
    race: Race,
    broadcast: Broadcast,
    /// The last track (title, artist) and the tick its change shows until.
    track: Option<(String, String)>,
    track_until: i32,
    cover: CoverCache,
    anim: Anim,
    subtitles: Subtitles,
    /// The last frame's time (seconds).
    last: Option<f64>,
}

/// The animation state: the size and corner springs, the content shown and its fade.
#[derive(Default)]
pub struct Anim {
    shown: Option<View>,
    width: Spring,
    height: Spring,
    corner: Spring,
    content: Spring,
    opacity: Spring,
    /// The idle size: events are at least this big (they expand the island).
    idle_size: egui::Vec2,
    /// The progress bar's position while it is dragged (0..1).
    seeking: Option<f32>,
    /// The player's content height as last drawn (its size follows its content).
    player_h: Option<f32>,
    /// The player's lyrics view is open (its "Lyrics" button).
    player_lyrics: bool,
    /// The volume slider's value while it is dragged.
    volume: Option<f32>,
    /// When the volume slider opened.
    volume_open: Option<f64>,
    /// The island's rect as last drawn (the subtitles go under it).
    pub rect: Option<Rect>,
    last: Option<f64>,
}

/// The island's top offset, the broadcast's wrap width, the player's width (points).
const TOP: i32 = 16;
const WRAP: f32 = 480.0;
pub const PLAYER_WIDTH: f32 = 360.0;
/// Damping of the size springs: a small overshoot (about 3 %), the island's motion.
const ZETA: f32 = 0.75;
/// The equalizer: bars, their width and gap.
const EQ_BARS: usize = 4;
const EQ_BAR: f32 = 3.0;
const EQ_GAP: f32 = space::XS;

/// A laid-out view: a lead (icon or cover), one or more rows, an optional trailing timer;
/// or (idle) a row of elements.
struct Laid {
    lead: Option<Lead>,
    rows: Vec<Arc<Galley>>,
    /// The width of the rows (or of the elements).
    width: f32,
    trail: Option<Arc<Galley>>,
    elems: Vec<Elem>,
}

enum Lead {
    Icon(Arc<Galley>),
    Cover(Option<egui::TextureId>),
}

impl Lead {
    fn size(&self) -> egui::Vec2 {
        match self {
            Lead::Icon(g) => g.size(),
            Lead::Cover(_) => egui::Vec2::splat(size::COVER),
        }
    }
}

/// An element of the idle row.
enum Elem {
    Text(Arc<Galley>),
    /// Mini cover and equalizer.
    Media(Option<egui::TextureId>, bool),
}

impl Elem {
    fn size(&self) -> egui::Vec2 {
        match self {
            Elem::Text(g) => g.size(),
            Elem::Media(..) => vec2(size::ICON_HUD + space::S + eq_width(), size::ICON_HUD),
        }
    }
}

fn eq_width() -> f32 {
    EQ_BARS as f32 * EQ_BAR + (EQ_BARS - 1) as f32 * EQ_GAP
}

fn job(spans: &[(&str, f32, Weight, Color32)], wrap: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    for &(text, size, weight, color) in spans {
        // large text (timer, finish time, track title) in Inter Display
        let f = if size >= size::LABEL && weight != Weight::Regular {
            display(size, weight)
        } else {
            font(size, weight)
        };
        job.append(text, 0.0, TextFormat::simple(f, color));
    }
    job.wrap.max_width = wrap;
    job
}

/// The timer's text and colour: red in the last minute of a time limit, half
/// transparent when blinking.
fn timer_text(timer: &Timer) -> (String, Color32) {
    match timer {
        Timer::SuddenDeath => ("Sudden Death".into(), color::TEXT),
        Timer::Clock { text, warn, blink } => {
            let c = if *warn { color::DANGER } else { color::TEXT };
            let c = if *blink { c.gamma_multiply(0.5) } else { c };
            (text.clone(), c)
        }
    }
}

fn lay_out(ui: &Ui, view: &View, weak: Color32) -> Laid {
    let ctx = ui.ctx();
    let galley = |j: LayoutJob| ctx.fonts_mut(|f| f.layout_job(j));
    let icon = |glyph: &str, size: f32, c: Color32| {
        Some(Lead::Icon(ctx.fonts_mut(|f| {
            f.layout_no_wrap(glyph.into(), icon_font(size), c)
        })))
    };
    let laid = |lead, rows: Vec<Arc<Galley>>, trail| Laid {
        lead,
        width: rows.iter().map(|r| r.size().x).fold(0.0, f32::max),
        rows,
        trail,
        elems: Vec::new(),
    };
    let inf = f32::INFINITY;
    let timer_galley = |t: &Timer| {
        let (s, c) = timer_text(t);
        galley(job(&[(&s, size::TITLE, Weight::SemiBold, c)], inf))
    };
    match view {
        View::Idle(parts) => {
            // one row in the configured order; the status icon is a span in the icon
            // font; digits are tabular, so the timer's width only changes with its
            // class (like C++'s)
            let elems: Vec<Elem> = (parts.iter())
                .map(|part| match part {
                    Part::Status(glyph, text) => {
                        let mut j = LayoutJob::default();
                        j.append(glyph, 0.0, TextFormat::simple(icon_font(size::ICON), weak));
                        let f = font(size::BODY, Weight::Medium);
                        j.append(text, space::S, TextFormat::simple(f, weak));
                        Elem::Text(galley(j))
                    }
                    Part::Timer(t) => Elem::Text(timer_galley(t)),
                    Part::Checkpoint(Effect::Checkpoint { text, sign }) => {
                        let c = match sign {
                            1 => color::DANGER,
                            -1 => color::SUCCESS,
                            _ => color::TEXT,
                        };
                        Elem::Text(galley(job(&[(text, size::BODY, Weight::Medium, c)], inf)))
                    }
                    Part::Checkpoint(_) => Elem::Text(galley(LayoutJob::default())),
                    Part::Media(m) => Elem::Media(m.cover, m.playing),
                })
                .collect();
            let gaps = elems.len().saturating_sub(1) as f32 * space::M;
            Laid {
                lead: None,
                rows: Vec::new(),
                width: elems.iter().map(|e| e.size().x).sum::<f32>() + gaps,
                trail: None,
                elems,
            }
        }
        View::Finish(Effect::Finish { time, diff, record }) => {
            let (glyph, title) = match record {
                Record::Server => (icons::TROPHY, "Server record  "),
                Record::Personal => (icons::TROPHY, "Personal best  "),
                Record::None => (icons::CHECK_CIRCLE, "Finish time  "),
            };
            let mut rows = vec![galley(job(
                &[
                    (title, size::BODY, Weight::Regular, weak),
                    (time, size::TITLE, Weight::SemiBold, color::TEXT),
                ],
                inf,
            ))];
            if let Some((d, faster)) = diff {
                let c = if *faster {
                    color::SUCCESS
                } else {
                    color::DANGER
                };
                rows.push(galley(job(
                    &[
                        (d, size::BODY, Weight::Medium, c),
                        ("  vs. your best", size::BODY, Weight::Regular, weak),
                    ],
                    inf,
                )));
            }
            laid(icon(glyph, size::ICON_LARGE, color::SUCCESS), rows, None)
        }
        View::Finish(_) | View::Player(_) => laid(None, Vec::new(), None),
        View::Broadcast { text, timer } => {
            let row = galley(job(
                &[(text, size::BODY, Weight::Medium, color::TEXT)],
                WRAP,
            ));
            let lead = icon(icons::MEGAPHONE, size::ICON, color::TEXT);
            laid(lead, vec![row], timer.as_ref().map(timer_galley))
        }
        View::Track(t) => {
            let rows = vec![
                galley(job(
                    &[(&t.title, size::LABEL, Weight::SemiBold, color::TEXT)],
                    WRAP,
                )),
                galley(job(&[(&t.artist, size::BODY, Weight::Regular, weak)], WRAP)),
            ];
            laid(Some(Lead::Cover(t.cover)), rows, None)
        }
    }
}

/// The padded size of a laid-out view and whether it is one line (a pill).
fn measure(l: &Laid) -> (egui::Vec2, bool) {
    if !l.elems.is_empty() {
        let h = l.elems.iter().map(|e| e.size().y).fold(0.0, f32::max) + 2.0 * space::S;
        return (
            vec2((l.width + 2.0 * space::XL).max(h + space::XXL), h),
            true,
        );
    }
    let rows_h: f32 = l.rows.iter().map(|r| r.size().y).sum();
    let one_line = l.rows.len() == 1 && l.rows[0].rows.len() == 1;
    let lead = l.lead.as_ref().map_or(egui::Vec2::ZERO, Lead::size);
    let lead_w = if l.lead.is_some() {
        lead.x + space::M
    } else {
        0.0
    };
    let trail_w = l.trail.as_ref().map_or(0.0, |t| t.size().x + space::XL);
    let py = if one_line && l.lead.as_ref().is_none_or(|l| matches!(l, Lead::Icon(_))) {
        space::S
    } else {
        space::M
    };
    let h = rows_h.max(lead.y) + 2.0 * py;
    // a pill is at least as wide as tall
    let w = (lead_w + l.width + trail_w + 2.0 * space::XL).max(h + space::XXL);
    (vec2(w, h), one_line)
}

/// The player's first size (then its drawn content's): the track row, the progress row
/// when there is a timeline, the controls; L inside.
fn player_size(p: &Player) -> egui::Vec2 {
    let timeline = p.track.as_ref().is_some_and(|t| t.1.is_some());
    let mut h = 2.0 * space::L + size::COVER_PLAYER + space::M + size::CONTROL;
    if timeline {
        h += space::M + size::CAPTION + space::XS;
    }
    vec2(PLAYER_WIDTH, h)
}

/// Whether `a` turning into `b` updates in place (no fade): the idle row ticking, the
/// same broadcast with its timer ticking, the player's position moving.
fn in_place(a: &Option<View>, b: &Option<View>) -> bool {
    match (a, b) {
        (Some(View::Idle(_)), Some(View::Idle(_))) => true,
        (Some(View::Player(_)), Some(View::Player(_))) => true,
        (Some(View::Broadcast { text: x, .. }), Some(View::Broadcast { text: y, .. })) => x == y,
        _ => false,
    }
}

impl Anim {
    /// Springs towards `view` and draws the island; `now` in seconds. Returns what the
    /// user asked of the media player (the expanded player's controls).
    pub fn show(
        &mut self,
        ui: &mut Ui,
        glass: Glass,
        view: Option<View>,
        now: f64,
    ) -> Option<Command> {
        let dt = self.last.map_or(0.0, |l| (now - l).clamp(0.0, 0.1) as f32);
        self.last = Some(now);
        let weak = glass.text_secondary();
        // updates in place; any other change swaps once the old content has faded out,
        // while the size already moves
        if in_place(&self.shown, &view) || self.content.value < 0.02 {
            self.shown.clone_from(&view);
        }
        let hidden = self.opacity.value < 0.02;
        if let Some(v) = &view {
            let (s, pill) = match v {
                View::Player(p) => {
                    let s = player_size(p);
                    let h = self.player_h.map_or(s.y, |h| h + 2.0 * space::L);
                    (vec2(s.x, h), false)
                }
                _ => measure(&lay_out(ui, v, weak)),
            };
            let s = if matches!(v, View::Idle(_)) {
                self.idle_size = s;
                s
            } else {
                // an event expands the island: never smaller than the idle state
                s.max(self.idle_size)
            };
            if hidden {
                // grow out of a small capsule
                self.width = Spring::new(s.y);
                self.height = Spring::new(s.y * 0.8);
                self.corner = Spring::new(s.y * 0.4);
            }
            let r = if pill { s.y / 2.0 } else { radius::CARD as f32 };
            self.width.update_damped(s.x, motion::SPRING, ZETA, dt);
            self.height.update_damped(s.y, motion::SPRING, ZETA, dt);
            self.corner.update_damped(r, motion::SPRING, ZETA, dt);
        } else if hidden {
            self.shown = None;
            self.rect = None;
            return None;
        }
        let on = view.is_some() as i32 as f32;
        self.opacity.update(on, motion::NORMAL, dt);
        let settled = (in_place(&self.shown, &view) || self.shown == view) as i32 as f32;
        self.content.update(on * settled, motion::FAST, dt);

        let size = vec2(self.width.value.max(0.0), self.height.value.max(0.0));
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        self.rect = Some(rect);
        let opacity = self.opacity.value.clamp(0.0, 1.0);
        let radius = self.corner.value.clamp(0.0, rect.height() / 2.0);
        let p = ui.painter().clone();
        let cr = CornerRadius::same(radius.round() as u8);
        let mut shadow = HUD_SHADOW;
        shadow.color = shadow.color.gamma_multiply(opacity);
        p.add(shadow.as_shape(rect, cr));
        // the blur cannot fade, so it goes with the shape's first and last frames
        if glass != Glass::Off && opacity > 0.5 {
            let liquid = glass == Glass::Full;
            p.add(crate::glass::shape(rect.shrink(1.0), radius - 1.0, liquid));
        }
        let fill = if glass == Glass::Full {
            Glass::liquid_fill()
        } else {
            glass.hud_fill()
        };
        p.rect_filled(rect, cr, fill.gamma_multiply(opacity));

        let alpha = (self.content.value * opacity).clamp(0.0, 1.0);
        match &self.shown {
            Some(View::Player(player)) => {
                let inner = Rect::from_min_size(rect.min, player_size(player));
                let mut child =
                    ui.new_child(egui::UiBuilder::new().max_rect(inner.shrink(space::L)));
                child.set_clip_rect(rect.intersect(ui.clip_rect()));
                child.set_opacity(alpha);
                let player = player.clone();
                let mut controls = Controls {
                    seeking: &mut self.seeking,
                    lyrics_open: &mut self.player_lyrics,
                    volume: &mut self.volume,
                    volume_open: &mut self.volume_open,
                    card: rect,
                };
                let command = player_ui(&mut child, &player, Some(&mut controls), 0, weak);
                self.player_h = Some(child.min_rect().height());
                return command;
            }
            Some(shown) => {
                let laid = lay_out(ui, shown, weak);
                paint_content(&p.with_clip_rect(rect), rect, &laid, alpha, now);
            }
            None => {}
        }
        None
    }
}

/// A cover: the image, or a music note on a raised square while there is none.
fn paint_cover(p: &egui::Painter, rect: Rect, cover: Option<egui::TextureId>, alpha: f32) {
    let cr = CornerRadius::same(radius::COVER);
    match cover {
        Some(tex) => {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            let tint = Color32::WHITE.gamma_multiply(alpha);
            p.add(egui::epaint::RectShape::filled(rect, cr, tint).with_texture(tex, uv));
        }
        None => {
            p.rect_filled(rect, cr, color::SURFACE_HOVER.gamma_multiply(alpha));
            let g = p.layout_no_wrap(
                icons::MUSIC_NOTES.into(),
                icon_font(rect.height() * 0.55),
                color::TEXT.gamma_multiply(alpha),
            );
            p.galley(rect.center() - g.size() / 2.0, g, Color32::TRANSPARENT);
        }
    }
}

/// The equalizer in `rect`: bars moving while the track plays, low while paused.
fn paint_equalizer(p: &egui::Painter, rect: Rect, playing: bool, alpha: f32, now: f64) {
    let h = rect.height();
    for i in 0..EQ_BARS {
        let level = if playing {
            // each bar on its own rate and phase
            let t = now as f32 * (5.0 + 1.7 * i as f32) + i as f32 * 1.3;
            0.3 + 0.7 * (0.5 + 0.5 * t.sin())
        } else {
            0.2
        };
        let x = rect.left() + i as f32 * (EQ_BAR + EQ_GAP);
        let bar = Rect::from_min_max(
            pos2(x, rect.bottom() - h * level),
            pos2(x + EQ_BAR, rect.bottom()),
        );
        let c = color::ACCENT.gamma_multiply(alpha);
        p.rect_filled(bar, CornerRadius::same(radius::PILL), c);
    }
}

/// The lead left of the rows, the trailing timer right, the group centred; or the idle
/// elements left to right, M apart.
fn paint_content(p: &egui::Painter, rect: Rect, l: &Laid, alpha: f32, now: f64) {
    let text = |pos, g: &Arc<Galley>| {
        p.add(TextShape::new(pos, g.clone(), Color32::TRANSPARENT).with_opacity_factor(alpha));
    };
    if !l.elems.is_empty() {
        let mut x = rect.center().x - l.width / 2.0;
        for e in &l.elems {
            let s = e.size();
            let at = pos2(x, rect.center().y - s.y / 2.0);
            match e {
                Elem::Text(g) => text(at, g),
                Elem::Media(cover, playing) => {
                    let c = Rect::from_min_size(at, egui::Vec2::splat(size::ICON_HUD));
                    paint_cover(p, c, *cover, alpha);
                    let eq = Rect::from_min_size(
                        pos2(c.right() + space::S, rect.center().y - size::ICON / 2.0),
                        vec2(eq_width(), size::ICON),
                    );
                    paint_equalizer(p, eq, *playing, alpha, now);
                }
            }
            x += s.x + space::M;
        }
        return;
    }
    let lead = l.lead.as_ref().map_or(egui::Vec2::ZERO, Lead::size);
    let lead_w = if l.lead.is_some() {
        lead.x + space::M
    } else {
        0.0
    };
    let trail_w = l.trail.as_ref().map_or(0.0, |t| t.size().x + space::XL);
    let rows_h: f32 = l.rows.iter().map(|r| r.size().y).sum();
    let left = rect.center().x - (lead_w + l.width + trail_w) / 2.0;
    match &l.lead {
        Some(Lead::Icon(g)) => text(pos2(left, rect.center().y - g.size().y / 2.0), g),
        Some(Lead::Cover(c)) => {
            let r = Rect::from_min_size(pos2(left, rect.center().y - lead.y / 2.0), lead);
            paint_cover(p, r, *c, alpha);
        }
        None => {}
    }
    let mut y = rect.center().y - rows_h / 2.0;
    for row in &l.rows {
        text(pos2(left + lead_w, y), row);
        y += row.size().y;
    }
    if let Some(t) = &l.trail {
        let x = left + lead_w + l.width + space::XL;
        text(pos2(x, rect.center().y - t.size().y / 2.0), t);
    }
}

/// Three plain dots (iOS "more"), no background, a `size::CONTROL` square to click.
fn dots_button(ui: &mut Ui) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::Vec2::splat(size::CONTROL), Sense::click());
    let c = if r.hovered() {
        color::TEXT
    } else {
        color::TEXT.gamma_multiply(0.8)
    };
    for dx in [-1.0, 0.0, 1.0] {
        let at = rect.center() + vec2(dx * 6.0, 0.0);
        ui.painter()
            .circle_filled(at, 2.0, c.gamma_multiply(ui.opacity()));
    }
    r
}

/// A source's app id as a name: "Spotify.exe" -> "Spotify", a packaged app's
/// "Publisher.App_hash!App" -> "App".
pub fn source_name(id: &str) -> &str {
    let name = id.rsplit('!').next().unwrap_or(id);
    let name = name.strip_suffix(".exe").unwrap_or(name);
    name.rsplit('.')
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or(name)
}

/// The expanded player (SPEC §2, iOS style): cover, title and artist with the "⋯" source
/// menu; elapsed, progress bar (click or drag to seek), remaining; previous, play/pause,
/// next. Without a track: a hint and the source menu.
/// The F7 player's interactive state.
pub struct Controls<'a> {
    /// The progress bar's position while it is dragged.
    pub seeking: &'a mut Option<f32>,
    /// The lyrics view is open.
    pub lyrics_open: &'a mut bool,
    /// The volume slider's value while it is dragged.
    pub volume: &'a mut Option<f32>,
    /// When the volume slider opened (`None`: closed).
    pub volume_open: &'a mut Option<f64>,
    /// The player card's rect (the slider slides out of its side).
    pub card: Rect,
}

/// `controls`: the interactive F7 player (source menu, lyrics button, seeking, buttons,
/// volume); without them a display-only card (the "Now playing" widget). `lyric_lines`:
/// how many lyrics lines show (0: none, 1: the current line, 3: previous, current,
/// next).
pub fn player_ui(
    ui: &mut Ui,
    p: &Player,
    mut controls: Option<&mut Controls>,
    lyric_lines: usize,
    weak: Color32,
) -> Option<Command> {
    let mut command = None;
    let interactive = controls.is_some();
    let seeking = controls.as_ref().and_then(|c| *c.seeking);
    // the source menu, top right
    let menu = |ui: &mut Ui, command: &mut Option<Command>| {
        if !interactive {
            return;
        }
        let r = dots_button(ui).on_hover_text("Source");
        egui::Popup::menu(&r).show(|ui| {
            w::text(ui, "Source", size::CAPTION, Weight::Regular, weak);
            if ui
                .selectable_label(p.source.is_none(), "Automatic")
                .clicked()
            {
                *command = Some(Command::Source(None));
            }
            for s in &p.sessions {
                let on = p.source.as_deref() == Some(s.as_str());
                if ui.selectable_label(on, source_name(s)).clicked() {
                    *command = Some(Command::Source(Some(s.clone())));
                }
            }
        });
    };
    ui.spacing_mut().item_spacing = vec2(space::M, space::M);
    let Some((track, timeline, [can_prev, can_next, can_seek])) = &p.track else {
        ui.horizontal(|ui| {
            let (rect, _) =
                ui.allocate_exact_size(egui::Vec2::splat(size::COVER_PLAYER), Sense::hover());
            paint_cover(ui.painter(), rect, None, ui.opacity());
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = space::XS;
                w::text(
                    ui,
                    "Nothing playing",
                    size::LABEL,
                    Weight::SemiBold,
                    color::TEXT,
                );
                w::text(
                    ui,
                    "Play music in any app",
                    size::BODY,
                    Weight::Regular,
                    weak,
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                menu(ui, &mut command)
            });
        });
        return command;
    };
    ui.horizontal(|ui| {
        let (rect, _) =
            ui.allocate_exact_size(egui::Vec2::splat(size::COVER_PLAYER), Sense::hover());
        paint_cover(ui.painter(), rect, track.cover, ui.opacity());
        let lyrics_open = controls.as_mut().map(|c| &mut *c.lyrics_open);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            menu(ui, &mut command);
            // the lyrics view, when lyrics are on
            if let Some(open) = lyrics_open.filter(|_| p.lyrics.is_some()) {
                let c = if *open { color::ACCENT } else { color::TEXT };
                let t = egui::RichText::new(icons::LYRICS)
                    .font(icon_font(size::ICON))
                    .color(c);
                let b = egui::Button::new(t)
                    .frame(false)
                    .min_size(egui::Vec2::splat(size::CONTROL));
                if ui.add(b).on_hover_text("Lyrics").clicked() {
                    *open = !*open;
                }
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = space::XS;
                ui.add_space(space::S);
                let title = egui::RichText::new(&track.title)
                    .font(display(size::LABEL, Weight::SemiBold))
                    .color(color::TEXT);
                ui.add(egui::Label::new(title).truncate());
                let artist = egui::RichText::new(&track.artist)
                    .font(font(size::BODY, Weight::Regular))
                    .color(weak);
                ui.add(egui::Label::new(artist).truncate());
            });
        });
    });
    if let Some((pos, dur)) = timeline {
        // elapsed, the bar, remaining
        ui.spacing_mut().item_spacing.y = space::XS;
        let (row, _) = ui.allocate_exact_size(
            vec2(ui.available_width(), size::CAPTION + space::XS),
            Sense::hover(),
        );
        let caption = |s: String| {
            ui.fonts_mut(|f| f.layout_no_wrap(s, display(size::CAPTION, Weight::Medium), weak))
        };
        let frac_now = if *dur > 0.0 { (pos / dur) as f32 } else { 0.0 };
        let frac = seeking.unwrap_or(frac_now).clamp(0.0, 1.0);
        // the reported position (the fraction rounds it), or where the bar is dragged
        let shown = seeking.map_or(*pos, |f| f as f64 * dur);
        let (elapsed, remaining) = (
            caption(time(shown)),
            caption(format!("-{}", time(dur - shown))),
        );
        let bar = Rect::from_min_max(
            pos2(row.left() + elapsed.size().x + space::M, row.top()),
            pos2(row.right() - remaining.size().x - space::M, row.bottom()),
        );
        let painter = ui.painter();
        painter.galley(
            pos2(row.left(), row.center().y - elapsed.size().y / 2.0),
            elapsed.clone(),
            weak,
        );
        let r_at = pos2(
            row.right() - remaining.size().x,
            row.center().y - remaining.size().y / 2.0,
        );
        painter.galley(r_at, remaining, weak);
        let track_rect = Rect::from_center_size(bar.center(), vec2(bar.width(), space::XS));
        let pill = CornerRadius::same(radius::PILL);
        painter.rect_filled(
            track_rect,
            pill,
            color::TEXT.gamma_multiply(0.25 * ui.opacity()),
        );
        let filled = Rect::from_min_size(
            track_rect.min,
            vec2(track_rect.width() * frac, track_rect.height()),
        );
        painter.rect_filled(filled, pill, color::TEXT.gamma_multiply(ui.opacity()));
        if let Some(c) = controls.as_mut().filter(|_| *can_seek) {
            let r = ui.interact(bar, ui.scope_id().with("seek"), Sense::click_and_drag());
            let at = |r: &egui::Response| {
                r.interact_pointer_pos()
                    .map(|p| ((p.x - bar.left()) / bar.width().max(1.0)).clamp(0.0, 1.0))
            };
            if r.dragged() {
                *c.seeking = at(&r);
            }
            if (r.drag_stopped() || r.clicked())
                && let Some(f) = at(&r).or(*c.seeking)
            {
                command = Some(Command::Seek(f as f64 * dur));
                *c.seeking = None;
            }
        }
    }
    // the bottom area, one fixed height: the controls, or (lyrics view) the current
    // line and the next one, so the card never changes size
    if let Some(c) = controls.as_mut() {
        let (area, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), BOTTOM_HEIGHT), Sense::hover());
        let lyrics = p.lyrics.as_ref().filter(|_| *c.lyrics_open);
        if let Some(l) = lyrics {
            paint_lyric_pair(ui, area, l, timeline.map(|t| t.0), weak);
        } else {
            if let Some(cmd) = transport(ui, area, track.playing, *can_prev, *can_next) {
                command = Some(cmd);
            }
            if let Some(level) = p.volume
                && let Some(cmd) = volume_button(ui, area, level, c)
            {
                command = Some(cmd);
            }
        }
    }
    if lyric_lines > 0
        && let Some(l) = &p.lyrics
    {
        lyrics_ui(ui, l, timeline.map(|t| t.0), lyric_lines, weak);
    }
    command
}

/// The player's bottom area: the play/pause control and a little room.
const BOTTOM_HEIGHT: f32 = size::CONTROL + space::S;
/// The centres of previous and next, left and right of play/pause's.
const TRANSPORT_STEP: f32 = size::CONTROL + space::XL;
/// The volume slider's height.
const VOLUME_HEIGHT: f32 = 120.0;

/// Previous, play/pause and next (Phosphor Fill), symmetric around the area's centre.
fn transport(ui: &mut Ui, area: Rect, playing: bool, prev: bool, next: bool) -> Option<Command> {
    let mut command = None;
    let buttons = [
        (
            -1.0,
            icons::SKIP_BACK,
            size::ICON_LARGE,
            prev,
            "Previous",
            Command::Prev,
        ),
        (
            0.0,
            if playing { icons::PAUSE } else { icons::PLAY },
            size::CONTROL,
            true,
            "Play or pause",
            Command::PlayPause,
        ),
        (
            1.0,
            icons::SKIP_FORWARD,
            size::ICON_LARGE,
            next,
            "Next",
            Command::Next,
        ),
    ];
    for (dx, glyph, glyph_size, enabled, tip, cmd) in buttons {
        let at = area.center() + vec2(dx * TRANSPORT_STEP, 0.0);
        let rect = Rect::from_center_size(at, egui::Vec2::splat(size::CONTROL));
        let r = ui.interact(rect, ui.scope_id().with(tip), Sense::click());
        let c = if !enabled {
            color::TEXT.gamma_multiply(0.3)
        } else if r.hovered() {
            color::TEXT
        } else {
            color::TEXT.gamma_multiply(0.9)
        };
        let g = ui
            .ctx()
            .fonts_mut(|f| f.layout_no_wrap(glyph.into(), icon_font(glyph_size), c));
        let p = ui.painter();
        // centred on the glyph's ink, not its advance box (side bearings differ)
        let pos = at - g.mesh_bounds.center().to_vec2();
        p.add(TextShape::new(pos, g, c).with_opacity_factor(ui.opacity()));
        if enabled && r.on_hover_text(tip).clicked() {
            command = Some(cmd);
        }
    }
    command
}

/// The speaker at the area's left: its icon follows the level (muted, low, high); the
/// wheel over it changes the volume in 5 % steps; a click slides a vertical slider out
/// of the card's side (`volume_pane`), a click outside closes it.
fn volume_button(ui: &mut Ui, area: Rect, level: f32, c: &mut Controls) -> Option<Command> {
    let mut command = None;
    let shown = c.volume.unwrap_or(level);
    let glyph = if shown <= 0.001 {
        icons::VOLUME_OFF
    } else if shown < 0.5 {
        icons::VOLUME_DOWN
    } else {
        icons::VOLUME_UP
    };
    let rect = Rect::from_min_size(
        pos2(area.left(), area.center().y - size::CONTROL / 2.0),
        egui::Vec2::splat(size::CONTROL),
    );
    let r = ui.interact(rect, ui.scope_id().with("volume"), Sense::click());
    let ink = if r.hovered() {
        color::TEXT
    } else {
        color::TEXT.gamma_multiply(0.8)
    };
    let g = ui
        .ctx()
        .fonts_mut(|f| f.layout_no_wrap(glyph.into(), icon_font(size::ICON), ink));
    let pos = rect.center() - g.mesh_bounds.center().to_vec2();
    ui.painter()
        .add(TextShape::new(pos, g, ink).with_opacity_factor(ui.opacity()));
    if r.hovered() {
        // the wheel's notches this frame (up: louder)
        let wheel: f32 = ui.input(|i| {
            (i.raw.events.iter())
                .filter_map(|e| match e {
                    egui::Event::MouseWheel { delta, .. } => Some(delta.y),
                    _ => None,
                })
                .sum()
        });
        if wheel != 0.0 {
            let v = (shown + 0.05 * wheel.signum()).clamp(0.0, 1.0);
            *c.volume = Some(v);
            command = Some(Command::Volume(v));
        }
    }
    let r = r.on_hover_text(format!("Volume {}%", (shown * 100.0).round()));
    let now = ui.input(|i| i.time);
    if r.clicked() {
        *c.volume_open = c.volume_open.is_none().then_some(now);
    }
    if let Some(opened) = *c.volume_open {
        let (cmd, pane) = volume_pane(ui, c.card, rect, shown, now - opened, c.volume);
        command = command.or(cmd);
        // a click anywhere else closes it
        if pane.clicked_elsewhere() && !r.clicked() {
            *c.volume_open = None;
        }
    }
    // the media thread reports the new level within a poll; until then the set one shows
    if !r.hovered() && c.volume.is_some_and(|d| (d - level).abs() < 0.01) {
        *c.volume = None;
    }
    command
}

/// Whether the volume slider fits left of the card (else it opens on the right).
fn pane_on_left(card: Rect, screen: Rect) -> bool {
    card.left() - space::S - size::CONTROL >= screen.left() + space::S
}

/// The volume slider: a narrow capsule that slides out of the card's left edge, level
/// with the speaker (the right edge when the card sits at the screen's left edge), so it
/// never covers the card. `age`: seconds since it opened. Returns a volume command and
/// the pane's response (for closing on a click elsewhere).
fn volume_pane(
    ui: &Ui,
    card: Rect,
    speaker: Rect,
    shown: f32,
    age: f64,
    dragged: &mut Option<f32>,
) -> (Option<Command>, egui::Response) {
    let mut command = None;
    let size = vec2(size::CONTROL, VOLUME_HEIGHT);
    let screen = ui.ctx().content_rect();
    let left = pane_on_left(card, screen);
    let y = (speaker.center().y - size.y / 2.0).clamp(
        screen.top() + space::S,
        (screen.bottom() - size.y - space::S).max(0.0),
    );
    // slides out from under the card's edge over motion::FAST
    let t = crate::anim::ease_out((age as f32 / motion::FAST).min(1.0));
    let x = if left {
        card.left() - (space::S + size.x) * t
    } else {
        card.right() + (space::S + size.x) * t - size.x
    };
    let area = egui::Area::new(ui.scope_id().with("volume pane"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos2(x, y))
        .show(ui.ctx(), |ui| {
            ui.set_opacity(t);
            let (pane, br) = ui.allocate_exact_size(size, Sense::click_and_drag());
            let pill = CornerRadius::same(radius::PILL);
            let p = ui.painter();
            p.add(HUD_SHADOW.as_shape(pane, pill));
            p.rect_filled(pane, pill, color::SURFACE.gamma_multiply(0.92 * t));
            let track = Rect::from_center_size(
                pane.center(),
                vec2(space::XS, pane.height() - 2.0 * space::L),
            );
            let at = |y: f32| ((track.bottom() - y) / track.height()).clamp(0.0, 1.0);
            let mut v = shown;
            if let Some(pos) = br.interact_pointer_pos() {
                v = at(pos.y);
                *dragged = Some(v);
            }
            let rail = CornerRadius::same(radius::PILL);
            p.rect_filled(track, rail, color::TEXT.gamma_multiply(0.25 * t));
            let top = track.bottom() - track.height() * v;
            let filled = Rect::from_min_max(pos2(track.left(), top), track.max);
            p.rect_filled(filled, rail, color::TEXT.gamma_multiply(t));
            // a thin handle across the bar, no round knob
            let handle =
                Rect::from_center_size(pos2(track.center().x, top), vec2(space::M, space::XS));
            p.rect_filled(handle, rail, color::TEXT.gamma_multiply(t));
            if br.drag_stopped() || br.clicked() {
                command = Some(Command::Volume(v));
            }
        });
    (command, area.response)
}

/// The lyrics view in the player's bottom area: the current line (Inter Display, full
/// white) and the next one dimmed under it, each cut to one line.
fn paint_lyric_pair(ui: &Ui, area: Rect, lyrics: &Lyrics, position: Option<f64>, weak: Color32) {
    let note = match lyrics {
        Lyrics::Loading => Some("Looking for lyrics"),
        Lyrics::NotFound => Some("No synced lyrics for this track"),
        Lyrics::Unavailable => Some("Lyrics unavailable (offline?)"),
        Lyrics::Lines(_) if position.is_none() => Some("This player does not report the position"),
        Lyrics::Lines(_) => None,
    };
    let one_line = |text: &str, f: egui::FontId, c: Color32| {
        let mut job = LayoutJob::single_section(text.into(), TextFormat::simple(f, c));
        job.wrap = egui::text::TextWrapping::truncate_at_width(area.width());
        ui.ctx().fonts_mut(|fonts| fonts.layout_job(job))
    };
    let rows = match (note, lyrics, position) {
        (Some(n), ..) => vec![one_line(n, font(size::BODY, Weight::Regular), weak)],
        (None, Lyrics::Lines(all), Some(pos)) => {
            let now = lyrics::current(all, pos);
            let line = |i: usize| all.get(i).map_or("", |l| l.1.as_str());
            let current = now.map_or("…", line);
            let next = line(now.map_or(0, |i| i + 1));
            vec![
                one_line(current, display(size::LABEL, Weight::SemiBold), color::TEXT),
                one_line(next, font(size::CAPTION, Weight::Regular), weak),
            ]
        }
        _ => Vec::new(),
    };
    let h: f32 = rows.iter().map(|r| r.size().y).sum();
    let mut y = area.center().y - h / 2.0;
    for g in rows {
        let x = area.center().x - g.size().x / 2.0;
        let size = g.size().y;
        ui.painter().add(
            TextShape::new(pos2(x, y), g, Color32::TRANSPARENT).with_opacity_factor(ui.opacity()),
        );
        y += size;
    }
}

/// The subtitles under the island: the current lyrics line, small, translucent, no box
/// (a soft shadow of the glyphs keeps it legible on bright maps); a new line crossfades
/// in.
#[derive(Default)]
pub struct Subtitles {
    line: String,
    previous: String,
    fade: Spring,
}

impl Subtitles {
    /// Paints `line` centred under `below` (nothing: fades out); `dt` in seconds.
    pub fn show(&mut self, ui: &Ui, below: Rect, line: Option<&str>, dt: f32) {
        let line = line.unwrap_or("");
        if line != self.line {
            self.previous = std::mem::replace(&mut self.line, line.to_string());
            self.fade = Spring::new(0.0);
        }
        let f = self.fade.update(1.0, motion::NORMAL, dt).clamp(0.0, 1.0);
        let at = below.center_bottom() + vec2(0.0, space::S);
        for (text, alpha) in [(&self.previous, 1.0 - f), (&self.line, f)] {
            if text.is_empty() || alpha <= 0.01 {
                continue;
            }
            let g = ui.ctx().fonts_mut(|fonts| {
                let font = font(size::BODY, Weight::Medium);
                fonts.layout(text.clone(), font, color::TEXT, WRAP)
            });
            let rect = Rect::from_min_size(at - vec2(g.size().x / 2.0, 0.0), g.size());
            let p = ui.painter();
            // a soft shadow of the glyphs themselves (two rings of faint dark copies),
            // so the line reads on bright maps without a box
            for (r, a) in [(1.0, 0.16), (2.0, 0.08)] {
                for k in 0..8 {
                    let t = k as f32 * std::f32::consts::FRAC_PI_4;
                    let off = vec2(t.cos(), t.sin()) * r + vec2(0.0, 1.0);
                    let c = Color32::BLACK.gamma_multiply(a * alpha);
                    p.galley_with_override_text_color(rect.min + off, g.clone(), c);
                }
            }
            let shape = TextShape::new(rect.min, g, Color32::TRANSPARENT);
            p.add(shape.with_opacity_factor(alpha * SUBTITLE_ALPHA));
        }
    }
}

/// The subtitles' opacity: present, never in the way.
const SUBTITLE_ALPHA: f32 = 0.8;

/// Synced lyrics at `position`: the current line in Inter Display, with the previous and
/// next ones dimmed when `lines` is 3; a short note while loading or without lyrics.
pub fn lyrics_ui(ui: &mut Ui, lyrics: &Lyrics, position: Option<f64>, lines: usize, weak: Color32) {
    let note = |ui: &mut Ui, s: &str| w::text(ui, s, size::BODY, Weight::Regular, weak);
    let all = match lyrics {
        Lyrics::Loading => return drop(note(ui, "Looking for lyrics")),
        Lyrics::NotFound => return drop(note(ui, "No synced lyrics for this track")),
        Lyrics::Unavailable => return drop(note(ui, "Lyrics unavailable (offline?)")),
        Lyrics::Lines(all) => all,
    };
    let Some(position) = position else {
        return drop(note(ui, "This player does not report the position"));
    };
    let now = lyrics::current(all, position);
    let line = |i: Option<usize>| i.and_then(|i| all.get(i)).map_or("", |l| l.1.as_str());
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = space::XS;
        let label = |ui: &mut Ui, s: &str, f: egui::FontId, c: Color32| {
            let t = egui::RichText::new(if s.is_empty() { " " } else { s })
                .font(f)
                .color(c);
            ui.add(egui::Label::new(t).truncate());
        };
        if lines >= 3 {
            let prev = now.and_then(|i| i.checked_sub(1));
            label(ui, line(prev), font(size::BODY, Weight::Regular), weak);
        }
        // "..." between lines before the first one
        let current = now.map_or("…", |i| line(Some(i)));
        label(
            ui,
            current,
            display(size::LABEL, Weight::SemiBold),
            color::TEXT,
        );
        if lines >= 3 {
            let next = now.map_or(Some(0), |i| Some(i + 1));
            label(ui, line(next), font(size::BODY, Weight::Regular), weak);
        }
    });
}

/// A cover's texture, uploaded once per cover (the media thread's cover id).
#[derive(Default)]
pub struct CoverCache {
    cover: Option<(u64, egui::TextureHandle)>,
}

impl CoverCache {
    pub fn get(
        &mut self,
        ctx: &egui::Context,
        cover: Option<&media::Cover>,
    ) -> Option<egui::TextureId> {
        let c = cover?;
        if self.cover.as_ref().is_none_or(|(id, _)| *id != c.id) {
            let image = egui::ColorImage::from_rgba_unmultiplied(c.size, &c.rgba);
            let tex = ctx.load_texture("island cover", image, egui::TextureOptions::LINEAR);
            self.cover = Some((c.id, tex));
        }
        self.cover.as_ref().map(|(_, t)| t.id())
    }
}

impl Widget for Island {
    fn id(&self) -> &'static str {
        "island"
    }

    fn name(&self) -> &'static str {
        "Dynamic Island"
    }

    fn icon(&self) -> &'static str {
        icons::TIMER
    }

    fn default_placement(&self) -> Placement {
        Placement {
            anchor: Anchor::Top,
            x: 0,
            y: TOP,
            enabled: true,
        }
    }

    /// The expanded player takes clicks (it is open like a menu).
    fn interactive(&self, data: &HudData) -> bool {
        data.player_open
    }

    fn show(&mut self, ui: &mut Ui, data: &HudData, config: &HudConfig) {
        let client = data.client;
        let Some(tick) = client.game_tick() else {
            return;
        };
        let local = client.local_id();
        for msg in data.events {
            self.race.on_message(msg, local, tick);
            self.broadcast.on_message(msg, tick);
        }
        let on = |key: &str| hud::item_on(config, "island", &ITEMS, key);
        let items = client.snapshot();
        let info = (items.iter())
            .find(|it| it.type_id == g::NETOBJTYPE_GAMEINFO)
            .map(|it| g::ObjGameInfo::from_ints(&it.data));
        let now = ui.input(|i| i.time);
        let spectator = spectating(items);
        // C++ renders the race effects only for a local character that is not spectating
        let race = if data.tee.is_some() && spectator.is_none() {
            self.race.effect(tick, on("checkpoint"))
        } else {
            None
        };
        let (finish, checkpoint) = match race {
            Some(f @ Effect::Finish { .. }) => (Some(f), None),
            cp => (None, cp),
        };
        let status = if info.as_ref().is_some_and(paused) {
            Some(Part::Status(icons::PAUSE, "Game paused".into()))
        } else {
            spectator.map(|s| Part::Status(icons::EYE, s))
        };
        let timer = info.as_ref().map(|i| timer(i, tick, now));
        // now playing; a new title or artist is a track change
        let state = data.media.map(Media::state).unwrap_or_default();
        let ctx = ui.ctx().clone();
        let cover = (self.cover).get(&ctx, state.track.as_ref().and_then(|t| t.cover.as_ref()));
        let playing = state.track.as_ref().map(|t| NowPlaying {
            title: t.title.clone(),
            artist: t.artist.clone(),
            playing: t.playing,
            cover,
        });
        let key = playing
            .as_ref()
            .map(|t| (t.title.clone(), t.artist.clone()));
        if key.is_some() && key != self.track {
            self.track_until = tick.wrapping_add(TRACK_TICKS);
        }
        self.track = key;
        let (mut idle, mut events) = (Vec::new(), Vec::new());
        for key in hud::item_order(config, "island", &ITEMS) {
            if !on(key) {
                continue;
            }
            let part = match key {
                "status" => status.clone(),
                "timer" => timer.clone().map(Part::Timer),
                "checkpoint" => checkpoint.clone().map(Part::Checkpoint),
                "media" => playing.clone().map(Part::Media),
                _ => None,
            };
            idle.extend(part);
            let event = match key {
                "finish" => finish.clone().map(View::Finish),
                "broadcasts" => self.broadcast.shown(tick).map(|text| View::Broadcast {
                    text: text.into(),
                    // the timer stays at its end when it is on
                    timer: timer.clone().filter(|_| on("timer")),
                }),
                "track" => (playing.clone())
                    .filter(|_| self.track_until.wrapping_sub(tick) > 0)
                    .map(View::Track),
                _ => None,
            };
            events.extend(event);
        }
        if idle.is_empty() && data.editing {
            idle.push(Part::Timer(Timer::Clock {
                text: "00:00".into(),
                warn: false,
                blink: false,
            }));
        }
        let v =
            if data.player_open {
                let now_i = std::time::Instant::now();
                Some(View::Player(Player {
                    track: state.track.as_ref().zip(playing).map(|(t, np)| {
                        (np, t.position(now_i), [t.can_prev, t.can_next, t.can_seek])
                    }),
                    sessions: state.sessions.clone(),
                    source: state.source.clone(),
                    lyrics: state.lyrics.clone(),
                    volume: state.volume,
                }))
            } else {
                view(events, idle)
            };
        let dt = self.last.map_or(0.0, |l| (now - l).clamp(0.0, 0.1) as f32);
        self.last = Some(now);
        if let (Some(c), Some(m)) = (self.anim.show(ui, data.glass, v, now), data.media) {
            m.send(c);
        }
        // the lyrics as subtitles (not while the player shows them itself)
        let line = match (&state.lyrics, state.track.as_ref()) {
            (Some(Lyrics::Lines(all)), Some(t)) if lyrics_place(config) == "island" => t
                .position(std::time::Instant::now())
                .and_then(|(pos, _)| lyrics::current(all, pos))
                .map(|i| all[i].1.as_str()),
            _ => None,
        };
        let line = line.filter(|_| !data.player_open);
        if let Some(rect) = self.anim.rect {
            self.subtitles.show(ui, rect, line, dt);
        }
    }

    fn settings(&mut self, ui: &mut Ui, config: &mut HudConfig) {
        // where the lyrics show (one place)
        ui.label("Lyrics");
        let mut i = (LYRICS_PLACES.iter())
            .position(|p| p.0 == lyrics_place(config))
            .unwrap_or(0);
        let labels = LYRICS_PLACES.map(|p| p.1);
        if w::segmented(ui, &mut i, &labels).changed() {
            config.set("island", "lyrics_place", LYRICS_PLACES[i].0);
        }
        let weak = ui.visuals().weak_text_color();
        w::text(
            ui,
            "From lrclib.net (title and artist are sent)",
            size::CAPTION,
            Weight::Regular,
            weak,
        );
        ui.add_space(space::S);
        hud::item_list(ui, config, "island", &ITEMS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(flags: i32, start: i32, warmup: i32, limit: i32) -> g::ObjGameInfo {
        g::ObjGameInfo {
            game_state_flags: flags,
            round_start_tick: start,
            warmup_timer: warmup,
            time_limit: limit,
            ..Default::default()
        }
    }

    fn clock(t: Timer) -> (String, bool, bool) {
        match t {
            Timer::Clock { text, warn, blink } => (text, warn, blink),
            Timer::SuddenDeath => panic!("sudden death"),
        }
    }

    /// Cases worked out from `CHud::RenderGameTimer` (hud.cpp:118).
    #[test]
    fn timer_follows_render_game_timer() {
        let race = g::GAMESTATEFLAG_RACETIME;
        // race: (tick + warmup) / 50, the warmup being -start
        let c = clock(timer(&info(race, 0, -1000, 0), 1000 + 50 * 65 + 49, 0.0));
        assert_eq!(c, ("01:05".into(), false, false));
        // round time; before the start the division truncates to 0, then negative = 0
        assert_eq!(clock(timer(&info(0, 100, 0, 0), 51, 0.0)).0, "00:00");
        assert_eq!(clock(timer(&info(0, 100, 0, 0), 49, 0.0)).0, "00:00");
        // days and hours
        assert_eq!(
            clock(timer(&info(0, 0, 0, 0), 50 * 90_000, 0.0)).0,
            "1d 01:00:00"
        );
        assert_eq!(
            clock(timer(&info(0, 0, 0, 0), 50 * 3600, 0.0)).0,
            "01:00:00"
        );
        // time limit: counts down, red in the last 60 s, blinks in the last 10 on odd
        // half seconds, 0 when the game is over
        assert_eq!(
            clock(timer(&info(0, 0, 0, 2), 50 * 70, 0.0)),
            ("00:50".into(), true, false)
        );
        assert!(!clock(timer(&info(0, 0, 0, 2), 50 * 59, 0.0)).1);
        assert!(clock(timer(&info(0, 0, 0, 2), 50 * 115, 0.6)).2);
        assert!(!clock(timer(&info(0, 0, 0, 2), 50 * 115, 1.1)).2);
        let over = g::GAMESTATEFLAG_GAMEOVER;
        assert_eq!(clock(timer(&info(over, 0, 0, 2), 50 * 30, 0.0)).0, "00:00");
        // a warmup hides the limit: plain round time
        let w = clock(timer(&info(0, 0, 5, 2), 50 * 30, 0.0));
        assert_eq!(w, ("00:30".into(), false, false));
        let sd = g::GAMESTATEFLAG_SUDDENDEATH;
        assert_eq!(timer(&info(sd, 0, 0, 0), 0, 0.0), Timer::SuddenDeath);
        // hostile values do not panic
        timer(&info(race, i32::MIN, i32::MIN, i32::MAX), i32::MAX, 0.0);
        timer(&info(0, i32::MIN, 0, i32::MAX), i32::MAX, 0.0);
    }

    fn time(time: i32, check: i32, finish: i32) -> GameMsg {
        GameMsg::SvDDRaceTime(g::MsgSvDDRaceTime {
            time,
            check,
            finish,
        })
    }

    /// Cases worked out from `CHud::OnMessage` and `RenderDDRaceEffects`.
    #[test]
    fn race_effects_follow_the_hud() {
        let mut r = Race::default();
        assert_eq!(r.effect(0, true), None);
        r.on_message(&time(4210, -36, 1), Some(0), 100);
        let finish = Effect::Finish {
            time: "42.10".into(), // C++ str_time drops the minutes below one
            diff: Some(("-00.36".into(), true)),
            record: Record::None,
        };
        assert_eq!(r.effect(100 + EFFECT_TICKS - 1, true), Some(finish));
        assert_eq!(r.effect(100 + EFFECT_TICKS, true), None);
        // a record from Sv_RaceFinish of the local player at the same tick
        let rf = |id, server| {
            GameMsg::SvRaceFinish(g::MsgSvRaceFinish {
                client_id: id,
                record_server: server,
                record_personal: 1,
                ..Default::default()
            })
        };
        r.on_message(&rf(3, 1), Some(0), 200); // another player
        r.on_message(&time(4210, 0, 1), Some(0), 200);
        r.on_message(&rf(0, 1), Some(0), 200);
        let Some(Effect::Finish { diff, record, .. }) = r.effect(200, true) else {
            panic!("no finish")
        };
        assert_eq!((diff, record), (None, Record::Server));
        // checkpoint: sign and the C++ even centiseconds (0.37 -> 00.38)
        r.on_message(&time(100, 37, 0), Some(0), 300);
        let cp = Effect::Checkpoint {
            text: "+00.38".into(),
            sign: 1,
        };
        assert_eq!(r.effect(300, true), Some(cp));
        assert_eq!(r.effect(300, false), None);
        r.on_message(&time(100, 0, 0), Some(0), 300);
        let zero = r.effect(300, true);
        assert!(matches!(zero, Some(Effect::Checkpoint { sign: 0, .. })));
        // legacy message: the same handling
        let legacy = g::MsgSvDDRaceTimeLegacy {
            time: 100,
            check: -120,
            finish: 0,
        };
        r.on_message(&GameMsg::SvDDRaceTimeLegacy(legacy), None, 400);
        let cp = Effect::Checkpoint {
            text: "-01.20".into(),
            sign: -1,
        };
        assert_eq!(r.effect(400, true), Some(cp));
    }

    #[test]
    fn broadcasts_last_ten_seconds() {
        let mut b = Broadcast::default();
        assert_eq!(b.shown(0), None);
        let msg = |s: &str| GameMsg::SvBroadcast(g::MsgSvBroadcast { message: s.into() });
        b.on_message(&msg("\n\n   Welcome   \n"), 1000);
        assert_eq!(b.shown(1000 + BROADCAST_TICKS - 1), Some("Welcome"));
        assert_eq!(b.shown(1000 + BROADCAST_TICKS), None);
        b.on_message(&msg(" "), 2000);
        assert_eq!(b.shown(2000), None);
    }

    fn item(type_id: i32, id: i32, data: Vec<i32>) -> Item {
        Item { type_id, id, data }
    }

    fn name_ints(s: &str) -> Vec<i32> {
        let mut b = s.as_bytes().to_vec();
        b.resize(16, 0);
        (b.chunks(4))
            .map(|c| i32::from_be_bytes([c[0] ^ 0x80, c[1] ^ 0x80, c[2] ^ 0x80, c[3] ^ 0x80]))
            .collect()
    }

    #[test]
    fn spectating_follows_spec_info() {
        let player = |team| item(g::NETOBJTYPE_PLAYERINFO, 2, vec![1, 2, team, 0, 0]);
        let mut client = name_ints("brainless");
        client.resize(g::ObjClientInfo::NUM_INTS, 0);
        let mut items = vec![player(0), item(g::NETOBJTYPE_CLIENTINFO, 5, client)];
        assert_eq!(spectating(&items), None);
        items[0] = player(TEAM_SPECTATORS);
        assert_eq!(spectating(&items).as_deref(), Some("Free-View"));
        items.push(item(g::NETOBJTYPE_SPECTATORINFO, 0, vec![5, 0, 0]));
        assert_eq!(spectating(&items).as_deref(), Some("Following brainless"));
        // /pause in a team: DDNetPlayer flags
        items[0] = player(0);
        let flags = g::EXPLAYERFLAG_PAUSED;
        items.push(item(g::NETOBJTYPE_DDNETPLAYER, 2, vec![flags, 0, 0, 0]));
        assert!(spectating(&items).is_some());
    }

    #[test]
    fn view_priority() {
        let t = || Part::Timer(timer(&info(0, 0, 0, 0), 0, 0.0));
        let finish = View::Finish(Effect::Finish {
            time: "00:01.00".into(),
            diff: None,
            record: Record::None,
        });
        let b = View::Broadcast {
            text: "b".into(),
            timer: None,
        };
        // events come in priority order; the first wins over the idle row
        let v = view(vec![b.clone(), finish.clone()], vec![t()]);
        assert_eq!(v, Some(b));
        assert_eq!(view(vec![], vec![t()]), Some(View::Idle(vec![t()])));
        assert_eq!(view(vec![], vec![]), None);
        // pause: C++ RenderPauseNotification, not when the game is over
        assert!(paused(&info(g::GAMESTATEFLAG_PAUSED, 0, 0, 0)));
        let over = g::GAMESTATEFLAG_PAUSED | g::GAMESTATEFLAG_GAMEOVER;
        assert!(!paused(&info(over, 0, 0, 0)));
    }

    #[test]
    fn volume_slider_opens_beside_the_card() {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 720.0));
        let card = |x: f32| Rect::from_min_size(pos2(x, 16.0), vec2(360.0, 160.0));
        assert!(pane_on_left(card(460.0), screen));
        assert!(!pane_on_left(card(16.0), screen));
        // just enough room: S + its width + S
        assert!(pane_on_left(card(space::S * 2.0 + size::CONTROL), screen));
    }

    #[test]
    fn items_keep_their_order() {
        let mut c = HudConfig::default();
        let order = |c: &HudConfig| hud::item_order(c, "island", &ITEMS);
        assert_eq!(order(&c)[..3], ["status", "timer", "checkpoint"]);
        c.execute("hud_set", r#"island order "broadcasts,timer""#);
        assert_eq!(order(&c)[..3], ["broadcasts", "timer", "status"]);
        c.execute("hud_set", "island timer 0");
        assert!(!hud::item_on(&c, "island", &ITEMS, "timer"));
        assert!(hud::item_on(&c, "island", &ITEMS, "finish"));
    }
}
