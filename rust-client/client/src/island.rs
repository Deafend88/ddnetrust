//! The Dynamic Island (PLAN §10.3, 7.9; merged with the watermark at the user's request):
//! one element, top centre by default. Idle it is the watermark (logo, stats) with the
//! race timer, a pause or spectator status and the mini player; events (finish, chat
//! mention, broadcast, track change) expand it with a spring, then it collapses back.
//! What it shows is decided by ported C++ logic: the race timer
//! (`CHud::RenderGameTimer`), the finish time and checkpoint difference
//! (`CHud::OnMessage`, `RenderDDRaceEffects`), broadcasts (`CBroadcast`), the pause text
//! (`RenderPauseNotification`), the spectator text (`RenderSpectatorHud`) and mentions
//! (`CChat::AddLine`, `LineShouldHighlight`, `chat.rs`).

use std::sync::Arc;

use ddnet::map::ints_to_str;
use ddnet::protocol::generated::{self as g, GameMsg};
use ddnet::protocol::snapshot::Item;
use egui::epaint::TextShape;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, CornerRadius, Galley, Rect, Sense, Ui, pos2, vec2};

use crate::anim::Spring;
use crate::chat::Mentions;
use crate::hud::{Anchor, HudConfig, HudData, Placement, Widget, item_toggles};
use crate::icons;
use crate::style::{
    Glass, HUD_SHADOW, Weight, color, font, icon_font, motion, radius, size, space,
};
use crate::timefmt::{TimeFormat, str_time, str_time_float};
use crate::watermark::{self, ID, Look, Stat};
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
    Clock {
        text: String,
        /// C++ sizes the timer by the widest text of its class ("00:00" .. "000d
        /// 00:00:00"), so it does not jitter as digits change.
        template: &'static str,
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
    const DAY: i32 = 3600 * 24;
    let template = match time {
        t if t >= DAY * 100 => "000d 00:00:00",
        t if t >= DAY * 10 => "00d 00:00:00",
        t if t >= DAY => "0d 00:00:00",
        t if t >= 3600 => "00:00:00",
        _ => "00:00",
    };
    let warn = limit && time <= 60;
    Timer::Clock {
        text: str_time(time as i64 * 100, TimeFormat::Days),
        template,
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

/// The track the OS media session plays (7.10 fills it; until then there is none).
#[derive(Clone, Debug, PartialEq)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub cover: Option<egui::TextureId>,
}

/// The idle island: the watermark (logo and stats) with the race timer, a status (pause
/// or spectator text) and the mini player.
#[derive(Clone, Debug, PartialEq)]
pub struct Idle {
    pub compact: bool,
    pub look: Look,
    pub timer: Option<Timer>,
    /// The checkpoint difference next to the timer.
    pub checkpoint: Option<Effect>,
    pub status: Option<(&'static str, String)>,
    pub stats: Vec<Stat>,
    pub media: Option<NowPlaying>,
}

impl Idle {
    /// Nothing but the logo: hidden, like a watermark with every item off.
    pub fn is_empty(&self) -> bool {
        self.timer.is_none()
            && self.status.is_none()
            && self.stats.is_empty()
            && self.media.is_none()
    }
}

/// What the island shows: the idle state or an event that expands it.
#[derive(Clone, Debug, PartialEq)]
pub enum View {
    Idle(Idle),
    Finish(Effect),
    /// A server broadcast; the timer stays at its end (C++ shows both).
    Broadcast {
        text: String,
        timer: Option<Timer>,
    },
    /// A chat line that mentions the local player.
    Mention {
        author: String,
        text: String,
    },
    /// The track changed.
    Track(NowPlaying),
}

/// The events by priority over the idle state: finish > mention > broadcast > track.
pub fn view(
    idle: Option<Idle>,
    finish: Option<Effect>,
    mention: Option<(String, String)>,
    broadcast: Option<(String, Option<Timer>)>,
    track: Option<NowPlaying>,
) -> Option<View> {
    if let Some(f) = finish {
        return Some(View::Finish(f));
    }
    if let Some((author, text)) = mention {
        return Some(View::Mention { author, text });
    }
    if let Some((text, timer)) = broadcast {
        return Some(View::Broadcast { text, timer });
    }
    if let Some(t) = track {
        return Some(View::Track(t));
    }
    idle.map(View::Idle)
}

/// How long a mention and a track change show (C++ has no such notice; the concept's 3 s
/// for the track).
pub const MENTION_TICKS: i32 = TICK_SPEED * 4;
pub const TRACK_TICKS: i32 = TICK_SPEED * 3;

/// The island's items (`hud_set island <key> 0|1`), first values from C++ where it has
/// them. The watermark's items are listed in `watermark::ITEMS`.
const ITEMS: [(&str, &str, bool); 5] = [
    ("timer", "Race timer", true),
    ("checkpoint", "Checkpoint difference", true),
    ("broadcasts", "Broadcasts", true),
    ("spectator", "Spectator text", true),
    ("mentions", "Chat mentions", true),
];

/// The island widget: the watermark, the race state, the events and the springs.
#[derive(Default)]
pub struct Island {
    race: Race,
    broadcast: Broadcast,
    mentions: Mentions,
    /// The last mention (author, text) and the tick it shows until.
    mention: Option<(String, String, i32)>,
    /// The last track seen (title, artist) and the tick its change shows until.
    track: Option<(String, String)>,
    track_until: i32,
    anim: Anim,
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
    /// The idle state's size: events are at least this big (they expand the island).
    idle_size: egui::Vec2,
    last: Option<f64>,
}

/// The island's top offset, the event text's wrap width (points).
const TOP: i32 = 16;
const WRAP: f32 = 480.0;
/// Damping of the size springs: a small overshoot (about 3 %), the island's motion.
const ZETA: f32 = 0.75;
/// The equalizer: bars, their width and gap, height (points).
const EQ_BARS: usize = 4;
const EQ_BAR: f32 = 3.0;
const EQ_GAP: f32 = space::XS;

/// A laid-out event: a lead (icon or cover), one or more rows, a trailing timer.
struct Laid {
    lead: Option<Lead>,
    rows: Vec<Arc<Galley>>,
    /// The width of the rows.
    width: f32,
    trail: Option<Arc<Galley>>,
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

fn job(spans: &[(&str, f32, Weight, Color32)], wrap: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    for &(text, size, weight, color) in spans {
        job.append(text, 0.0, TextFormat::simple(font(size, weight), color));
    }
    job.wrap.max_width = wrap;
    job
}

/// The timer's text, colour (red in the last minute of a time limit, half transparent
/// when blinking) and C++ width template.
fn timer_text(timer: &Timer) -> (String, Color32, &'static str) {
    match timer {
        Timer::SuddenDeath => ("Sudden Death".into(), color::TEXT, ""),
        Timer::Clock {
            text,
            template,
            warn,
            blink,
        } => {
            let c = if *warn { color::DANGER } else { color::TEXT };
            let c = if *blink { c.gamma_multiply(0.5) } else { c };
            (text.clone(), c, template)
        }
    }
}

fn layout(ui: &Ui, j: LayoutJob) -> Arc<Galley> {
    ui.ctx().fonts_mut(|f| f.layout_job(j))
}

fn icon_galley(ui: &Ui, glyph: &str, size: f32, c: Color32) -> Arc<Galley> {
    ui.ctx()
        .fonts_mut(|f| f.layout_no_wrap(glyph.into(), icon_font(size), c))
}

/// Lays out an event (the idle state is drawn with egui widgets instead).
fn lay_out(ui: &Ui, view: &View, weak: Color32) -> Laid {
    let inf = f32::INFINITY;
    let laid = |lead, rows: Vec<Arc<Galley>>, trail| Laid {
        lead,
        width: rows.iter().map(|r| r.size().x).fold(0.0, f32::max),
        rows,
        trail,
    };
    match view {
        View::Idle(_) => laid(None, Vec::new(), None),
        View::Finish(Effect::Finish { time, diff, record }) => {
            let (glyph, title) = match record {
                Record::Server => (icons::TROPHY, "Server record  "),
                Record::Personal => (icons::TROPHY, "Personal best  "),
                Record::None => (icons::CHECK_CIRCLE, "Finish time  "),
            };
            let mut rows = vec![layout(
                ui,
                job(
                    &[
                        (title, size::BODY, Weight::Regular, weak),
                        (time, size::TITLE, Weight::SemiBold, color::TEXT),
                    ],
                    inf,
                ),
            )];
            if let Some((d, faster)) = diff {
                let c = if *faster {
                    color::SUCCESS
                } else {
                    color::DANGER
                };
                let spans = [
                    (d.as_str(), size::BODY, Weight::Medium, c),
                    ("  vs. your best", size::BODY, Weight::Regular, weak),
                ];
                rows.push(layout(ui, job(&spans, inf)));
            }
            let icon = icon_galley(ui, glyph, size::ICON_LARGE, color::SUCCESS);
            laid(Some(Lead::Icon(icon)), rows, None)
        }
        View::Finish(_) => laid(None, Vec::new(), None),
        View::Broadcast { text, timer } => {
            let row = job(&[(text, size::BODY, Weight::Medium, color::TEXT)], WRAP);
            let trail = timer.as_ref().map(|t| {
                let (s, c, _) = timer_text(t);
                layout(ui, job(&[(&s, size::TITLE, Weight::SemiBold, c)], inf))
            });
            let icon = icon_galley(ui, icons::MEGAPHONE, size::ICON, color::TEXT);
            laid(Some(Lead::Icon(icon)), vec![layout(ui, row)], trail)
        }
        View::Mention { author, text } => {
            let rows = vec![
                layout(
                    ui,
                    job(&[(author, size::BODY, Weight::SemiBold, weak)], inf),
                ),
                layout(
                    ui,
                    job(&[(text, size::BODY, Weight::Regular, color::TEXT)], WRAP),
                ),
            ];
            let icon = icon_galley(ui, icons::CHAT, size::ICON_LARGE, color::TEXT);
            laid(Some(Lead::Icon(icon)), rows, None)
        }
        View::Track(t) => {
            let rows = vec![
                layout(
                    ui,
                    job(
                        &[(&t.title, size::LABEL, Weight::SemiBold, color::TEXT)],
                        WRAP,
                    ),
                ),
                layout(
                    ui,
                    job(&[(&t.artist, size::BODY, Weight::Regular, weak)], WRAP),
                ),
            ];
            laid(Some(Lead::Cover(t.cover)), rows, None)
        }
    }
}

/// An event's padded size and whether it is one line (a pill).
fn measure(l: &Laid) -> (egui::Vec2, bool) {
    let rows_h: f32 = l.rows.iter().map(|r| r.size().y).sum();
    let one_line = l.rows.len() == 1 && l.rows[0].rows.len() == 1;
    let lead = l.lead.as_ref().map_or(egui::Vec2::ZERO, Lead::size);
    let lead_w = if l.lead.is_some() {
        lead.x + space::M
    } else {
        0.0
    };
    let trail_w = l.trail.as_ref().map_or(0.0, |t| t.size().x + space::XL);
    let pad = if one_line { space::S } else { space::M };
    let h = rows_h.max(lead.y) + 2.0 * pad;
    (
        vec2(lead_w + l.width + trail_w + 2.0 * space::XL, h),
        one_line,
    )
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

/// The equalizer: bars moving while the track plays, low while paused.
fn equalizer(ui: &mut Ui, playing: bool, now: f64) {
    let h = size::ICON;
    let w = EQ_BARS as f32 * EQ_BAR + (EQ_BARS - 1) as f32 * EQ_GAP;
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
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
        ui.painter()
            .rect_filled(bar, CornerRadius::same(radius::PILL), color::ACCENT);
    }
}

/// The idle row: logo, timer (+ checkpoint difference), status, stats with dots, then the
/// mini player.
fn draw_idle(ui: &mut Ui, idle: &Idle, weak: Color32, now: f64) {
    ui.spacing_mut().interact_size.y = idle.look.logo_height;
    ui.spacing_mut().item_spacing.x = 0.0;
    watermark::logo(ui, idle.compact, idle.look);
    let mut first = true;
    let mut gap = |ui: &mut Ui| {
        if std::mem::take(&mut first) {
            ui.add_space(space::XXL);
        } else {
            w::dot(ui);
        }
    };
    if let Some(timer) = &idle.timer {
        gap(ui);
        let (text, c, template) = timer_text(timer);
        let galley = layout(
            ui,
            job(&[(&text, size::TITLE, Weight::SemiBold, c)], f32::INFINITY),
        );
        let template_w = if template.is_empty() {
            0.0
        } else {
            let spans = [(template, size::TITLE, Weight::SemiBold, c)];
            layout(ui, job(&spans, f32::INFINITY)).size().x
        };
        // C++ keeps the width of the timer's class, so the row does not jitter
        let size = vec2(galley.size().x.max(template_w), galley.size().y);
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        ui.painter().galley(
            rect.center() - galley.size() / 2.0,
            galley,
            Color32::TRANSPARENT,
        );
        if let Some(Effect::Checkpoint { text, sign }) = &idle.checkpoint {
            let c = match sign {
                1 => color::DANGER,
                -1 => color::SUCCESS,
                _ => color::TEXT,
            };
            ui.add_space(space::S);
            w::text(ui, text, size::BODY, Weight::Medium, c);
        }
    }
    if let Some((glyph, text)) = &idle.status {
        gap(ui);
        w::icon(ui, glyph, size::ICON, weak);
        ui.add_space(space::S);
        w::text(ui, text, size::BODY, Weight::Medium, weak);
    }
    for (glyph, value, unit) in &idle.stats {
        gap(ui);
        w::stat(ui, glyph, value, *unit);
    }
    if let Some(m) = &idle.media {
        gap(ui);
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(size::ICON_HUD), Sense::hover());
        paint_cover(ui.painter(), rect, m.cover, ui.opacity());
        ui.add_space(space::S);
        equalizer(ui, m.playing, now);
    }
}

/// The idle row's size: laid out once without painting.
fn measure_idle(ui: &mut Ui, idle: &Idle, weak: Color32, now: f64) -> egui::Vec2 {
    // one row as tall as the logo (the tallest item), so centring adds nothing
    let at = Rect::from_min_size(ui.max_rect().min, vec2(10_000.0, idle.look.logo_height));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(at)
            .layout(egui::Layout::left_to_right(egui::Align::Center))
            .invisible(),
    );
    draw_idle(&mut child, idle, weak, now);
    let m = watermark::margin(!idle.is_empty());
    child.min_rect().size() + vec2(m.leftf() + m.rightf(), m.topf() + m.bottomf())
}

/// Whether `a` turning into `b` updates in place (no fade): the idle state ticking, the
/// same broadcast with its timer ticking.
fn in_place(a: &Option<View>, b: &Option<View>) -> bool {
    match (a, b) {
        (Some(View::Idle(_)), Some(View::Idle(_))) => true,
        (Some(View::Broadcast { text: x, .. }), Some(View::Broadcast { text: y, .. })) => x == y,
        _ => false,
    }
}

impl Anim {
    /// Springs towards `view` and draws the island; `now` in seconds.
    pub fn show(&mut self, ui: &mut Ui, glass: Glass, view: Option<View>, now: f64) {
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
            let (s, r) = match v {
                View::Idle(idle) => {
                    let s = measure_idle(ui, idle, weak, now);
                    self.idle_size = s;
                    (s, watermark::corner(idle.look, s.y) as f32)
                }
                _ => {
                    let (s, pill) = measure(&lay_out(ui, v, weak));
                    // an event expands the island: never smaller than the idle state
                    let s = s.max(self.idle_size);
                    (s, if pill { s.y / 2.0 } else { radius::CARD as f32 })
                }
            };
            if hidden {
                // grow out of a small capsule
                self.width = Spring::new(s.y);
                self.height = Spring::new(s.y * 0.8);
                self.corner = Spring::new(s.y * 0.4);
            }
            self.width.update_damped(s.x, motion::SPRING, ZETA, dt);
            self.height.update_damped(s.y, motion::SPRING, ZETA, dt);
            self.corner.update_damped(r, motion::SPRING, ZETA, dt);
        } else if hidden {
            self.shown = None;
            return;
        }
        let on = view.is_some() as i32 as f32;
        self.opacity.update(on, motion::NORMAL, dt);
        let settled = (in_place(&self.shown, &view) || self.shown == view) as i32 as f32;
        self.content.update(on * settled, motion::FAST, dt);

        let size = vec2(self.width.value.max(0.0), self.height.value.max(0.0));
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
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
            Some(View::Idle(idle)) => {
                let content = self.idle_size;
                let m = watermark::margin(!idle.is_empty());
                let inner = Rect::from_center_size(rect.center(), content);
                let inner = Rect::from_min_max(
                    inner.min + vec2(m.leftf(), m.topf()),
                    inner.max - vec2(m.rightf(), m.bottomf()),
                );
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(inner)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                child.set_clip_rect(rect.intersect(ui.clip_rect()));
                child.set_opacity(alpha);
                draw_idle(&mut child, idle, weak, now);
            }
            Some(v) => paint_event(&p.with_clip_rect(rect), rect, &lay_out(ui, v, weak), alpha),
            None => {}
        }
    }
}

