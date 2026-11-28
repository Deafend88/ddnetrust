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

    /// C++: the first lines of `CVoting::Render`: whether the notice shows, and the
    /// running vote's reset when its time is up (C++ resets only while it would show).
    pub fn shown(&mut self, now: i64, show_after_voting: bool, scoreboard: bool) -> bool {
        if (!show_after_voting && !scoreboard && self.voted != 0) || !self.is_voting() {
            return false;
        }
        if self.seconds_left(now) < 0 {
            self.on_reset();
            return false;
        }
        true
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

/// What the vote notice draws: the running vote, seconds left, the keys bound to
/// `vote yes` and `vote no` (C++ `m_Binds.GetKey`, empty when unbound).
pub struct Notice<'a> {
    pub voting: &'a Voting,
    pub seconds: i32,
    pub keys: [String; 2],
}

/// The sample the HUD editor shows while no vote runs.
fn sample() -> Voting {
    Voting {
        description: "Change map to Sunny Side Up".into(),
        reason: "fun".into(),
        yes: 3,
        no: 1,
        total: 6,
        close_time: 1,
        ..Default::default()
    }
}

/// The vote notice (C++ `CVoting::Render` in our style): a rounded card with the vote,
/// its reason, the yes/no bar and the two keys; the chosen side is tinted.
pub struct VoteNotice;

/// The notice's width (points).
const WIDTH: f32 = 320.0;

/// The yes/no bar: yes fills from the left, no from the right, a mark at the middle (the
/// pass line of a simple majority), like C++ `RenderBars`.
fn bars(ui: &mut egui::Ui, v: &Voting) {
    use crate::style::{color, space};
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), space::XS),
        egui::Sense::hover(),
    );
    let p = ui.painter();
    let r = egui::CornerRadius::same(crate::style::radius::PILL);
    p.rect_filled(rect, r, color::SURFACE_HOVER);
    if v.total != 0 {
        // C++: Bars.w * m_Yes / m_Total (float times int, then divided)
        let w = rect.width();
        if v.yes != 0 {
            let yes = egui::Rect::from_min_size(
                rect.min,
                egui::vec2(w * v.yes as f32 / v.total as f32, rect.height()),
            );
            p.rect_filled(yes, r, color::SUCCESS);
        }
        if v.no != 0 {
            let nw = w * v.no as f32 / v.total as f32;
            let no = egui::Rect::from_min_max(egui::pos2(rect.right() - nw, rect.top()), rect.max);
            p.rect_filled(no, r, color::DANGER);
        }
    }
    let mid = rect.center().x;
    p.line_segment(
        [
            egui::pos2(mid, rect.top() - 2.0),
            egui::pos2(mid, rect.bottom() + 2.0),
        ],
        egui::Stroke::new(1.0, color::TEXT_SECONDARY),
    );
}

/// A key capsule and its label ("F3 Yes"); `chosen`: our vote, tinted in its colour.
fn choice(
    ui: &mut egui::Ui,
    key: &str,
    label: &str,
    tint: egui::Color32,
    chosen: bool,
    weak: egui::Color32,
) {
    use crate::style::{Weight, color, font, radius, size, space};
    let fill = if chosen {
        tint.gamma_multiply(0.18)
    } else {
        color::SURFACE_RAISED
    };
    let text = if chosen { tint } else { color::TEXT };
    egui::Frame::new()
        .fill(fill)
        .corner_radius(egui::CornerRadius::same(radius::PILL))
        .inner_margin(egui::Margin::symmetric(space::M as i8, space::XS as i8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = space::S;
                if !key.is_empty() {
                    let k = egui::RichText::new(key.to_uppercase())
                        .font(font(size::CAPTION, Weight::SemiBold));
                    ui.label(k.color(if chosen { tint } else { weak }));
                }
                ui.label(
                    egui::RichText::new(label)
                        .font(font(size::BODY, Weight::Medium))
                        .color(text),
                );
            });
        });
}

impl crate::hud::Widget for VoteNotice {
    fn id(&self) -> &'static str {
        "vote"
    }

    fn name(&self) -> &'static str {
        "Vote"
    }

    fn icon(&self) -> &'static str {
        crate::icons::USERS
    }

    fn default_placement(&self) -> crate::hud::Placement {
        // C++ draws it at the left edge, a fifth down; ours sits at the left, centred
        crate::hud::Placement {
            anchor: crate::hud::Anchor::Left,
            x: 16,
            y: 0,
            enabled: true,
        }
    }

    fn show(&mut self, ui: &mut egui::Ui, data: &crate::hud::HudData, _: &crate::hud::HudConfig) {
        use crate::style::{Weight, color, font, radius, size, space};
        let editing_sample;
        let (v, seconds, keys) = match &data.vote {
            Some(n) => (n.voting, n.seconds, n.keys.clone()),
            None if data.editing => {
                editing_sample = sample();
                (&editing_sample, 24, ["f3".to_string(), "f4".to_string()])
            }
            None => return,
        };
        let weak = data.glass.text_secondary();
        let margin = egui::Margin::same(space::L as i8);
        crate::widgets::hud_bar(ui, data.glass, radius::CARD, margin, 0.0, |ui| {
            ui.vertical(|ui| {
                ui.set_width(WIDTH - 2.0 * space::L);
                ui.spacing_mut().item_spacing = egui::vec2(space::S, space::S);
                ui.horizontal(|ui| {
                    // seconds left on the right, the description cut short before it
                    let secs = egui::RichText::new(format!("{seconds}s"))
                        .font(font(size::BODY, Weight::Medium))
                        .color(weak);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(secs);
                        let t = egui::RichText::new(&v.description)
                            .font(font(size::LABEL, Weight::SemiBold))
                            .color(color::TEXT);
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(egui::Label::new(t).truncate());
                        });
                    });
                });
                if !v.reason.is_empty() {
                    let t = egui::RichText::new(&v.reason)
                        .font(font(size::BODY, Weight::Regular))
                        .color(weak);
                    ui.add(egui::Label::new(t).truncate());
                }
                ui.add_space(space::XS);
                bars(ui, v);
                ui.add_space(space::XS);
                // both capsules centred on the same row rect
                let size = egui::vec2(ui.available_width(), size::CONTROL);
                let (row, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                let mut side = |layout: egui::Layout, add: &mut dyn FnMut(&mut egui::Ui)| {
                    ui.scope_builder(egui::UiBuilder::new().max_rect(row).layout(layout), |ui| {
                        add(ui)
                    });
                };
                side(
                    egui::Layout::left_to_right(egui::Align::Center),
                    &mut |ui| choice(ui, &keys[0], "Yes", color::SUCCESS, v.voted == 1, weak),
                );
                side(
                    egui::Layout::right_to_left(egui::Align::Center),
                    &mut |ui| choice(ui, &keys[1], "No", color::DANGER, v.voted == -1, weak),
                );
            });
        });
    }
}
