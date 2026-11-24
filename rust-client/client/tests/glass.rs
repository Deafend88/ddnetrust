//! M7.2: glass on the GPU over synthetic backdrops (PLAN §10.3 7.2): off is the exact
//! opaque colour, the blur keeps flat colours and stays inside the rounded-rect mask, the
//! mask's area matches the analytic one, Liquid Glass refracts inward near the edge and
//! darkens more over bright backdrops.

use ddnet_rs::gfx::batch::{Batch, Blend, QuadItem};
use ddnet_rs::gfx::{Gfx, Size};
use ddnet_rs::glass;
use ddnet_rs::style::{Glass, color};
use ddnet_rs::ui::Painter;
use ddnet_rs::widgets;
use egui::{Rect, pos2, vec2};

const SIZE: Size = Size {
    width: 400,
    height: 240,
};

/// Draws `backdrop`, then whatever `ui` paints with egui at `glass`; returns RGBA rows.
fn render(
    glass: Glass,
    backdrop: impl Fn(&mut Batch),
    ui: impl Fn(&egui::Context),
) -> Option<Vec<u8>> {
    let Some(mut g) = Gfx::headless(SIZE) else {
        eprintln!("SKIPPED: no graphics adapter");
        return None;
    };
    let mut painter = Painter::new(&g, g.format, glass);
    let world = g.world_view();
    // egui areas fade in: three passes with the clock moving, the backdrop redrawn first
    for pass in 0..3 {
        let mut b = Batch::default();
        b.map_screen(0.0, 0.0, SIZE.width as f32, SIZE.height as f32);
        b.texture_set(None);
        b.blend(Blend::None);
        backdrop(&mut b);
        g.render(&mut b, &world, SIZE);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                pos2(0.0, 0.0),
                vec2(SIZE.width as f32, SIZE.height as f32),
            )),
            time: Some(pass as f64 * 10.0),
            ..Default::default()
        };
        painter.paint(&g, &world, SIZE, input, &ui);
    }
    Some(g.read_world())
}

fn quad(b: &mut Batch, x: f32, w: f32, c: [f32; 4]) {
    b.quads_begin();
    b.set_color(c);
    b.quads_draw_tl(&[QuadItem {
        x,
        y: 0.0,
        w,
        h: SIZE.height as f32,
    }]);
}

/// Just a glass backdrop (no fill, no border) at `rect`.
fn backdrop_only(rect: Rect, radius: f32, liquid: bool) -> impl Fn(&egui::Context) {
    move |ctx| {
        ctx.layer_painter(egui::LayerId::background())
            .add(glass::shape(rect, radius, liquid));
    }
}

fn px(img: &[u8], x: u32, y: u32) -> [u8; 4] {
    let o = ((y * SIZE.width + x) * 4) as usize;
    [img[o], img[o + 1], img[o + 2], img[o + 3]]
}

#[test]
fn off_panel_is_the_exact_opaque_colour() {
    let Some(img) = render(
        Glass::Off,
        |b| quad(b, 0.0, 400.0, [0.9, 0.5, 0.1, 1.0]),
        |ctx| {
            egui::Area::new(egui::Id::unique("p"))
                .fixed_pos(pos2(100.0, 60.0))
                .show(ctx, |ui| {
                    widgets::panel(ui, Glass::Off, |ui| ui.set_min_size(vec2(120.0, 80.0)));
                });
        },
    ) else {
        return;
    };
    let s = color::SURFACE;
    assert_eq!(px(&img, 160, 100)[..3], [s.r(), s.g(), s.b()]);
}

/// Outside the mask nothing changes (left half red, right half blue); inside, a flat
/// area keeps its colour (a grey: full glass's saturation leaves greys alone, and its
/// blur reaches across this whole target, so the flat area is all of it).
#[test]
fn blur_keeps_flat_colour_and_stays_inside_the_mask() {
    let rect = Rect::from_min_max(pos2(40.0, 40.0), pos2(150.0, 200.0));
    let halves = |b: &mut Batch| {
        quad(
            b,
            0.0,
            200.0,
            [200.0 / 255.0, 40.0 / 255.0, 40.0 / 255.0, 1.0],
        );
        quad(
            b,
            200.0,
            200.0,
            [30.0 / 255.0, 90.0 / 255.0, 200.0 / 255.0, 1.0],
        );
    };
    let grey = |b: &mut Batch| {
        quad(
            b,
            0.0,
            400.0,
            [150.0 / 255.0, 150.0 / 255.0, 150.0 / 255.0, 1.0],
        )
    };
    let Some(plain) = render(Glass::Off, halves, |_| {}) else {
        return;
    };
    for glass in [Glass::Off, Glass::Subtle, Glass::Full] {
        let img = render(glass, halves, backdrop_only(rect, 24.0, false)).unwrap();
        let flat = render(glass, grey, backdrop_only(rect, 24.0, false)).unwrap();
        for y in 0..SIZE.height {
            for x in 0..SIZE.width {
                let p = pos2(x as f32 + 0.5, y as f32 + 0.5);
                let (got, want) = (px(&img, x, y), px(&plain, x, y));
                if !rect.expand(1.0).contains(p) {
                    assert_eq!(got[..3], want[..3], "{glass:?} outside at {x},{y}");
                } else if rect.shrink(24.0).contains(p) {
                    // the straight inner part, away from the corners' anti-aliasing
                    let got = px(&flat, x, y);
                    let d = got[..3].iter().map(|v| v.abs_diff(150)).max();
                    assert!(d <= Some(1), "{glass:?} inside at {x},{y}: {got:?}");
                }
            }
        }
    }
}

