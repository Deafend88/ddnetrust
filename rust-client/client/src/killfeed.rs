//! Killfeed capsules (PLAN §10.3, 7.8; replaces `CInfoMessages`, infomessages.cpp): kill,
//! team kill and finish messages as capsules top right that slide in and fade out. The
//! entries, their count limit (5, the oldest overwritten) and lifetime (10 s of game
//! ticks) follow C++; `Feed` is that logic, the drawing is ours.

use ddnet::map::ints_to_str;
use ddnet::protocol::generated::{self as g, GameMsg};
use egui::emath::TSTransform;
use egui::{Align, Color32, Layout, Margin, Ui};

use crate::anim::ease_out;
use crate::hud::{Anchor, HudConfig, HudData, Placement, Widget};
use crate::icons;
use crate::statusbar::WeaponIcons;
use crate::storage::Storage;
use crate::style::{Weight, color, motion, size, space};
use crate::timefmt::{TimeFormat, str_time_float};
use crate::widgets as w;

/// C++ `MAX_INFOMSGS`.
pub const MAX: usize = 5;
/// C++: an entry shows until `GameTick > m_Tick + GameTickSpeed() * 10`.
pub const LIFETIME_TICKS: i32 = 50 * 10;

/// What an entry says.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// `Sv_KillMsg`: the victim and the killer (id, name) when they are active clients,
    /// and the weapon (negative: self, world or game).
    Kill {
        victim: Option<(i32, String)>,
        killer: Option<(i32, String)>,
        weapon: i32,
    },
    /// `Sv_KillMsgTeam`: a DDRace team died. C++ lists up to four members (from the
    /// teams state, not ported); the entry is kept with its name.
    TeamKill { team: i32 },
    /// `Sv_RaceFinish`: who, the time and the difference to their record as C++ formats
    /// them, and whether it is a record.
    Finish {
        name: String,
        time: String,
        diff: Option<String>,
        faster: bool,
        record: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// The game tick it arrived at (C++ `m_Tick`).
    pub tick: i32,
    pub kind: Kind,
}

/// C++: `CInfoMessages::m_aInfoMsgs` and `m_InfoMsgCurrent`.
#[derive(Debug, Default)]
pub struct Feed {
    slots: [Option<Entry>; MAX],
    current: usize,
}

impl Feed {
    /// C++: `AddInfoMsg`: the next slot of the ring is overwritten.
    fn add(&mut self, entry: Entry) {
        self.current = (self.current + 1) % MAX;
        self.slots[self.current] = Some(entry);
    }

    /// C++: `CInfoMessages::OnMessage`. `name` is a client's name if it is active (C++
    /// `m_aClients[id].m_Active`: its `PlayerInfo` is in the snapshot).
    pub fn on_message(&mut self, msg: &GameMsg, name: impl Fn(i32) -> Option<String>, tick: i32) {
        let kind = match msg {
            GameMsg::SvKillMsg(m) => {
                let victim = name(m.victim).map(|n| (m.victim, n));
                let killer = name(m.killer).map(|n| (m.killer, n));
                if victim.is_none() && killer.is_none() && m.weapon < 0 {
                    return; // message would be empty
                }
                Kind::Kill {
                    victim,
                    killer,
                    weapon: m.weapon,
                }
            }
            GameMsg::SvKillMsgTeam(m) => Kind::TeamKill { team: m.team },
            GameMsg::SvRaceFinish(m) => {
                let Some(name) = name(m.client_id) else {
                    return;
                };
                let format = TimeFormat::HoursCentisecs;
                // C++: absolute(m_Diff) / 1000.0f, "(%c%s)"
                let diff = (m.diff != 0).then(|| {
                    let t = str_time_float(m.diff.wrapping_abs() as f32 / 1000.0, format);
                    format!("({}{t})", if m.diff < 0 { '-' } else { '+' })
                });
                Kind::Finish {
                    name,
                    time: str_time_float(m.time as f32 / 1000.0, format),
                    diff,
                    faster: m.diff < 0,
                    record: m.record_personal != 0 || m.record_server != 0,
                }
            }
            _ => return,
        };
        self.add(Entry { tick, kind });
    }

    /// C++: `CInfoMessages::OnRender`'s walk: the shown entries, oldest first; expired
    /// ones are dropped.
    pub fn entries(&mut self, tick: i32) -> Vec<&Entry> {
        for slot in &mut self.slots {
            if slot
                .as_ref()
                .is_some_and(|e| tick > e.tick.wrapping_add(LIFETIME_TICKS))
            {
                *slot = None;
            }
        }
        (1..=MAX)
            .filter_map(|i| self.slots[(self.current + i) % MAX].as_ref())
            .collect()
    }
}

/// A client's name if C++ counts it as active: its `PlayerInfo` (client id = item id)
/// is in the snapshot; the name comes from its `ClientInfo`.
fn active_name(items: &[ddnet::protocol::snapshot::Item], id: i32) -> Option<String> {
    let active = items.iter().any(|it| {
        it.type_id == g::NETOBJTYPE_PLAYERINFO
            && it.id == id
            && g::ObjPlayerInfo::from_ints(&it.data).client_id == id
    });
    let info = items
        .iter()
        .find(|it| it.type_id == g::NETOBJTYPE_CLIENTINFO && it.id == id);
    active.then(|| {
        info.and_then(|it| ints_to_str(&g::ObjClientInfo::from_ints(&it.data).name))
            .unwrap_or_default()
    })
}

