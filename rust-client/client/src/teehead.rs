//! A tee's head as an egui image (the killfeed's tees): the skin's body outline, body and
//! two eyes looking right, placed like C++ `CRenderTools::RenderTee` (render.cpp) draws
//! an idle tee, tinted with the custom body colour like C++.

use std::collections::HashMap;

use egui::{ColorImage, TextureHandle};

use crate::assets;
use crate::image::Image;
use crate::render::content::{
    SPRITE_TEE_BODY, SPRITE_TEE_BODY_OUTLINE, SPRITE_TEE_EYE_NORMAL, SPRITES,
};
use crate::render::tee::skin_color;
use crate::skin::{SkinData, load_skin_data, sprite_image};
use crate::storage::Storage;

/// A client's tee look, as its `ClientInfo` gives it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TeeLook {
    pub skin: String,
    pub custom_color: bool,
    pub color_body: i32,
}

/// The head image, `SIZE` pixels square, showing the middle `VIEW` units of a 64 unit
/// tee (body quad 64, eyes 25.6): the body fills about two thirds of its quad.
const SIZE: usize = 64;
const VIEW: f32 = 48.0;

/// Samples `img` at (`u`, `v`) in 0..1 (nearest), tinted.
fn sample(img: &Image, u: f32, v: f32, tint: [f32; 4]) -> [f32; 4] {
    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
        return [0.0; 4];
    }
    let x = (u * img.width as f32) as usize;
    let y = (v * img.height as f32) as usize;
    let p = &img.data[(y * img.width as usize + x) * 4..][..4];
    let f = |i: usize| p[i] as f32 / 255.0;
    [
        f(0) * tint[0],
        f(1) * tint[1],
        f(2) * tint[2],
        f(3) * tint[3],
    ]
}

/// Composes the head: outline, body, then the eyes (the right one mirrored), with the
/// direction (1, 0): separation 0.065, offset (0.125, -0.05) of the size (C++ RenderTee).
pub fn compose(skin: &SkinData, look: &TeeLook) -> ColorImage {
    let (source, tint) = if look.custom_color {
        (&skin.grayscale, skin_color(look.color_body))
    } else {
        (&skin.image, [1.0; 4])
    };
    let part = |sprite: usize| sprite_image(source, None, &SPRITES[sprite]);
    let (outline, body, eye) = (
        part(SPRITE_TEE_BODY_OUTLINE),
        part(SPRITE_TEE_BODY),
        part(SPRITE_TEE_EYE_NORMAL),
    );
    let size = SIZE as f32;
    let (eye_size, separation) = (size * 0.40, size * 0.065);
    let (eye_x, eye_y) = (size * 0.125, size * -0.05);
    let mut pixels = Vec::with_capacity(SIZE * SIZE);
    // 2x2 samples per pixel
    let offsets = [0.25, 0.75];
    for py in 0..SIZE {
        for px in 0..SIZE {
            let mut sum = [0.0f32; 4];
            for oy in offsets {
                for ox in offsets {
                    // tee units, centre 0
                    let to_units = |p: f32| p / size * VIEW - VIEW / 2.0;
                    let (x, y) = (to_units(px as f32 + ox), to_units(py as f32 + oy));
                    let mut c = [0.0f32; 4];
                    let mut over = |s: [f32; 4]| {
                        for i in 0..3 {
                            c[i] = s[i] * s[3] + c[i] * (1.0 - s[3]);
                        }
                        c[3] = s[3] + c[3] * (1.0 - s[3]);
                    };
                    let (u, v) = (x / size + 0.5, y / size + 0.5);
                    over(sample(&outline, u, v, tint));
                    over(sample(&body, u, v, tint));
                    for (dx, mirror) in [(-separation, false), (separation, true)] {
                        let eu = (x - (dx + eye_x)) / eye_size + 0.5;
                        let ev = (y - eye_y) / eye_size + 0.5;
                        let eu = if mirror { 1.0 - eu } else { eu };
                        over(sample(&eye, eu, ev, tint));
                    }
                    for i in 0..4 {
                        sum[i] += c[i] / 4.0;
                    }
                }
            }
            // egui wants premultiplied colour
            let b = |f: f32| (f.clamp(0.0, 1.0) * 255.0).round() as u8;
            pixels.push(egui::Color32::from_rgba_premultiplied(
                b(sum[0]),
                b(sum[1]),
                b(sum[2]),
                b(sum[3]),
            ));
        }
    }
    ColorImage::new([SIZE, SIZE], pixels)
}

/// Loaded skins and head textures, by skin name and look.
pub struct TeeHeads {
    storage: Storage,
    skins: HashMap<String, Option<SkinData>>,
    heads: HashMap<TeeLook, Option<TextureHandle>>,
}

impl TeeHeads {
    pub fn new(storage: &Storage) -> Self {
        Self {
            storage: storage.clone(),
            skins: HashMap::new(),
            heads: HashMap::new(),
        }
    }

    /// The head of `look` (skins C++ would not load fall back to "default").
    pub fn get(&mut self, ctx: &egui::Context, look: &TeeLook) -> Option<&TextureHandle> {
        if !self.heads.contains_key(look) {
            let storage = &self.storage;
            let mut load = |name: &str| {
                self.skins
                    .entry(name.to_string())
                    .or_insert_with(|| assets::load_skin(storage, name).and_then(load_skin_data))
                    .is_some()
            };
            let name = if load(&look.skin) {
                look.skin.as_str()
            } else {
                "default"
            };
            load(name);
            let texture = self.skins[name].as_ref().map(|skin| {
                ctx.load_texture(
                    format!(
                        "tee_head_{}_{}_{}",
                        name, look.custom_color, look.color_body
                    ),
                    compose(skin, look),
                    egui::TextureOptions::LINEAR,
                )
            });
            self.heads.insert(look.clone(), texture);
        }
        self.heads[look].as_ref()
    }
}
