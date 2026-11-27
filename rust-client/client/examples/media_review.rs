//! 7.10 review: the compact F7 player with its volume slider, the player with its lyrics
//! view open, the idle island with the lyrics as subtitles under it, and the "Now
//! playing" card, over a real map, as one PNG. The track is a stand-in (covers come from
//! the OS media session).
//!
//! Usage: cargo run -p ddnet-rs --example media_review -- <map> <x> <y> <glass 0-2> <out.png>

use std::sync::Arc;

use ddnet::map::Map;
use ddnet_rs::assets;
use ddnet_rs::gfx::batch::Batch;
use ddnet_rs::gfx::{Gfx, Size};
use ddnet_rs::island::{
    Anim, Controls, NowPlaying, PLAYER_WIDTH, Part, Player, Subtitles, Timer, View, player_ui,
};
use ddnet_rs::lyrics::Lyrics;
use ddnet_rs::render::map::{MapRender, View as MapView, hsl_packed_to_rgb};
use ddnet_rs::storage::Storage;
use ddnet_rs::style::{Glass, radius, space};
use ddnet_rs::ui::Painter;
use ddnet_rs::widgets as w;
use egui::{Align2, Margin};

fn player(controls: bool) -> Player {
    let lines = [
        (120.0, "Streetlights hum along the empty road"),
        (126.0, "Night drive, the city fades behind"),
        (132.0, "Hold the line until the morning light"),
    ];
    Player {
        track: Some((
            NowPlaying {
                title: "Night drive".into(),
                artist: "Ambient Tees".into(),
                playing: true,
                cover: None,
            },
            Some((128.0, 214.0)),
            [controls; 3],
        )),
        sessions: vec!["Spotify.exe".into()],
        source: None,
        lyrics: Some(Lyrics::Lines(Arc::new(
            lines.iter().map(|(t, s)| (*t, s.to_string())).collect(),
        ))),
        volume: controls.then_some(0.7),
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let center = (a[1].parse().unwrap(), a[2].parse().unwrap());
    let glass = Glass::from_setting(a[3].parse().unwrap());
    let storage = Storage::new(None).unwrap();
    let map = Map::load(std::fs::read(&a[0]).unwrap()).unwrap();
    let images = assets::load_map_images(&storage, &map);
    let size = Size {
        width: 1280,
        height: 720,
    };
    let mut gfx = Gfx::headless(size).expect("graphics adapter");
    let render = MapRender::new(&mut gfx, &map, &images);
    let mut painter = Painter::new(&gfx, gfx.format, glass);
    let (mut anim, mut idle, mut subtitles) =
        (Anim::default(), Anim::default(), Subtitles::default());
    let (mut seeking, mut volume, mut open, mut volume_open) = (None, None, true, None);
    for i in 0..60 {
        let t = i as f64 / 30.0;
        let view = MapView {
            center,
            zoom: 1.0,
            width: size.width,
            height: size.height,
            envelope_time: 0,
            overlay: 0,
        };
        let mut batch = Batch::default();
        let [r, g, b] = hsl_packed_to_rgb(128);
        batch.clear(r, g, b);
        render.render_background(&mut batch, &view);
        render.render_foreground(&mut batch, &view);
        let world = gfx.world_view();
        gfx.render(&mut batch, &world, size);
        // click the compact player's speaker once, so its volume popover shows
        let speaker = anim
            .rect
            .map(|r| egui::pos2(r.left() + space::L + 16.0, r.bottom() - space::L - 20.0));
        let mut events = Vec::new();
        if let Some(at) = speaker.filter(|_| i == 40 || i == 41) {
            events.push(egui::Event::PointerMoved(at));
            events.push(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: i == 40,
                modifiers: Default::default(),
            });
        }
        let input = egui::RawInput {
            events,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(size.width as f32, size.height as f32),
            )),
            time: Some(t),
            ..Default::default()
        };
        let world = gfx.world_view();
        let weak = glass.text_secondary();
        painter.paint(&gfx, &world, size, input, |ctx| {
            // the compact F7 player (top centre)
            egui::Area::new(egui::Id::unique("player"))
                .anchor(Align2::CENTER_TOP, [0.0, 16.0])
                .show(ctx, |ui| {
                    anim.show(ui, glass, Some(View::Player(player(true))), t);
                });
            // the player with its lyrics view open (left)
            egui::Area::new(egui::Id::unique("lyrics"))
                .anchor(Align2::LEFT_TOP, [16.0, 16.0])
                .show(ctx, |ui| {
                    let margin = Margin::same(space::L as i8);
                    w::hud_bar(ui, glass, radius::CARD, margin, 0.0, |ui| {
                        ui.vertical(|ui| {
                            ui.set_width(PLAYER_WIDTH - 2.0 * space::L);
                            let mut c = Controls {
                                seeking: &mut seeking,
                                lyrics_open: &mut open,
                                volume: &mut volume,
                                volume_open: &mut volume_open,
                                card: ui.max_rect(),
                                glass,
                            };
                            player_ui(ui, &player(true), Some(&mut c), 0, weak);
                        });
                    });
                });
            // the idle island with the subtitles (right)
            egui::Area::new(egui::Id::unique("idle"))
                .anchor(Align2::RIGHT_TOP, [-120.0, 16.0])
                .show(ctx, |ui| {
                    let timer = Timer::Clock {
                        text: "16:07".into(),
                        warn: false,
                        blink: false,
                    };
                    let np = NowPlaying {
                        title: "Night drive".into(),
                        artist: "Ambient Tees".into(),
                        playing: true,
                        cover: None,
                    };
                    let v = View::Idle(vec![Part::Timer(timer), Part::Media(np)]);
                    idle.show(ui, glass, Some(v), t);
                    if let Some(rect) = idle.rect {
                        let line = Some("Night drive, the city fades behind");
                        subtitles.show(ui, rect, line, 1.0 / 30.0);
                    }
                });
            // the "Now playing" card, no lyrics (bottom left)
            egui::Area::new(egui::Id::unique("card"))
                .anchor(Align2::LEFT_BOTTOM, [16.0, -16.0])
                .show(ctx, |ui| {
                    let margin = Margin::same(space::L as i8);
                    w::hud_bar(ui, glass, radius::CARD, margin, 0.0, |ui| {
                        ui.vertical(|ui| {
                            ui.set_width(PLAYER_WIDTH - 2.0 * space::L);
                            player_ui(ui, &player(false), None, 0, weak);
                        });
                    });
                });
        });
    }
    gfx.screenshot(std::path::Path::new(&a[4])).unwrap();
    println!("saved {}", a[4]);
}
