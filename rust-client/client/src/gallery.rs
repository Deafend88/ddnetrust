//! Every design-system component on one page, drawn headless (M7.1): for review
//! (`examples/gallery.rs` writes a PNG) and as a regression test (`tests/gfx.rs`).

use egui::Align2;

use crate::gfx::batch::{Batch, QuadItem};
use crate::gfx::{Gfx, Size};
use crate::icons;
use crate::statusbar::WeaponIcons;
use crate::statusview;
use crate::storage::Storage;
use crate::style::{Glass, Weight, color, size, space};
use crate::ui::Painter;
use crate::watermark;
use crate::widgets as w;

pub const SIZE: Size = Size {
    width: 960,
    height: 640,
};

/// A colourful backdrop standing in for the game, so translucent panels show.
fn stripes(gfx: &mut Gfx) {
    let mut b = Batch::default();
    b.clear(0.18, 0.22, 0.30);
    let size = gfx.world_size();
    b.map_screen(0.0, 0.0, size.width as f32, size.height as f32);
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
            h: size.height as f32,
        }]);
    }
    let world = gfx.world_view();
    gfx.render(&mut b, &world, size);
}

/// The page itself, its panels at `glass`.
fn page(ctx: &egui::Context, glass: Glass) {
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
                        "Title 19 semibold",
                        size::TITLE,
                        Weight::SemiBold,
                        color::TEXT,
                    );
                    w::text(
                        ui,
                        "Label 16 medium",
                        size::LABEL,
                        Weight::Medium,
                        color::TEXT,
                    );
                    w::text(
                        ui,
                        "Body 15 regular",
                        size::BODY,
                        Weight::Regular,
                        color::TEXT,
                    );
                    let weak = ui.visuals().weak_text_color();
                    w::text(
                        ui,
                        "Caption 12, secondary",
                        size::CAPTION,
                        Weight::Regular,
                        weak,
                    );
                    ui.add_space(space::S);
                    ui.horizontal(|ui| {
                        w::button(ui, None, "Button");
                        w::accent_button(ui, Some(icons::CONNECT), "Connect");
                    });
                    ui.horizontal(|ui| {
                        w::toggle(ui, &mut true);
                        ui.label("On");
                        w::toggle(ui, &mut false);
                        ui.label("Off");
                    });
                    let mut v = 60.0f32;
                    w::slider(ui, &mut v, 0.0..=100.0);
                    w::segmented(ui, &mut 1, &["Off", "Subtle", "Full"]);
                    ui.add_space(space::S);
                    ui.horizontal_wrapped(|ui| {
                        for glyph in icons::ALL {
                            w::icon(ui, glyph, size::ICON_LARGE, color::TEXT);
                        }
                    });
                });
                ui.vertical(|ui| {
                    watermark::draw(
                        ui,
                        glass,
                        false,
                        &[
                            (icons::USER, "TeDeafe".into(), "", None),
                            (icons::WIFI, "27".into(), "000", Some("ms")),
                            (icons::MONITOR, "372".into(), "0000", Some("fps")),
                        ],
                        watermark::Look::default(),
                    );
                    ui.add_space(space::M);
                    // the island's look (7.9): Liquid Glass at full strength
                    w::glass_pill(ui, glass, true, |ui| {
                        w::icon(ui, icons::CHECK_CIRCLE, size::ICON_LARGE, color::SUCCESS);
                        w::text(
                            ui,
                            "Finish  16:05.45",
                            size::LABEL,
                            Weight::SemiBold,
                            color::TEXT,
                        );
                        w::text(ui, "-0:42.10", size::BODY, Weight::Medium, color::SUCCESS);
                    });
                    // last in the column: the open popover has nothing below it
                    ui.add_space(space::M);
                    let settings = w::button(ui, Some(icons::GEAR), "Widget settings");
                    w::popover(&settings, true, glass, None, |ui| {
                        ui.horizontal(|ui| {
                            w::toggle(ui, &mut true);
                            ui.label("Compact");
                        });
                    });
                });
            });
        });
}

/// Draws the gallery (glass subtle, over coloured stripes) into the world target of
/// `gfx` (sized `SIZE`).
pub fn render(gfx: &mut Gfx) {
    render_with(gfx, Glass::Subtle, stripes);
}

/// Draws the gallery at `glass` over what `backdrop` draws into the world target. egui
/// needs a few passes to settle its layout and finish fade-ins; each pass redraws the
/// backdrop first.
pub fn render_with(gfx: &mut Gfx, glass: Glass, backdrop: impl FnMut(&mut Gfx)) {
    render_page(gfx, glass, backdrop, |ctx| page(ctx, glass));
}

/// Draws `page` like the gallery: a few passes over `backdrop`, the clock moving on.
fn render_page(
    gfx: &mut Gfx,
    glass: Glass,
    mut backdrop: impl FnMut(&mut Gfx),
    mut page: impl FnMut(&egui::Context),
) {
    let size = gfx.world_size();
    let mut painter = Painter::new(gfx, gfx.format, glass);
    // the clock moves on between passes so fade-ins finish (deterministically)
    for pass in 0..3 {
        backdrop(gfx);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(size.width as f32, size.height as f32),
            )),
            time: Some(pass as f64 * 10.0),
            ..Default::default()
        };
        let world = gfx.world_view();
        painter.paint(gfx, &world, size, input, &mut page);
    }
}

/// The race status review page (7.7): every state, labelled.
pub const STATUS_SIZE: Size = Size {
    width: 1280,
    height: 720,
};

/// Draws the race status page into the world target of `gfx` (sized `STATUS_SIZE`).
pub fn render_status(gfx: &mut Gfx, storage: &Storage) {
    let glass = Glass::Subtle;
    let mut icons = WeaponIcons::new(storage);
    let examples = statusview::examples();
    render_page(gfx, glass, stripes, |ctx| {
        egui::Area::new(egui::Id::unique("status_gallery"))
            .anchor(Align2::LEFT_TOP, [space::XL, space::XL])
            .show(ctx, |ui| {
                w::text(
                    ui,
                    "Race status",
                    size::TITLE,
                    Weight::SemiBold,
                    color::TEXT,
                );
                egui::Grid::new("status_states")
                    .spacing([space::XXL, space::M])
                    .show(ui, |ui| {
                        for (n, (label, status, freeze)) in examples.iter().enumerate() {
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = space::XS;
                                let weak = ui.visuals().weak_text_color();
                                w::text(ui, label, size::CAPTION, Weight::Regular, weak);
                                statusview::draw(ui, glass, status, *freeze, &mut icons, false);
                            });
                            if n % 2 == 1 {
                                ui.end_row();
                            }
                        }
                    });
            });
    });
}