/// An event: the lead left of the rows, the trailing timer right, the group centred.
fn paint_event(p: &egui::Painter, rect: Rect, l: &Laid, alpha: f32) {
    let text = |pos, g: &Arc<Galley>| {
        p.add(TextShape::new(pos, g.clone(), Color32::TRANSPARENT).with_opacity_factor(alpha));
    };
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

    fn show(&mut self, ui: &mut Ui, data: &HudData, config: &HudConfig) {
        let client = data.client;
        // no snapshot yet: the idle state only (no events can have arrived)
        let tick = client.game_tick().unwrap_or(0);
        let local = client.local_id();
        let items = client.snapshot();
        let name_of = |id: i32| {
            (items.iter())
                .find(|it| it.type_id == g::NETOBJTYPE_CLIENTINFO && it.id == id)
                .and_then(|it| ints_to_str(&g::ObjClientInfo::from_ints(&it.data).name))
                .unwrap_or_default()
        };
        let local_name = local.map(name_of).unwrap_or_default();
        for msg in data.events {
            self.race.on_message(msg, local, tick);
            self.broadcast.on_message(msg, tick);
            if let GameMsg::SvChat(m) = msg
                && let Some(text) =
                    self.mentions
                        .on_chat(m, &name_of(m.client_id), local, &local_name)
            {
                let until = tick.wrapping_add(MENTION_TICKS);
                self.mention = Some((name_of(m.client_id), text, until));
            }
        }
        let on = |key: &str| {
            let d = ITEMS.iter().find(|i| i.0 == key).is_none_or(|i| i.2);
            config.on(ID, key, d)
        };
        let info = (items.iter())
            .find(|it| it.type_id == g::NETOBJTYPE_GAMEINFO)
            .map(|it| g::ObjGameInfo::from_ints(&it.data));
        let now = ui.input(|i| i.time);
        let timer = info
            .as_ref()
            .filter(|_| on("timer"))
            .map(|i| timer(i, tick, now));
        let spectator = spectating(items).filter(|_| on("spectator"));
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
            Some((icons::PAUSE, "Game paused".to_string()))
        } else {
            spectator.map(|s| (icons::EYE, s))
        };
        // the track change: a new title or artist (7.10 reports the track)
        let media = data.now_playing.cloned();
        let track = media.as_ref().map(|m| (m.title.clone(), m.artist.clone()));
        if track.is_some() && track != self.track {
            self.track_until = tick.wrapping_add(TRACK_TICKS);
        }
        self.track = track;
        let track = media
            .clone()
            .filter(|_| self.track_until.wrapping_sub(tick) > 0);
        let stats = (watermark::order(config).into_iter())
            .filter(|k| watermark::shown(config, k))
            .filter_map(|k| watermark::stat(k, data))
            .collect();
        let idle = Idle {
            compact: watermark::compact(config),
            look: Look::of(config),
            timer: timer.clone(),
            checkpoint,
            status,
            stats,
            media,
        };
        // DDNet's rule: every item off hides it (the editor shows the logo to grab)
        let idle = Some(idle).filter(|i| !i.is_empty() || data.editing);
        let mention = (self.mention.clone())
            .filter(|m| on("mentions") && m.2.wrapping_sub(tick) > 0)
            .map(|(a, t, _)| (a, t));
        let broadcast = (self.broadcast.shown(tick))
            .filter(|_| on("broadcasts"))
            .map(|b| (b.to_string(), timer));
        let v = view(idle, finish, mention, broadcast, track);
        self.anim.show(ui, data.glass, v, now);
    }

    fn settings(&mut self, ui: &mut Ui, config: &mut HudConfig) {
        item_toggles(ui, config, ID, &ITEMS);
        watermark::settings(ui, config);
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

    fn clock(t: Timer) -> (String, &'static str, bool, bool) {
        match t {
            Timer::Clock {
                text,
                template,
                warn,
                blink,
            } => (text, template, warn, blink),
            Timer::SuddenDeath => panic!("sudden death"),
        }
    }

    /// Cases worked out from `CHud::RenderGameTimer` (hud.cpp:118).
    #[test]
    fn timer_follows_render_game_timer() {
        let race = g::GAMESTATEFLAG_RACETIME;
        // race: (tick + warmup) / 50, the warmup being -start
        let c = clock(timer(&info(race, 0, -1000, 0), 1000 + 50 * 65 + 49, 0.0));
        assert_eq!(c, ("01:05".into(), "00:00", false, false));
        // round time; before the start the division truncates to 0, then negative = 0
        assert_eq!(clock(timer(&info(0, 100, 0, 0), 51, 0.0)).0, "00:00");
        assert_eq!(clock(timer(&info(0, 100, 0, 0), 49, 0.0)).0, "00:00");
        // days and hours pick the C++ width class
        let d = clock(timer(&info(0, 0, 0, 0), 50 * 90_000, 0.0));
        assert_eq!((d.0.as_str(), d.1), ("1d 01:00:00", "0d 00:00:00"));
        let h = clock(timer(&info(0, 0, 0, 0), 50 * 3600, 0.0));
        assert_eq!((h.0.as_str(), h.1), ("01:00:00", "00:00:00"));
        // time limit: counts down, red in the last 60 s, blinks in the last 10 on odd
        // half seconds, 0 when the game is over
        assert_eq!(
            clock(timer(&info(0, 0, 0, 2), 50 * 70, 0.0)),
            ("00:50".into(), "00:00", true, false)
        );
        assert!(!clock(timer(&info(0, 0, 0, 2), 50 * 59, 0.0)).2);
        assert!(clock(timer(&info(0, 0, 0, 2), 50 * 115, 0.6)).3);
        assert!(!clock(timer(&info(0, 0, 0, 2), 50 * 115, 1.1)).3);
        let over = g::GAMESTATEFLAG_GAMEOVER;
        assert_eq!(clock(timer(&info(over, 0, 0, 2), 50 * 30, 0.0)).0, "00:00");
        // a warmup hides the limit: plain round time
        let w = clock(timer(&info(0, 0, 5, 2), 50 * 30, 0.0));
        assert_eq!(w, ("00:30".into(), "00:00", false, false));
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
        let idle = Idle {
            compact: false,
            look: Look::default(),
            timer: Some(timer(&info(0, 0, 0, 0), 0, 0.0)),
            checkpoint: None,
            status: None,
            stats: Vec::new(),
            media: None,
        };
        let finish = Effect::Finish {
            time: "00:01.00".into(),
            diff: None,
            record: Record::None,
        };
        let mention = || Some(("a".to_string(), "hi tee".to_string()));
        let broadcast = || Some(("b".to_string(), None));
        let track = NowPlaying {
            title: "t".into(),
            artist: "a".into(),
            playing: true,
            cover: None,
        };
        let i = || Some(idle.clone());
        let t = || Some(track.clone());
        let v = view(i(), Some(finish.clone()), mention(), broadcast(), t());
        assert_eq!(v, Some(View::Finish(finish)));
        let v = view(i(), None, mention(), broadcast(), t());
        assert!(matches!(v, Some(View::Mention { .. })));
        let v = view(i(), None, None, broadcast(), t());
        assert!(matches!(v, Some(View::Broadcast { .. })));
        assert_eq!(
            view(i(), None, None, None, t()),
            Some(View::Track(track.clone()))
        );
        assert_eq!(
            view(i(), None, None, None, None),
            Some(View::Idle(idle.clone()))
        );
        assert_eq!(view(None, None, None, None, None), None);
        // pause: C++ RenderPauseNotification, not when the game is over
        assert!(paused(&info(g::GAMESTATEFLAG_PAUSED, 0, 0, 0)));
        let over = g::GAMESTATEFLAG_PAUSED | g::GAMESTATEFLAG_GAMEOVER;
        assert!(!paused(&info(over, 0, 0, 0)));
    }
}
