//! A tee icon as an egui image (killfeed, chat, scoreboard): the whole idle tee (feet,
//! body, eyes looking right) as C++ `CRenderTools::RenderIdleTeeAtCenter` (render.cpp:267)
//! draws it, tinted with the custom colours like C++.

use std::collections::HashMap;

use egui::{ColorImage, TextureHandle};

use crate::assets;
use crate::image::Image;
use crate::render::anim::AnimState;
use crate::render::content::{
    SPRITE_TEE_BODY, SPRITE_TEE_BODY_OUTLINE, SPRITE_TEE_EYE_NORMAL, SPRITE_TEE_FOOT,
    SPRITE_TEE_FOOT_OUTLINE, SPRITES,
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
    pub color_feet: i32,
}

/// The icon, `SIZE` pixels square, showing the middle `VIEW` units around a 64 unit tee's
/// rendered centre (C++ size 64: the visible tee is about 46 units wide).
const SIZE: usize = 64;
const VIEW: f32 = 52.0;

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

/// C++: `GetRenderTeeOffsetToRenderedTee` (render.cpp:202) for a 0.6 skin at size 64
/// (anim scale 1, `cl_fat_skins` 0): the y offset that centres the rendered tee.
fn offset_to_mid(skin: &SkinData, anim: &AnimState) -> f32 {
    let feet_y = anim.front_foot.y.max(anim.back_foot.y);
    let body_h = skin.body.height_normalized() * 64.0;
    let mut min_y = -32.0;
    min_y += anim.body.y;
    min_y += skin.body.offset_y_normalized() * 64.0;
    let (feet_off, feet_h) = (
        skin.feet.offset_y_normalized() * 32.0,
        skin.feet.height_normalized() * 32.0,
    );
    let max_y = (min_y + body_h).max((-16.0 + feet_y) + feet_off + feet_h);
    let full = max_y - min_y;
    -(min_y + full / 2.0)
}

/// One quad of the tee: its image, centre and size in tee units, mirrored or not.
struct Layer {
    image: Image,
    center: (f32, f32),
    size: (f32, f32),
    mirror: bool,
    tint: [f32; 4],
}

/// Composes the idle tee in C++ `RenderTee6` order (render.cpp:494): outlines (back foot,
/// body, front foot), then fillings (back foot, body, eyes, front foot), direction (1, 0).
pub fn compose(skin: &SkinData, look: &TeeLook) -> ColorImage {
    let (source, body_tint, feet_tint) = if look.custom_color {
        (
            &skin.grayscale,
            skin_color(look.color_body),
            skin_color(look.color_feet),
        )
    } else {
        (&skin.image, [1.0; 4], [1.0; 4])
    };
    let part = |sprite: usize| sprite_image(source, None, &SPRITES[sprite]);
    let anim = AnimState::idle();
    let mid = offset_to_mid(skin, &anim);
    let body = (anim.body.x, anim.body.y + mid);
    let foot = |back: bool, outline: bool| {
        let k = if back {
            &anim.back_foot
        } else {
            &anim.front_foot
        };
        let sprite = if outline {
            SPRITE_TEE_FOOT_OUTLINE
        } else {
            SPRITE_TEE_FOOT
        };
        Layer {
            image: part(sprite),
            center: (k.x, k.y + mid),
            size: (64.0, 32.0),
            mirror: false,
            tint: feet_tint,
        }
    };
    let body_layer = |sprite: usize| Layer {
        image: part(sprite),
        center: body,
        size: (64.0, 64.0),
        mirror: false,
        tint: body_tint,
    };
    // eyes: separation (0.075 - 0.010) * 64, offset (0.125, -0.05) * 64, size 0.40 * 64
    let (separation, eye) = (0.065 * 64.0, 0.40 * 64.0);
    let eye_at = |dx: f32, mirror: bool| Layer {
        image: part(SPRITE_TEE_EYE_NORMAL),
        center: (body.0 + dx + 0.125 * 64.0, body.1 - 0.05 * 64.0),
        size: (eye, eye),
        mirror,
        tint: body_tint,
    };
    let layers = [
        foot(true, true),
        body_layer(SPRITE_TEE_BODY_OUTLINE),
        foot(false, true),
        foot(true, false),
        body_layer(SPRITE_TEE_BODY),
        eye_at(-separation, false),
        eye_at(separation, true),
        foot(false, false),
    ];
    let size = SIZE as f32;
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
                    for l in &layers {
                        let u = (x - l.center.0) / l.size.0 + 0.5;
                        let v = (y - l.center.1) / l.size.1 + 0.5;
                        let u = if l.mirror { 1.0 - u } else { u };
                        let s = sample(&l.image, u, v, l.tint);
                        for i in 0..3 {
                            c[i] = s[i] * s[3] + c[i] * (1.0 - s[3]);
                        }
                        c[3] = s[3] + c[3] * (1.0 - s[3]);
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
                        "tee_{}_{}_{}_{}",
                        name, look.custom_color, look.color_body, look.color_feet
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