/// Over a target with alpha 0 the backdrop's alpha is its coverage: the sum over the
/// panel equals the area of the rounded rect, w*h - (4 - pi) r^2.
#[test]
fn mask_area_matches_the_rounded_rect() {
    let (w, h, r) = (200.0f64, 120.0, 40.0);
    let rect = Rect::from_min_size(pos2(60.0, 50.0), vec2(w as f32, h as f32));
    let Some(img) = render(
        Glass::Subtle,
        |b| b.clear(0.0, 0.0, 0.0),
        backdrop_only(rect, r as f32, false),
    ) else {
        return;
    };
    let area: f64 = img.chunks(4).map(|p| p[3] as f64 / 255.0).sum();
    let want = w * h - (4.0 - std::f64::consts::PI) * r * r;
    assert!((area - want).abs() < 2.0, "area {area}, analytic {want}");
}

/// A horizontal gradient; Liquid Glass over it is the blurred gradient times one tint
/// factor, except near the edge, where samples come from further inside: on the right
/// edge (no rim light there) that is further left, so darker.
#[test]
fn liquid_glass_refracts_inward_near_the_edge() {
    let rect = Rect::from_min_size(pos2(60.0, 80.0), vec2(280.0, 80.0));
    let gradient = |b: &mut Batch| {
        b.quads_begin();
        let (l, r) = ([0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]);
        b.set_color4(l, r, l, r);
        b.quads_draw_tl(&[QuadItem {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 240.0,
        }]);
    };
    let Some(flat) = render(Glass::Full, gradient, backdrop_only(rect, 40.0, false)) else {
        return;
    };
    let liquid = render(Glass::Full, gradient, backdrop_only(rect, 40.0, true)).unwrap();
    let red = |img: &[u8], x| px(img, x, 120)[0] as f32;
    let k = red(&liquid, 200) / red(&flat, 200);
    assert!(k < 1.0, "tint factor {k}");
    // well inside: only the tint
    assert!((red(&liquid, 250) / k - red(&flat, 250)).abs() <= 1.5);
    // 3 px inside the right edge: sampled further left
    let edge = 336;
    let (got, unrefracted) = (red(&liquid, edge) / k, red(&flat, edge));
    assert!(got < unrefracted - 2.0, "{got} vs {unrefracted}");
}

/// The tint darkens more over a bright backdrop than over a dark one.
#[test]
fn liquid_tint_follows_the_backdrop() {
    let rect = Rect::from_min_size(pos2(60.0, 80.0), vec2(280.0, 80.0));
    let factor = |v: f32| {
        let img = render(
            Glass::Full,
            |b| quad(b, 0.0, 400.0, [v, v, v, 1.0]),
            backdrop_only(rect, 40.0, true),
        )?;
        Some(px(&img, 200, 120)[0] as f32 / (v * 255.0).round())
    };
    let (Some(dark), Some(bright)) = (factor(30.0 / 255.0), factor(220.0 / 255.0)) else {
        return;
    };
    assert!(bright < dark && dark < 1.0, "dark {dark}, bright {bright}");
}

/// Full glass is a little more saturated than its backdrop; subtle keeps the colour.
#[test]
fn full_glass_saturates() {
    let rect = Rect::from_min_size(pos2(60.0, 60.0), vec2(280.0, 120.0));
    let chroma = |glass| {
        let img = render(
            glass,
            |b| {
                quad(
                    b,
                    0.0,
                    400.0,
                    [200.0 / 255.0, 80.0 / 255.0, 60.0 / 255.0, 1.0],
                )
            },
            backdrop_only(rect, 24.0, false),
        )?;
        let p = px(&img, 200, 120);
        Some(p[..3].iter().max().unwrap() - p[..3].iter().min().unwrap())
    };
    let (Some(subtle), Some(full)) = (chroma(Glass::Subtle), chroma(Glass::Full)) else {
        return;
    };
    assert_eq!(subtle, 140);
    assert!(full > subtle + 20, "full {full}, subtle {subtle}");
}
