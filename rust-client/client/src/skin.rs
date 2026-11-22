//! Skin images as the C++ client prepares them: `CSkins::LoadSkinData`
//! (`components/skins.cpp:289`) and the sprite cutting of `LoadSpriteTexture`
//! (graphics_threaded.cpp:321).

use crate::image::{Format, Image};
use crate::render::content::{
    SPRITE_TEE_BODY, SPRITE_TEE_BODY_OUTLINE, SPRITE_TEE_FOOT, SPRITE_TEE_FOOT_OUTLINE, SPRITES,
    SPRITESETS, Sprite,
};

/// C++: `CSkin::CSkinMetricVariable`. Sizes only grow and offsets only shrink when
/// assigned (`CSkinMetricVariableSize`/`CSkinMetricVariableInt`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Metric {
    pub width: i32,
    pub height: i32,
    pub offset_x: i32,
    pub offset_y: i32,
    pub max_width: i32,
    pub max_height: i32,
}

impl Default for Metric {
    fn default() -> Self {
        Self {
            width: i32::MIN,
            height: i32::MIN,
            offset_x: i32::MAX,
            offset_y: i32::MAX,
            max_width: i32::MIN,
            max_height: i32::MIN,
        }
    }
}

impl Metric {
    pub fn width_normalized(&self) -> f32 {
        self.width as f32 / self.max_width as f32
    }
    pub fn height_normalized(&self) -> f32 {
        self.height as f32 / self.max_height as f32
    }
    pub fn offset_x_normalized(&self) -> f32 {
        self.offset_x as f32 / self.max_width as f32
    }
    pub fn offset_y_normalized(&self) -> f32 {
        self.offset_y as f32 / self.max_height as f32
    }

    /// C++: `CheckMetrics` (skins.cpp:254) on an RGBA image.
    fn check(&mut self, img: &Image, x0: i32, y0: i32, w: i32, h: i32) {
        let (mut max_y, mut min_y, mut max_x, mut min_x) = (-1, h + 1, -1, w + 1);
        for y in 0..h {
            for x in 0..w {
                let i = ((y + y0) as usize * img.width as usize + (x + x0) as usize) * 4 + 3;
                if img.data[i] > 0 {
                    max_y = max_y.max(y);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    min_x = min_x.min(x);
                }
            }
        }
        self.width = self.width.max((max_x - min_x + 1).clamp(1, w));
        self.height = self.height.max((max_y - min_y + 1).clamp(1, h));
        self.offset_x = self.offset_x.min(min_x.clamp(0, w - 1));
        self.offset_y = self.offset_y.min(min_y.clamp(0, h - 1));
        self.max_width = self.max_width.max(w);
        self.max_height = self.max_height.max(h);
    }
}

/// C++: `CSkins::CSkinLoadData` after `LoadSkinData`.
pub struct SkinData {
    /// The skin, resized like C++ when its size is not divisible by the tee grid.
    pub image: Image,
    /// The colourable variant: grayscale with the body's most common shade moved to 192.
    pub grayscale: Image,
    pub blood: [f32; 3],
    pub body: Metric,
    pub feet: Metric,
}

/// A sprite's rectangle in an image of this size (C++ `LoadSpriteTexture`).
pub fn sprite_rect(width: u32, height: u32, s: &Sprite) -> [u32; 4] {
    let set = &SPRITESETS[s.set];
    let (gx, gy) = (width / set.gridx as u32, height / set.gridy as u32);
    [
        s.x as u32 * gx,
        s.y as u32 * gy,
        s.w as u32 * gx,
        s.h as u32 * gy,
    ]
}

/// C++: `LoadSpriteTexture`'s image: the sprite cut from `from`, or from `fallback`
/// when it is fully transparent in `from`.
pub fn sprite_image(from: &Image, fallback: Option<&Image>, s: &Sprite) -> Image {
    let [x, y, w, h] = sprite_rect(from.width, from.height, s);
    match fallback {
        Some(f) if from.sub_fully_transparent(x, y, w, h) => sprite_image(f, None, s),
        _ => from.sub_image(x, y, w, h),
    }
}

/// C++: `CSkins::LoadSkinData` (skins.cpp:289). `None` where C++ fails the skin.
pub fn load_skin_data(mut image: Image) -> Option<SkinData> {
    let body_sprite = &SPRITES[SPRITE_TEE_BODY];
    let set = &SPRITESETS[body_sprite.set];
    if !image.check_divisibility(set.gridx as u32, set.gridy as u32, true)
        || image.format != Format::Rgba
    {
        return None;
    }
    let [_, _, body_w, body_h] = sprite_rect(image.width, image.height, body_sprite);
    if body_w > image.width || body_h > image.height {
        return None;
    }

    // dig out blood color
    let mut sums = [0i64; 3];
    for y in 0..body_h as usize {
        for x in 0..body_w as usize {
            let p = &image.data[(y * image.width as usize + x) * 4..][..4];
            if p[3] > 128 {
                for c in 0..3 {
                    sums[c] += p[c] as i64;
                }
            }
        }
    }
    let v = sums.map(|s| s as f32);
    // C++: normalize(vec3)
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let blood = if len == 0.0 {
        [0.0; 3]
    } else {
        let l = 1.0 / len;
        v.map(|c| c * l)
    };

    let (mut body, mut feet) = (Metric::default(), Metric::default());
    let check = |metric: &mut Metric, sprite: usize| {
        let [x, y, w, h] = sprite_rect(image.width, image.height, &SPRITES[sprite]);
        metric.check(&image, x as i32, y as i32, w as i32, h as i32);
    };
    check(&mut body, SPRITE_TEE_BODY);
    check(&mut body, SPRITE_TEE_BODY_OUTLINE);
    check(&mut feet, SPRITE_TEE_FOOT);
    check(&mut feet, SPRITE_TEE_FOOT_OUTLINE);

    let mut grayscale = image.clone();
    grayscale.to_grayscale();
    let body_pixels = || {
        (0..body_h as usize).flat_map(move |y| {
            (0..body_w as usize).map(move |x| (y * image.width as usize + x) * 4)
        })
    };
    // find most common non-zero frequency
    let mut freq = [0i32; 256];
    for o in body_pixels() {
        if grayscale.data[o + 3] > 128 {
            freq[grayscale.data[o] as usize] += 1;
        }
    }
    let mut org_weight = 1u8;
    for i in 1..256 {
        if freq[org_weight as usize] < freq[i] {
            org_weight = i as u8;
        }
    }
    const NEW_WEIGHT: i32 = 192;
    for o in body_pixels() {
        let v = grayscale.data[o];
        let v = if v <= org_weight {
            ((v as f32 / org_weight as f32) * NEW_WEIGHT as f32) as u8
        } else {
            let (v, org) = (v as i32, org_weight as i32);
            (((v - org) as f32 / (255 - org) as f32) * (255 - NEW_WEIGHT) as f32
                + NEW_WEIGHT as f32) as u8
        };
        grayscale.data[o..o + 3].fill(v);
    }
    Some(SkinData {
        image,
        grayscale,
        blood,
        body,
        feet,
    })
}
