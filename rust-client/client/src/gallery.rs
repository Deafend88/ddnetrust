//! Every design-system component on one page, drawn headless (M7.1): for review
//! (`examples/gallery.rs` writes a PNG) and as a regression test (`tests/gfx.rs`).

use egui::{Align2, Color32, RichText};

use crate::gfx::batch::{Batch, QuadItem};
use crate::gfx::{Gfx, Size};
use crate::icons;
use crate::style::{Glass, Weight, color, size, space};
use crate::ui::Painter;
use crate::widgets as w;

pub const SIZE: Size = Size {
    width: 960,
    height: 640,
};

/// A colourful backdrop standing in for the game, so translucent panels show.
fn backdrop(gfx: &mut Gfx) {
    let mut b = Batch::default();
    b.clear(0.18, 0.22, 0.30);
    b.map_screen(0.0, 0.0, SIZE.width as f32, SIZE.height as f32);
    b.texture_set(None);
    b.quads_begin();
    for (i, c) in [
        [0.95, 0.55, 0.20, 1.0],
        [0.25, 0.70, 0.45, 1.0],
        [0.55, 0.35, 0.85, 1.0],
        [0.90, 0.85, 0.30, 1.0],
    ]
    .into_iter()
    .enumerate()
    {
        b.set_color(c);
        let x = 120.0 + i as f32 * 220.0;
        b.quads_draw_tl(&[QuadItem {
            x,
            y: 0.0,
            w: 90.0,
            h: SIZE.height as f32,
        }]);
    }
    let world = gfx.world_view();
    gfx.render(&mut b, &world, SIZE);
}

/// The page itself.
fn page(ctx: &egui::Context) {
    let glass = Glass::Subtle;
    egui::Area::new(egui::Id::unique("gallery"))
        .anchor(Align2::LEFT_TOP, [space::XL, space::XL])
        .show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                w::panel(ui, glass, |ui| {
                    ui.set_width(380.0);
                    w::text(
                        ui,
                        "Design system",
                        size::LARGE,
                        Weight::SemiBold,
                        color::TEXT,
                    );
                    w::text(
                        ui,
                        "Title 17 semibold",
                        size::TITLE,
                        Weight::SemiBold,
                        color::TEXT,
                    );
                    w::text(
                        ui,
                        "Label 14 medium",
                        size::LABEL,
                        Weight::Medium,
                        color::TEXT,
                    );
                    w::text(
                        ui,
                        "Body 13 regular",
                        size::BODY,
                        Weight::Regular,
                        color::TEXT,
                    );
                    w::text(
                        ui,
                        "Caption 11, secondary",
                        size::CAPTION,
                        Weight::Regular,
                        color::TEXT_SECONDARY,
                    );
                    ui.separator();
                    ui.horizontal(|ui| {
                        w::button(ui, None, "Button");
                        w::accent_button(ui, Some(icons::PLAY), "Connect");
                        let settings = w::button(ui, Some(icons::GEAR), "Widget settings");
                        w::popover(&settings, true, glass, |ui| {
                            ui.horizontal(|ui| {
                                w::toggle(ui, &mut true);
                                ui.label("Compact");
                            });
                        });
                    });
                    ui.add_space(space::XL + space::S);
                    ui.horizontal(|ui| {
                        w::toggle(ui, &mut true);
                        ui.label("On");
                        w::toggle(ui, &mut false);
                        ui.label("Off");
                    });
                    let mut v = 60.0f32;
                    w::slider(ui, &mut v, 0.0..=100.0);
                    w::segmented(ui, &mut 1, &["Off", "Subtle", "Full"]);
                    ui.separator();
                    ui.horizontal_wrapped(|ui| {
                        for glyph in icons::ALL {
                            w::icon(ui, glyph, size::ICON + 4.0, color::TEXT);
                        }
                    });
                });
                ui.vertical(|ui| {
                    // the watermark's look (7.5 builds the real widget)
                    w::pill(ui, Glass::Subtle.panel_fill(), |ui| {
                        w::pill(ui, color::LOGO, |ui| {
                            w::icon(ui, icons::BOLT, size::ICON - 2.0, Color32::from_gray(40));
                            w::text(
                                ui,
                                "DDNet-rs",
                                size::CAPTION,
                                Weight::Medium,
                                Color32::from_gray(40),
                            );
                        });
                        w::stat(ui, icons::USER, "TeDeafe", None);
                        w::dot(ui);
                        w::stat(ui, icons::CLOUD, "27", Some("ms"));
                        w::dot(ui);
                        w::stat(ui, icons::BOLT, "372", Some("fps"));
                    });
                    ui.add_space(space::M);
                    ui.horizontal(|ui| {
                        for (g, name) in [
                            (Glass::Off, "Off"),
                            (Glass::Subtle, "Subtle"),
                            (Glass::Full, "Full"),
                        ] {
                            w::panel(ui, g, |ui| {
                                ui.set_width(100.0);
                                w::text(ui, name, size::LABEL, Weight::Medium, color::TEXT);
                                ui.label(RichText::new("glass").color(color::TEXT_SECONDARY));
                            });
                        }
                    });
                    ui.add_space(space::M);
                    w::pill(ui, Glass::Subtle.panel_fill(), |ui| {
                        w::icon(ui, icons::CHECK_CIRCLE, size::TITLE, color::SUCCESS);
                        w::text(
                            ui,
                            "Finish  16:05.45",
                            size::LABEL,
                            Weight::SemiBold,
                            color::TEXT,
                        );
                        w::text(ui, "-0:42.10", size::BODY, Weight::Medium, color::SUCCESS);
                    });
                });
            });
        });
}

/// Draws the gallery into the world target of `gfx` (sized `SIZE`). egui needs a few
/// passes to settle its layout and finish fade-ins; each pass redraws the backdrop first.
pub fn render(gfx: &mut Gfx) {
    let mut painter = Painter::new(gfx, gfx.format, Glass::Subtle);
    // the clock moves on between passes so fade-ins finish (deterministically)
    for pass in 0..3 {
        backdrop(gfx);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(SIZE.width as f32, SIZE.height as f32),
            )),
            time: Some(pass as f64 * 10.0),
            ..Default::default()
        };
        let world = gfx.world_view();
        painter.paint(gfx, &world, SIZE, input, page);
    }
}
