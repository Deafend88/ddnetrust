//! M7.2 review: renders the gallery at each glass strength over a real map, and times the
//! glass passes (blur + four panels) at 2560x1440.
//!
//! Usage: cargo run -p ddnet-rs --example glass -- <map file> <center x> <center y> <out dir>

use std::time::Instant;

use ddnet::map::Map;
use ddnet_rs::assets;
use ddnet_rs::gfx::batch::Batch;
use ddnet_rs::gfx::{Gfx, Size};
use ddnet_rs::glass;
use ddnet_rs::render::map::{MapRender, View, hsl_packed_to_rgb};
use ddnet_rs::storage::Storage;
use ddnet_rs::style::Glass;
use ddnet_rs::ui::Painter;
use egui::{Rect, pos2, vec2};

const STRENGTHS: [(Glass, &str); 3] = [
    (Glass::Off, "off"),
    (Glass::Subtle, "subtle"),
    (Glass::Full, "full"),
];

fn draw_map(gfx: &mut Gfx, render: &MapRender, center: (f32, f32)) {
    let size = gfx.world_size();
    let view = View {
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
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let center = (a[1].parse().unwrap(), a[2].parse().unwrap());
    let out = std::path::Path::new(&a[3]);
    let storage = Storage::new(None).unwrap();
    let map = Map::load(std::fs::read(&a[0]).unwrap()).unwrap();
    let images = assets::load_map_images(&storage, &map);

    for (glass, name) in STRENGTHS {
        let mut gfx = Gfx::headless(Size {
            width: 1280,
            height: 720,
        })
        .expect("graphics adapter");
        let render = MapRender::new(&mut gfx, &map, &images);
        ddnet_rs::gallery::render_with(&mut gfx, glass, |g| draw_map(g, &render, center));
        let path = out.join(format!("glass-{name}.png"));
        gfx.screenshot(&path).unwrap();
        println!("saved {}", path.display());
    }

    // time: the map is drawn once; each frame is one egui pass with four glass panels,
    // waited for on the GPU; "off" (callbacks that draw nothing) is the baseline
    let size = Size {
        width: 2560,
        height: 1440,
    };
    let mut gfx = Gfx::headless(size).expect("graphics adapter");
    let render = MapRender::new(&mut gfx, &map, &images);
    draw_map(&mut gfx, &render, center);
    let panels = [
        (
            Rect::from_min_size(pos2(32.0, 32.0), vec2(560.0, 64.0)),
            255.0,
            false,
        ),
        (
            Rect::from_min_size(pos2(1000.0, 32.0), vec2(560.0, 96.0)),
            255.0,
            true,
        ),
        (
            Rect::from_min_size(pos2(64.0, 300.0), vec2(800.0, 900.0)),
            24.0,
            false,
        ),
        (
            Rect::from_min_size(pos2(1800.0, 900.0), vec2(500.0, 300.0)),
            24.0,
            false,
        ),
    ];
    let mut base = 0.0;
    for (glass, name) in STRENGTHS {
        let mut painter = Painter::new(&gfx, gfx.format, glass);
        // a separate target, so the world (the blur's source) stays the map
        let target = gfx.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: gfx.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let mut frame = |t: f64| {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    pos2(0.0, 0.0),
                    vec2(size.width as f32, size.height as f32),
                )),
                time: Some(t),
                ..Default::default()
            };
            painter.paint(&gfx, &view, size, input, |ctx| {
                let p = ctx.layer_painter(egui::LayerId::background());
                for (rect, radius, liquid) in panels {
                    p.add(glass::shape(rect, radius, liquid));
                }
            });
            gfx.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
        };
        for i in 0..20 {
            frame(i as f64);
        }
        let n = 300;
        let start = Instant::now();
        for i in 0..n {
            frame(100.0 + i as f64);
        }
        let ms = start.elapsed().as_secs_f64() * 1000.0 / n as f64;
        if glass == Glass::Off {
            base = ms;
        }
        println!(
            "2560x1440 {name}: {ms:.3} ms per frame, glass {:.3} ms",
            ms - base
        );
    }
}
