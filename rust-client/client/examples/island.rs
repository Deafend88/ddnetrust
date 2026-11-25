//! M7.9 review: renders the Dynamic Island's transitions over a real map, frame by frame
//! (30 fps), through a scripted sequence of states; the frames become the review GIF.
//!
//! Usage: cargo run -p ddnet-rs --example island -- <map> <x> <y> <glass 0-2> <out dir>

use ddnet::map::Map;
use ddnet_rs::assets;
use ddnet_rs::gfx::batch::Batch;
use ddnet_rs::gfx::{Gfx, Size};
use ddnet_rs::icons;
use ddnet_rs::island::{Anim, Effect, Idle, NowPlaying, Record, Timer, View};
use ddnet_rs::render::map::{MapRender, View as MapView, hsl_packed_to_rgb};
use ddnet_rs::storage::Storage;
use ddnet_rs::style::Glass;
use ddnet_rs::timefmt::{TimeFormat, str_time};
use ddnet_rs::ui::Painter;
use ddnet_rs::watermark::Look;

const FPS: f64 = 30.0;

fn clock(t: f64) -> Timer {
    Timer::Clock {
        text: str_time((965 + t as i64) * 100, TimeFormat::Days), // 16:05 on
        template: "00:00",
        warn: false,
        blink: false,
    }
}

fn track() -> NowPlaying {
    NowPlaying {
        title: "Night drive".into(),
        artist: "Ambient Tees".into(),
        playing: true,
        cover: None,
    }
}

/// The scripted sequence by time.
fn script(t: f64) -> Option<View> {
    let music = t >= 3.6;
    let idle = |checkpoint, status| {
        Some(View::Idle(Idle {
            compact: false,
            look: Look::default(),
            timer: Some(clock(t)),
            checkpoint,
            status,
            stats: vec![
                (icons::WIFI, "23".into(), Some("ms")),
                (icons::MONITOR, "240".into(), Some("fps")),
            ],
            media: music.then(track),
        }))
    };
    let cp = Effect::Checkpoint {
        text: "-01.20".into(),
        sign: -1,
    };
    match t {
        t if t < 0.4 => None,
        t if t < 1.8 => idle(None, None),
        t if t < 3.2 => idle(Some(cp), None),
        t if t < 3.6 => idle(None, None),
        t if t < 5.6 => Some(View::Track(track())),
        t if t < 6.4 => idle(None, None),
        t if t < 8.4 => Some(View::Mention {
            author: "brainless".into(),
            text: "nice run, tee! meet at the second freeze".into(),
        }),
        t if t < 9.0 => idle(None, None),
        t if t < 11.0 => Some(View::Broadcast {
            text: "Reach the finish line without touching freeze. Checkpoints show your \
                   difference to your best run."
                .into(),
            timer: Some(clock(t)),
        }),
        t if t < 11.6 => idle(None, None),
        t if t < 13.6 => Some(View::Finish(Effect::Finish {
            time: "16:05.45".into(),
            diff: Some(("-42.10".into(), true)),
            record: Record::Personal,
        })),
        t if t < 14.2 => idle(None, None),
        t if t < 15.6 => idle(None, Some((icons::PAUSE, "Game paused".into()))),
        t if t < 16.6 => idle(None, None),
        _ => None,
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let center = (a[1].parse().unwrap(), a[2].parse().unwrap());
    let glass = Glass::from_setting(a[3].parse().unwrap());
    let out = std::path::Path::new(&a[4]);
    std::fs::create_dir_all(out).unwrap();
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
    let frames = (17.2 * FPS) as usize;
    for i in 0..frames {
        let t = i as f64 / FPS;
        let view = MapView {
            center,
            zoom: 1.0,
            width: size.width,
            height: size.height,
            envelope_time: 0,
        };
        let mut batch = Batch::default();
        let [r, g, b] = hsl_packed_to_rgb(128); // C++ cl_background_color
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
        painter.paint(&gfx, &world, size, input, |ctx| {
            egui::Area::new(egui::Id::unique("island"))
                .anchor(egui::Align2::CENTER_TOP, [0.0, 16.0])
                .show(ctx, |ui| anim.show(ui, glass, script(t), t));
        });
        gfx.screenshot(&out.join(format!("{i:04}.png"))).unwrap();
    }
    println!("{frames} frames in {}", out.display());
}
