//! The "Now playing" card (7.10 review): the F7 player's look without its controls
//! (cover, title, artist, progress and times; the lyrics line when the lyrics' place is
//! the card), display only, placed in the HUD editor, off by default.

use egui::{Margin, Ui};

use crate::hud::{Anchor, HudConfig, HudData, Placement, Widget};
use crate::icons;
use crate::island::{CoverCache, NowPlaying, Player, player_ui};
use crate::style::{radius, space};
use crate::widgets as w;

/// The player's content for the current media state (no session: none).
fn player(data: &HudData, cover: &mut CoverCache, ui: &Ui) -> Option<Player> {
    let state = data.media?.state();
    let t = state.track.as_ref()?;
    let ctx = ui.ctx().clone();
    let np = NowPlaying {
        title: t.title.clone(),
        artist: t.artist.clone(),
        playing: t.playing,
        cover: cover.get(&ctx, t.cover.as_ref()),
    };
    let timeline = t.position(std::time::Instant::now());
    Some(Player {
        track: Some((np, timeline, [false; 3])),
        sessions: Vec::new(),
        source: None,
        lyrics: state.lyrics.clone(),
        volume: None,
    })
}

/// The sample the HUD editor shows while nothing plays, so the widget can be placed.
fn sample() -> Player {
    let np = NowPlaying {
        title: "Now playing".into(),
        artist: "Play music in any app".into(),
        playing: false,
        cover: None,
    };
    Player {
        track: Some((np, Some((42.0, 180.0)), [false; 3])),
        sessions: Vec::new(),
        source: None,
        lyrics: None,
        volume: None,
    }
}

/// The "Now playing" card.
#[derive(Default)]
pub struct NowPlayingCard {
    cover: CoverCache,
}

impl Widget for NowPlayingCard {
    fn id(&self) -> &'static str {
        "nowplaying"
    }

    fn name(&self) -> &'static str {
        "Now playing"
    }

    fn icon(&self) -> &'static str {
        icons::MUSIC_NOTES
    }

    fn default_placement(&self) -> Placement {
        Placement {
            anchor: Anchor::BottomLeft,
            x: 16,
            y: -16,
            enabled: false,
        }
    }

    fn show(&mut self, ui: &mut Ui, data: &HudData, config: &HudConfig) {
        let Some(p) = player(data, &mut self.cover, ui).or(data.editing.then(sample)) else {
            return;
        };
        let lines = (crate::island::lyrics_place(config) == "card") as usize;
        let weak = data.glass.text_secondary();
        let margin = Margin::same(space::L as i8);
        w::hud_bar(ui, data.glass, radius::CARD, margin, 0.0, |ui| {
            ui.vertical(|ui| {
                ui.set_width(crate::island::PLAYER_WIDTH - 2.0 * space::L);
                player_ui(ui, &p, None, lines, weak);
            });
        });
    }
}
