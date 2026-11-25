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
use crate::hud::{Anchor, HudConfig, HudData, Placement, Widget, item_toggles};
use crate::icons;
use crate::style::{
    Glass, HUD_SHADOW, Weight, color, font, icon_font, motion, radius, size, space,
};
use crate::timefmt::{TimeFormat, str_time, str_time_float};

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
    /// Idle: the race timer with the checkpoint difference, after a status (pause or
    /// spectator text) when there is one.
    Timer {
        status: Option<(&'static str, String)>,
        timer: Option<Timer>,
        checkpoint: Option<Effect>,
    },
    /// The finish time (an `Effect::Finish`).
    Finish(Effect),
    /// A server broadcast; the timer stays at its end (C++ shows both).
    Broadcast { text: String, timer: Option<Timer> },
}

/// The view by priority: finish > broadcast > the timer; `None`: nothing to show.
pub fn view(
    finish: Option<Effect>,
    broadcast: Option<String>,
    status: Option<(&'static str, String)>,
    timer: Option<Timer>,
    checkpoint: Option<Effect>,
) -> Option<View> {
    if let Some(f) = finish {
        return Some(View::Finish(f));
    }
    if let Some(text) = broadcast {
        return Some(View::Broadcast { text, timer });
    }
    (status.is_some() || timer.is_some()).then_some(View::Timer {
        status,
        timer,
        checkpoint,
    })
}

/// The island's items (`hud_set island <key> 0|1`), first values from C++.
const ITEMS: [(&str, &str, bool); 4] = [
    ("timer", "Race timer", true),
    ("checkpoint", "Checkpoint difference", true),
    ("broadcasts", "Broadcasts", true),
    ("spectator", "Spectator text", true),
];

/// The island widget: the race state, the broadcast and the springs.
#[derive(Default)]
pub struct Island {
    race: Race,
    broadcast: Broadcast,
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
    /// The idle size: events are at least this big (they expand the island).
    idle_size: egui::Vec2,
    last: Option<f64>,
}

/// The island's top offset, the broadcast's wrap width (points).
const TOP: i32 = 16;
const WRAP: f32 = 480.0;
/// Damping of the size springs: a small overshoot (about 3 %), the island's motion.
const ZETA: f32 = 0.75;

/// A laid-out view: an optional icon, one or more rows, an optional trailing timer.
struct Laid {
    icon: Option<Arc<Galley>>,
    rows: Vec<Arc<Galley>>,
    /// The width of the rows.
    width: f32,
    trail: Option<Arc<Galley>>,
}

fn job(spans: &[(&str, f32, Weight, Color32)], wrap: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    for &(text, size, weight, color) in spans {
        job.append(text, 0.0, TextFormat::simple(font(size, weight), color));
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
        Some(ctx.fonts_mut(|f| f.layout_no_wrap(glyph.into(), icon_font(size), c)))
    };
    let laid = |icon, rows: Vec<Arc<Galley>>, trail| Laid {
        icon,
        width: rows.iter().map(|r| r.size().x).fold(0.0, f32::max),
        rows,
        trail,
    };
    let inf = f32::INFINITY;
    let timer_galley = |t: &Timer| {
        let (s, c) = timer_text(t);
        galley(job(&[(&s, size::TITLE, Weight::SemiBold, c)], inf))
    };
    match view {
        View::Timer {
            status,
            timer,
            checkpoint,
        } => {
            // status text, then the timer and its checkpoint difference; digits are
            // tabular (style.rs), so the width only changes with the timer's class
            let mut spans = Vec::new();
            let (t, cp);
            if let Some((_, text)) = status {
                spans.push((text.as_str(), size::BODY, Weight::Medium, weak));
            }
            if let Some(timer) = timer {
                let (s, c) = timer_text(timer);
                t = if status.is_some() {
                    format!("   {s}")
                } else {
                    s
                };
                spans.push((t.as_str(), size::TITLE, Weight::SemiBold, c));
            }
            if let Some(Effect::Checkpoint { text, sign }) = checkpoint {
                let c = match sign {
                    1 => color::DANGER,
                    -1 => color::SUCCESS,
                    _ => color::TEXT,
                };
                cp = format!("   {text}");
                spans.push((cp.as_str(), size::BODY, Weight::Medium, c));
            }
            let lead = status.as_ref().and_then(|(g, _)| icon(g, size::ICON, weak));
            laid(lead, vec![galley(job(&spans, inf))], None)
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
        View::Finish(_) => laid(None, Vec::new(), None),
        View::Broadcast { text, timer } => {
            let row = galley(job(
                &[(text, size::BODY, Weight::Medium, color::TEXT)],
                WRAP,
            ));
            let lead = icon(icons::MEGAPHONE, size::ICON, color::TEXT);
            laid(lead, vec![row], timer.as_ref().map(timer_galley))
        }
    }
}

/// The padded size of a laid-out view and whether it is one line (a pill).
fn measure(l: &Laid) -> (egui::Vec2, bool) {
    let rows_h: f32 = l.rows.iter().map(|r| r.size().y).sum();
    let one_line = l.rows.len() == 1 && l.rows[0].rows.len() == 1;
    let icon_w = l.icon.as_ref().map_or(0.0, |i| i.size().x + space::M);
    let trail_w = l.trail.as_ref().map_or(0.0, |t| t.size().x + space::XL);
    let py = if one_line { space::S } else { space::M };
    let h = rows_h + 2.0 * py;
    // a pill is at least as wide as tall
    let w = (icon_w + l.width + trail_w + 2.0 * space::XL).max(h + space::XXL);
    (vec2(w, h), one_line)
}

/// Whether `a` turning into `b` updates in place (no fade): the timer ticking, the same
/// broadcast with its timer ticking.
fn in_place(a: &Option<View>, b: &Option<View>) -> bool {
    match (a, b) {
        (Some(View::Timer { .. }), Some(View::Timer { .. })) => true,
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
            let (s, pill) = measure(&lay_out(ui, v, weak));
            let s = if matches!(v, View::Timer { .. }) {
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

        if let Some(shown) = &self.shown {
            let alpha = (self.content.value * opacity).clamp(0.0, 1.0);
            let laid = lay_out(ui, shown, weak);
            paint_content(&p.with_clip_rect(rect), rect, &laid, alpha);
        }
    }
}

/// The icon left of the rows, the trailing timer right, the group centred.
fn paint_content(p: &egui::Painter, rect: Rect, l: &Laid, alpha: f32) {
    let text = |pos, g: &Arc<Galley>| {
        p.add(TextShape::new(pos, g.clone(), Color32::TRANSPARENT).with_opacity_factor(alpha));
    };
    let icon_w = l.icon.as_ref().map_or(0.0, |i| i.size().x + space::M);
    let trail_w = l.trail.as_ref().map_or(0.0, |t| t.size().x + space::XL);
    let rows_h: f32 = l.rows.iter().map(|r| r.size().y).sum();
    let left = rect.center().x - (icon_w + l.width + trail_w) / 2.0;
    if let Some(icon) = &l.icon {
        text(pos2(left, rect.center().y - icon.size().y / 2.0), icon);
    }
    let mut y = rect.center().y - rows_h / 2.0;
    for row in &l.rows {
        text(pos2(left + icon_w, y), row);
        y += row.size().y;
    }
    if let Some(t) = &l.trail {
        let x = left + icon_w + l.width + space::XL;
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
        let Some(tick) = client.game_tick() else {
            return;
        };
        let local = client.local_id();
        for msg in data.events {
            self.race.on_message(msg, local, tick);
            self.broadcast.on_message(msg, tick);
        }
        let on = |key: &str| {
            let d = ITEMS.iter().find(|i| i.0 == key).is_none_or(|i| i.2);
            config.on("island", key, d)
        };
        let items = client.snapshot();
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
        let broadcast = (self.broadcast.shown(tick))
            .filter(|_| on("broadcasts"))
            .map(str::to_string);
        let mut v = view(finish, broadcast, status, timer, checkpoint);
        if v.is_none() && data.editing {
            let timer = Timer::Clock {
                text: "00:00".into(),
                warn: false,
                blink: false,
            };
            v = view(None, None, None, Some(timer), None);
        }
        self.anim.show(ui, data.glass, v, now);
    }

    fn settings(&mut self, ui: &mut Ui, config: &mut HudConfig) {
        item_toggles(ui, config, "island", &ITEMS);
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
        let t = || Some(timer(&info(0, 0, 0, 0), 0, 0.0));
        let finish = Effect::Finish {
            time: "00:01.00".into(),
            diff: None,
            record: Record::None,
        };
        let cp = || {
            Some(Effect::Checkpoint {
                text: "+00.10".into(),
                sign: 1,
            })
        };
        let status = || Some((icons::PAUSE, "Game paused".to_string()));
        let b = || Some("b".to_string());
        let v = view(Some(finish.clone()), b(), status(), t(), cp());
        assert_eq!(v, Some(View::Finish(finish)));
        let v = view(None, b(), status(), t(), cp());
        let broadcast = View::Broadcast {
            text: "b".into(),
            timer: t(),
        };
        assert_eq!(v, Some(broadcast));
        let v = view(None, None, status(), t(), cp());
        let idle = View::Timer {
            status: status(),
            timer: t(),
            checkpoint: cp(),
        };
        assert_eq!(v, Some(idle));
        assert!(view(None, None, status(), None, None).is_some());
        assert_eq!(view(None, None, None, None, cp()), None);
        // pause: C++ RenderPauseNotification, not when the game is over
        assert!(paused(&info(g::GAMESTATEFLAG_PAUSED, 0, 0, 0)));
        let over = g::GAMESTATEFLAG_PAUSED | g::GAMESTATEFLAG_GAMEOVER;
        assert!(!paused(&info(over, 0, 0, 0)));
    }
}
