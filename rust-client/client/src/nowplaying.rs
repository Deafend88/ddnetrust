//! Media HUD widgets (7.10 review): the "Now playing" card (the F7 player's look without
//! its controls: cover, title, artist, progress and times, optionally the lyrics line)
//! and the lyrics line (the current synced line in a pill, under the island by default).
//! Both are display only, placed in the HUD editor, off by default.

use egui::{Margin, Ui};

use crate::hud::{Anchor, HudConfig, HudData, Placement, Widget, item_toggles};
use crate::icons;
use crate::island::{CoverCache, NowPlaying, Player, lyrics_ui, player_ui};
use crate::lyrics::Lyrics;
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
        let lines = config.on("nowplaying", "lyrics", false) as usize;
        let weak = data.glass.text_secondary();
        let margin = Margin::same(space::L as i8);
        w::hud_bar(ui, data.glass, radius::CARD, margin, 0.0, |ui| {
            ui.vertical(|ui| {
                ui.set_width(crate::island::PLAYER_WIDTH - 2.0 * space::L);
                player_ui(ui, &p, false, lines, &mut None, weak);
            });
        });
    }

    fn settings(&mut self, ui: &mut Ui, config: &mut HudConfig) {
        item_toggles(
            ui,
            config,
            "nowplaying",
            &[("lyrics", "Lyrics line", false)],
        );
    }
}

/// The lyrics line under the island.
#[derive(Default)]
pub struct LyricsLine {
    cover: CoverCache,
}

impl Widget for LyricsLine {
    fn id(&self) -> &'static str {
        "lyrics"
    }

    fn name(&self) -> &'static str {
        "Lyrics line"
    }

    fn icon(&self) -> &'static str {
        icons::MUSIC_NOTES
    }

    fn default_placement(&self) -> Placement {
        // under the island (16 + its 40 pt + 8)
        Placement {
            anchor: Anchor::Top,
            x: 0,
            y: 64,
            enabled: false,
        }
    }

    fn show(&mut self, ui: &mut Ui, data: &HudData, _config: &HudConfig) {
        let p = player(data, &mut self.cover, ui);
        let lyrics = p.as_ref().and_then(|p| p.lyrics.clone());
        let position = p.as_ref().and_then(|p| p.track.as_ref()?.1).map(|t| t.0);
        // only a sung line shows; notes (loading, none found) stay in the player
        let lyrics = match lyrics {
            Some(Lyrics::Lines(l)) => Lyrics::Lines(l),
            _ if data.editing => Lyrics::Lines(std::sync::Arc::new(vec![(0.0, "♪ lyrics".into())])),
            _ => return,
        };
        let position = position.or(data.editing.then_some(1.0));
        let weak = data.glass.text_secondary();
        let margin = Margin::symmetric(space::L as i8, space::S as i8);
        w::hud_bar(ui, data.glass, radius::PILL, margin, 0.0, |ui| {
            lyrics_ui(ui, &lyrics, position, 1, weak);
        });
    }
}