pub struct Killfeed {
    feed: Feed,
    weapons: WeaponIcons,
}

impl Killfeed {
    pub fn new(storage: &Storage) -> Self {
        Self {
            feed: Feed::default(),
            weapons: WeaponIcons::new(storage),
        }
    }
}

/// A piece of a capsule.
enum Part<'a> {
    Name(&'a str),
    Icon(&'static str),
    Weapon(usize),
    /// A secondary text (the time) or a coloured one (the difference).
    Text(&'a str, Weight, Color32),
}

/// What a capsule shows, left to right. Kills read like C++ (killer, weapon, victim; the
/// killer only when it is not the victim); a kill without a weapon shows a skull.
fn parts(kind: &Kind, weak: Color32) -> Vec<Part<'_>> {
    match kind {
        Kind::Kill {
            victim,
            killer,
            weapon,
        } => {
            let mut p = Vec::new();
            if let Some((id, n)) = killer
                && victim.as_ref().map(|v| v.0) != Some(*id)
            {
                p.push(Part::Name(n));
            }
            p.push(if *weapon >= 0 {
                Part::Weapon(*weapon as usize)
            } else {
                Part::Icon(icons::SKULL)
            });
            if let Some((_, n)) = victim {
                p.push(Part::Name(n));
            }
            p
        }
        Kind::TeamKill { .. } => vec![Part::Icon(icons::SKULL)],
        Kind::Finish {
            name,
            time,
            diff,
            faster,
            record,
        } => {
            let mut p = vec![
                Part::Icon(if *record {
                    icons::TROPHY
                } else {
                    icons::FLAG_CHECKERED
                }),
                Part::Name(name),
                Part::Text(time, Weight::Regular, weak),
            ];
            if let Some(d) = diff {
                let c = if *faster {
                    color::SUCCESS
                } else {
                    color::DANGER
                };
                p.push(Part::Text(d, Weight::Medium, c));
            }
            p
        }
    }
}

/// One capsule. Its row runs right to left (the column is right-aligned), so the parts
/// go in reversed.
fn capsule(ui: &mut Ui, data: &HudData, weapons: &mut WeaponIcons, kind: &Kind) {
    let margin = Margin::symmetric(space::L as i8, space::S as i8);
    let weak = ui.visuals().weak_text_color();
    // C++: Localize("Team %d")
    let team = match kind {
        Kind::TeamKill { team } => format!("Team {team}"),
        _ => String::new(),
    };
    let mut parts = parts(kind, weak);
    if !team.is_empty() {
        parts.push(Part::Name(&team));
    }
    w::hud_bar(ui, data.glass, false, margin, size::ICON, |ui| {
        ui.spacing_mut().item_spacing.x = space::S;
        for part in parts.iter().rev() {
            match *part {
                Part::Name(n) => {
                    w::text(ui, n, size::BODY, Weight::Medium, color::TEXT);
                }
                Part::Icon(glyph) => {
                    w::icon(ui, glyph, size::ICON, color::TEXT);
                }
                Part::Weapon(weapon) => weapons.draw(ui, weapon, 1.0),
                Part::Text(t, weight, c) => {
                    w::text(ui, t, size::BODY, weight, c);
                }
            }
        }
    });
}

impl Widget for Killfeed {
    fn id(&self) -> &'static str {
        "killfeed"
    }

    fn name(&self) -> &'static str {
        "Kills and finishes"
    }

    fn icon(&self) -> &'static str {
        icons::SKULL
    }

    fn default_placement(&self) -> Placement {
        Placement {
            anchor: Anchor::TopRight,
            x: -16,
            y: 16,
            enabled: true,
        }
    }

    fn show(&mut self, ui: &mut Ui, data: &HudData, _: &HudConfig) {
        let client = data.client;
        let Some(tick) = client.game_tick() else {
            return;
        };
        let items = client.snapshot();
        for msg in data.events {
            self.feed.on_message(msg, |id| active_name(items, id), tick);
        }
        let entries: Vec<Entry> = self.feed.entries(tick).into_iter().cloned().collect();
        let sample = Entry {
            tick,
            kind: Kind::Finish {
                name: "nameless tee".into(),
                time: "00:42.10".into(),
                diff: Some("(-00.36)".into()),
                faster: true,
                record: false,
            },
        };
        let entries = if entries.is_empty() && data.editing {
            vec![sample]
        } else {
            entries
        };
        ui.with_layout(Layout::top_down(Align::Max), |ui| {
            ui.spacing_mut().item_spacing.y = space::S;
            for e in &entries {
                // slide in from the right over `motion::NORMAL`, fade out over the last
                // `motion::NORMAL` of its life
                let age = ((tick - e.tick) as f32 + client.intra_tick) / 50.0;
                let left = LIFETIME_TICKS as f32 / 50.0 - age;
                let shown = if data.editing {
                    1.0
                } else {
                    ease_out(age / motion::NORMAL)
                };
                let alpha = (left / motion::NORMAL).clamp(0.0, 1.0);
                let slide =
                    TSTransform::from_translation(egui::vec2((1.0 - shown) * space::XXL, 0.0));
                ui.scope(|ui| {
                    ui.set_opacity(if data.editing { 1.0 } else { alpha });
                    ui.with_visual_transform(slide, |ui| {
                        capsule(ui, data, &mut self.weapons, &e.kind)
                    });
                });
            }
        });
    }
}
