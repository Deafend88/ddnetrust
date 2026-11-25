//! Review page: our Phosphor Fill icons next to Material Symbols Rounded (filled) for the
//! same meanings, to choose an icon set. Material is not bundled: pass a static filled
//! instance (FILL 1, wght 400, GRAD 0, opsz 24, made with fontTools' instancer from
//! Google's `MaterialSymbolsRounded[FILL,GRAD,opsz,wght].ttf`) and its `.codepoints`.
//!
//! Usage: cargo run -p ddnet-rs --example icon_compare -- <material.ttf> <codepoints> <out.png>

use std::collections::HashMap;
use std::sync::Arc;

use ddnet_rs::gfx::{Gfx, Size};
use ddnet_rs::icons;
use ddnet_rs::style::{self, Glass, Weight, color, icon_font, size, space};
use ddnet_rs::ui::Painter;
use ddnet_rs::widgets as w;
use egui::{FontFamily, FontId, RichText};

/// (meaning, Phosphor glyph, Material name): the player controls first, then the HUD.
const ICONS: [(&str, &str, &str); 34] = [
    ("play", icons::PLAY, "play_arrow"),
    ("pause", icons::PAUSE, "pause"),
    ("previous", icons::SKIP_BACK, "skip_previous"),
    ("next", icons::SKIP_FORWARD, "skip_next"),
    ("now playing", icons::MUSIC_NOTES, "music_note"),
    ("more", icons::DOTS_THREE, "more_horiz"),
    ("name", icons::USER, "person"),
    ("client id", icons::FINGERPRINT, "fingerprint"),
    ("framerate", icons::MONITOR, "desktop_windows"),
    ("ping", icons::WIFI, "wifi"),
    ("server", icons::GLOBE, "public"),
    ("local time", icons::CLOCK, "schedule"),
    ("speed", icons::GAUGE, "speed"),
    ("position", icons::MAP_PIN, "location_on"),
    ("aim", icons::CROSSHAIR, "my_location"),
    ("race time", icons::TIMER, "timer"),
    ("tick rate", icons::PULSE, "monitor_heart"),
    ("finish", icons::CHECK_CIRCLE, "check_circle"),
    ("record", icons::TROPHY, "trophy"),
    ("finish line", icons::FLAG_CHECKERED, "sports_score"),
    ("kill", icons::SKULL, "skull"),
    ("broadcast", icons::MEGAPHONE, "campaign"),
    ("pause/show", icons::EYE, "visibility"),
    ("hide", icons::EYE_SLASH, "visibility_off"),
    ("settings", icons::GEAR, "settings"),
    ("close", icons::X, "close"),
    ("deep freeze", icons::SNOWFLAKE, "ac_unit"),
    ("jetpack", icons::FLAME, "local_fire_department"),
    (
        "endless jump",
        icons::ARROW_FAT_LINES_UP,
        "keyboard_double_arrow_up",
    ),
    ("teleport", icons::SPARKLE, "auto_awesome"),
    ("solo", icons::PERSON, "accessibility_new"),
    ("no collision", icons::GHOST, "blur_on"),
    ("lock mode", icons::LOCK, "lock"),
    ("practice", icons::BARBELL, "fitness_center"),
];

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let ttf = std::fs::read(&a[0]).expect("material font");
    let points: HashMap<String, char> = std::fs::read_to_string(&a[1])
        .expect("codepoints")
        .lines()
        .filter_map(|l| {
            let (name, hex) = l.split_once(' ')?;
            let c = char::from_u32(u32::from_str_radix(hex.trim(), 16).ok()?)?;
            Some((name.to_string(), c))
        })
        .collect();
    let size_px = Size {
        width: 980,
        height: 480,
    };
    let gfx = Gfx::headless(size_px).expect("graphics adapter");
    let mut painter = Painter::new(&gfx, gfx.format, Glass::Off);
    let mut fonts = style::fonts();
    fonts
        .font_data
        .insert("material".into(), Arc::new(egui::FontData::from_owned(ttf)));
    fonts
        .families
        .insert(FontFamily::Name("material".into()), vec!["material".into()]);
    painter.ctx.set_fonts(fonts);
    let material = |s: f32| FontId::new(s, FontFamily::Name("material".into()));
    for pass in 0..3 {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(size_px.width as f32, size_px.height as f32),
            )),
            time: Some(pass as f64 * 10.0),
            ..Default::default()
        };
        let world = gfx.world_view();
        painter.paint(&gfx, &world, size_px, input, |ctx| {
            egui::Area::new(egui::Id::unique("page")).show(ctx, |ui| {
                let frame = egui::Frame::new()
                    .fill(color::SURFACE)
                    .inner_margin(space::XL);
                frame.show(ui, |ui| {
                    w::text(
                        ui,
                        "Phosphor Fill (left) vs Material Symbols Rounded, filled (right)",
                        size::LABEL,
                        Weight::SemiBold,
                        color::TEXT,
                    );
                    ui.add_space(space::M);
                    egui::Grid::new("icons")
                        .num_columns(8)
                        .spacing([space::L, space::S])
                        .show(ui, |ui| {
                            for (i, (meaning, ph, mat)) in ICONS.iter().enumerate() {
                                let weak = color::TEXT_SECONDARY;
                                w::text(ui, meaning, size::CAPTION, Weight::Regular, weak);
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = space::M;
                                    for s in [size::ICON, size::ICON_HUD] {
                                        ui.label(
                                            RichText::new(*ph)
                                                .font(icon_font(s))
                                                .color(color::TEXT),
                                        );
                                    }
                                    ui.add_space(space::S);
                                    let m = points
                                        .get(*mat)
                                        .map(|c| c.to_string())
                                        .unwrap_or("?".into());
                                    for s in [size::ICON, size::ICON_HUD] {
                                        ui.label(
                                            RichText::new(&m)
                                                .font(material(s * 1.15))
                                                .color(color::TEXT),
                                        );
                                    }
                                });
                                if i % 4 == 3 {
                                    ui.end_row();
                                }
                            }
                        });
                });
            });
        });
    }
    gfx.screenshot(std::path::Path::new(&a[2])).unwrap();
    println!("saved {}", a[2]);
}
