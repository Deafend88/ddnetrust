//! 7.10 review: the F7 player with synced lyrics, the "Now playing" card (display only,
//! with its lyrics line) and the lyrics line widget, over a real map, as one PNG. The
//! track is a stand-in (covers come from the OS media session).
//!
//! Usage: cargo run -p ddnet-rs --example media_review -- <map> <x> <y> <glass 0-2> <out.png>

use std::sync::Arc;

use ddnet::map::Map;
use ddnet_rs::assets;
use ddnet_rs::gfx::batch::Batch;
use ddnet_rs::gfx::{Gfx, Size};
use ddnet_rs::island::{Anim, NowPlaying, PLAYER_WIDTH, Player, View, lyrics_ui, player_ui};
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
    let mut anim = Anim::default();
    for i in 0..60 {
        let t = i as f64 / 30.0;
        let view = MapView {
            center,
            zoom: 1.0,
            width: size.width,
            height: size.height,
            envelope_time: 0,
        };
        let mut batch = Batch::default();
        let [r, g, b] = hsl_packed_to_rgb(128);
        batch.clear(r, g, b);
        render.render_background(&mut batch, &view);
        render.render_foreground(&mut batch, &view);
        let world = gfx.world_view();
        gfx.render(&mut batch, &world, size);
        let input = egui::RawInput {
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
            egui::Area::new(egui::Id::unique("island"))
                .anchor(Align2::CENTER_TOP, [0.0, 16.0])
                .show(ctx, |ui| {
                    anim.show(ui, glass, Some(View::Player(player(true))), t);
                });
            egui::Area::new(egui::Id::unique("card"))
                .anchor(Align2::LEFT_BOTTOM, [16.0, -16.0])
                .show(ctx, |ui| {
                    let margin = Margin::same(space::L as i8);
                    w::hud_bar(ui, glass, radius::CARD, margin, 0.0, |ui| {
                        ui.vertical(|ui| {
                            ui.set_width(PLAYER_WIDTH - 2.0 * space::L);
                            player_ui(ui, &player(false), false, 1, &mut None, weak);
                        });
                    });
                });
            egui::Area::new(egui::Id::unique("line"))
                .anchor(Align2::RIGHT_BOTTOM, [-16.0, -16.0])
                .show(ctx, |ui| {
                    let margin = Margin::symmetric(space::L as i8, space::S as i8);
                    let p = player(false);
                    w::hud_bar(ui, glass, radius::PILL, margin, 0.0, |ui| {
                        lyrics_ui(ui, p.lyrics.as_ref().unwrap(), Some(128.0), 1, weak);
                    });
                });
        });
    }
    gfx.screenshot(std::path::Path::new(&a[4])).unwrap();
    println!("saved {}", a[4]);
}
