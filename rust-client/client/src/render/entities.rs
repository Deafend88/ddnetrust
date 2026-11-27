//! The entities textures: C++ `CMapImages::GetEntities` (mapimages.cpp:262). The pack's
//! image for the server's mod (`cl_assets_entities`, else `editor/entities_clear`), the
//! tune tile made gray, then one texture per layer kind with only the tiles that kind can
//! hold (when the server masks entities).

use crate::image::{Format, Image, load_png_file};
use crate::storage::Storage;

/// C++ `gs_apModEntitiesNames`, by `EMapImageModType`.
pub const MOD_NAMES: [&str; 7] = [
    "ddnet",
    "ddrace",
    "race",
    "blockworlds",
    "fng",
    "vanilla",
    "f-ddrace",
];
pub const MOD_DDNET: usize = 0;
pub const MOD_DDRACE: usize = 1;
pub const MOD_RACE: usize = 2;
pub const MOD_BLOCKWORLDS: usize = 3;
pub const MOD_FNG: usize = 4;
pub const MOD_VANILLA: usize = 5;
pub const MOD_FDDRACE: usize = 6;

const ENTITY_OFFSET: i32 = 255 - 16 * 4;
const TILE_TUNE: i32 = 68;
const TILE_SWITCHTIMEDOPEN: i32 = 22;

/// The game info's entity flags (C++ `m_EntitiesFDDrace`, ..., `m_DontMaskEntities`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EntitiesInfo {
    pub fddrace: bool,
    pub ddnet: bool,
    pub ddrace: bool,
    pub race: bool,
    pub bw: bool,
    pub fng: bool,
    pub vanilla: bool,
    pub dont_mask: bool,
}

impl EntitiesInfo {
    /// C++ `GetGameInfo`'s entity flags from the GameInfoEx (version, flags, flags2).
    /// ponytail: without a GameInfoEx C++ guesses from the game type string; this takes
    /// DDNet's then (`Predictor` does the same).
    pub fn from_game_info((version, flags, flags2): (i32, i32, i32)) -> Self {
        use ddnet::protocol::generated as g;
        let has = |f: i32| flags & f != 0;
        let (ddnet, fddrace) = if version < 1 {
            (true, false)
        } else {
            (
                has(g::GAMEINFOFLAG_GAMETYPE_DDNET),
                version >= 6 && flags2 & g::GAMEINFOFLAG2_GAMETYPE_FDDRACE != 0,
            )
        };
        let ddrace = version < 1 || has(g::GAMEINFOFLAG_GAMETYPE_DDRACE) || ddnet || fddrace;
        let race = version < 1
            || has(g::GAMEINFOFLAG_GAMETYPE_RACE)
            || has(g::GAMEINFOFLAG_GAMETYPE_FASTCAP)
            || ddrace;
        let mut e = EntitiesInfo {
            fddrace,
            ddnet,
            ddrace,
            race,
            bw: version >= 1 && has(g::GAMEINFOFLAG_GAMETYPE_BLOCK_WORLDS),
            fng: version >= 1 && has(g::GAMEINFOFLAG_GAMETYPE_FNG),
            vanilla: version >= 1 && has(g::GAMEINFOFLAG_GAMETYPE_VANILLA),
            dont_mask: !ddnet,
        };
        if version >= 2 {
            e.ddnet = has(g::GAMEINFOFLAG_ENTITIES_DDNET);
            e.ddrace = has(g::GAMEINFOFLAG_ENTITIES_DDRACE);
            e.race = has(g::GAMEINFOFLAG_ENTITIES_RACE);
            e.fng = has(g::GAMEINFOFLAG_ENTITIES_FNG);
            e.vanilla = has(g::GAMEINFOFLAG_ENTITIES_VANILLA);
        }
        if version >= 3 {
            e.dont_mask = has(g::GAMEINFOFLAG_DONT_MASK_ENTITIES);
        }
        if version >= 4 {
            e.bw = has(g::GAMEINFOFLAG_ENTITIES_BW);
        }
        if version >= 6 {
            e.fddrace = flags2 & g::GAMEINFOFLAG2_ENTITIES_FDDRACE != 0;
        }
        e
    }

    /// C++ `GetEntitiesModType`.
    pub fn mod_type(&self) -> usize {
        if self.fddrace {
            MOD_FDDRACE
        } else if self.ddnet {
            MOD_DDNET
        } else if self.ddrace {
            MOD_DDRACE
        } else if self.race {
            MOD_RACE
        } else if self.bw {
            MOD_BLOCKWORLDS
        } else if self.fng {
            MOD_FNG
        } else if self.vanilla {
            MOD_VANILLA
        } else {
            MOD_DDNET
        }
    }
}

fn in_range(i: i32, a: i32, b: i32) -> bool {
    (a..=b).contains(&i)
}

/// C++ `IsValidEntity` (mapitems.cpp:143).
fn is_valid_entity(index: i32) -> bool {
    let i = index - ENTITY_OFFSET;
    in_range(i, 1, 27) || in_range(i, 29, 38) || in_range(i, 42, 47) || i == 49
}

/// The tile ranges `IsValidGameTile` and `IsValidFrontTile` share.
fn common_tile(i: i32) -> bool {
    i == 9
        || in_range(i, 11, 13)
        || in_range(i, 144, 145)
        || in_range(i, 16, 22)
        || in_range(i, 32, 62)
        || in_range(i, 64, 67)
        || in_range(i, 71, 76)
        || in_range(i, 88, 91)
        || in_range(i, 112, 113)
        || in_range(i, 128, 129)
        || in_range(i, 104, 107)
        || in_range(i, 190, 191)
        || is_valid_entity(i)
}

/// C++ `IsValidGameTile` (mapitems.cpp:16).
fn is_valid_game_tile(i: i32) -> bool {
    i == 0 || in_range(i, 1, 6) || in_range(i, 96, 97) || common_tile(i)
}

/// C++ `IsValidFrontTile` (mapitems.cpp:37).
fn is_valid_front_tile(i: i32) -> bool {
    i == 0 || i == 2 || in_range(i, 4, 6) || in_range(i, 96, 99) || common_tile(i)
}

/// C++ `IsValidTeleTile`.
fn is_valid_tele_tile(i: i32) -> bool {
    [10, 14, 15, 26, 27, 29, 30, 31, 63].contains(&i)
}

/// C++ `IsValidSpeedupTile`.
fn is_valid_speedup_tile(i: i32) -> bool {
    i == 28 || i == 29
}

/// C++ `IsValidSwitchTile` (mapitems.cpp:97).
fn is_valid_switch_tile(i: i32) -> bool {
    [7, 9, 12, 13, 144, 145, 19, 20, 79, 95, 98, 99].contains(&i)
        || in_range(i, 22, 25)
        || (is_valid_entity(i) && i >= ENTITY_OFFSET + 6)
}

/// C++ `IsCreditsTile`.
fn is_credits_tile(i: i32) -> bool {
    in_range(i, 140, 143) || in_range(i, 156, 159)
}

/// C++ `IsValidTile` (mapitems.cpp, mapimages.cpp:219); `switch` is the switch layer's
/// texture, else the one of every other layer.
pub fn is_valid_tile(switch: bool, masked: bool, mod_type: usize, i: i32) -> bool {
    if i == 0 {
        return false;
    }
    if !masked {
        return true;
    }
    if mod_type == MOD_DDNET || mod_type == MOD_DDRACE {
        let other = !is_valid_game_tile(i)
            && !is_valid_front_tile(i)
            && !is_valid_speedup_tile(i)
            && !is_valid_tele_tile(i)
            && i != TILE_TUNE;
        if (mod_type == MOD_DDNET || i != 28)
            && if switch {
                !is_valid_switch_tile(i)
            } else {
                other
            }
        {
            return false;
        }
    } else if matches!(mod_type, MOD_RACE | MOD_FNG | MOD_VANILLA) && is_credits_tile(i) {
        return false;
    }
    true
}

/// The image C++ `GetEntities` loads and the path it names the textures by: the pack's
/// `<mod>.png`, for DDNet a single `<pack>.png`, else `editor/entities_clear/<mod>.png`.
pub fn load_image(storage: &Storage, pack: &str, mod_type: usize) -> (String, Option<Image>) {
    let dir = if pack == "default" {
        "editor/entities_clear".to_string()
    } else {
        format!("assets/entities/{pack}")
    };
    let load = |p: &str| storage.read(p).and_then(|b| load_png_file(&b));
    let mut path = format!("{dir}/{}.png", MOD_NAMES[mod_type]);
    let mut image = load(&path);
    if image.is_none() && mod_type == MOD_DDNET {
        // try as single ddnet replacement
        path = format!("{dir}.png");
        image = load(&path);
    }
    if image.is_none() {
        path = format!("editor/entities_clear/{}.png", MOD_NAMES[mod_type]);
        image = load(&path);
    }
    (path, image)
}

/// C++ `ConvertToGrayscaleRect`.
fn grayscale_rect(img: &mut Image, x0: usize, y0: usize, w: usize, h: usize) {
    if matches!(img.format, Format::R | Format::Ra) {
        return;
    }
    let step = img.format.channels();
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            let i = (y * img.width as usize + x) * step;
            let Some(p) = img.data.get_mut(i..i + 3) else {
                continue;
            };
            let luma =
                (0.2126f32 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) as u8;
            p.fill(luma);
        }
    }
}

/// C++ `GetEntities`' texture build: [every layer but switch, switch, the tune colour
/// map], same size and format as `img`.
pub fn build(mut img: Image, masked: bool, mod_type: usize) -> [Image; 3] {
    let (cw, ch) = (img.width as usize / 16, img.height as usize / 16);
    let tune = TILE_TUNE as usize;
    let (tx, ty) = ((tune % 16) * cw, (tune / 16) * ch);
    grayscale_rect(&mut img, tx, ty, cw, ch);
    let px = img.format.channels();
    let row = img.width as usize * px;
    let tune_map = tune_map(&img, (tx, ty), (cw, ch));
    let [all, switch] = [false, true].map(|switch| {
        let mut out = vec![0u8; img.data.len()];
        for i in 0..256 {
            if !is_valid_tile(switch, masked, mod_type, i) {
                continue;
            }
            let t = if switch && i == TILE_SWITCHTIMEDOPEN {
                8
            } else {
                i
            } as usize;
            let (ox, oy) = ((t % 16) * cw, (t / 16) * ch);
            for y in oy..oy + ch {
                let a = y * row + ox * px;
                out[a..a + cw * px].copy_from_slice(&img.data[a..a + cw * px]);
            }
        }
        Image {
            width: img.width,
            height: img.height,
            format: img.format,
            data: out,
        }
    });
    [all, switch, tune_map]
}

/// C++ `CMapImages::InitOverlayTextures` with `cl_text_entities_size` `scale`: the
/// [bottom, top, center] number atlases, 1024x1024 with a 64 px cell per number (1-255).
/// The glyphs come from ab_glyph instead of FreeType (no hinting: digit widths can differ
/// by a pixel; DECISIONS 13.4).
pub fn overlay_atlases(font: &[u8], scale: i32) -> Option<[Image; 3]> {
    let font = ab_glyph::FontRef::try_from_slice(font).ok()?;
    let size = (64 * scale / 100).clamp(2, 64);
    // C++: int = int + float
    let center = ((64 - size) / 2) as f32 + size as f32 * 0.1;
    let center = center as i32;
    let atlas = |size: i32, y_offset: i32| {
        let mut img = Image {
            width: 1024,
            height: 1024,
            format: Format::Rgba,
            data: vec![0; 1024 * 1024 * 4],
        };
        for (power, max) in [(0, 9), (1, 99), (2, 255)] {
            layer_text(&font, &mut img, size, 64, y_offset, power, max);
        }
        img
    };
    Some([
        atlas(size / 2, 32 + center / 2),
        atlas(size / 2, center / 2),
        atlas(size, center),
    ])
}

/// A digit's glyph at FreeType pixel size `px` (em = px).
fn digit_glyph(font: &ab_glyph::FontRef, c: char, px: i32) -> Option<ab_glyph::OutlinedGlyph> {
    use ab_glyph::Font;
    let scale = px as f32 * font.height_unscaled() / font.units_per_em()?;
    font.outline_glyph(font.glyph_id(c).with_scale(scale))
}

/// C++ `CalculateTextWidth`: each glyph's width plus 1.
fn text_width(font: &ab_glyph::FontRef, text: &str, px: i32) -> i32 {
    text.chars()
        .filter_map(|c| digit_glyph(font, c, px))
        .map(|g| g.px_bounds().width() as i32 + 1)
        .sum()
}

/// C++ `CMapImages::UpdateEntityLayerText` with `AdjustFontSize` and the glyph upload of
/// `CGlyphMap::UploadEntityLayerText`: the numbers 10^power..=max, each centred in its
/// cell, glyphs top-aligned and 1 px apart, white with the coverage as alpha.
fn layer_text(
    font: &ab_glyph::FontRef,
    img: &mut Image,
    size: i32,
    max_width: i32,
    y_offset: i32,
    power: u32,
    max: i32,
) {
    let first = 10i32.pow(power);
    let width = text_width(font, &first.to_string(), 100);
    let mut font_size = (100.0 / (width as f32 / max_width as f32)) as i32;
    if size > 0 && font_size > size {
        font_size = size;
    }
    let px = (font_size as f32 * 0.92) as i32;
    let y_offset = y_offset + (size - px) / 2;
    if px < 1 {
        return;
    }
    for n in first..=max {
        let text = n.to_string();
        let (x, y) = ((n % 16) * 64, (n / 16) * 64);
        let x_off = (max_width - text_width(font, &text, px).clamp(0, max_width)) / 2;
        let (x, y) = ((x + x_off) as f32, (y + y_offset) as f32);
        let (sub_w, sub_h) = ((64 - x_off) as f32, (64 - y_offset) as f32);
        let mut last = 0;
        for g in text.chars().filter_map(|c| digit_glyph(font, c, px)) {
            let w = g.px_bounds().width() as u32;
            g.draw(|ox, oy, c| {
                let ix = (x + ox as f32 + last as f32).clamp(x, x + sub_w - 1.0) as usize;
                let iy = (y + oy as f32).clamp(y, y + sub_h - 1.0) as usize;
                let i = (iy * 1024 + ix) * 4;
                img.data[i..i + 3].fill(255);
                img.data[i + 3] = (c * 255.0).round() as u8;
            });
            last += w + 1;
        }
    }
}

/// C++ `normalized_golden_angle` (base/math.h).
const NORMALIZED_GOLDEN_ANGLE: f32 = 137.50776 / 360.0;

/// C++ `GetEntities`' tune map (mapimages.cpp:340): the gray tune tile in every cell but
/// 0, each coloured with `ColorizeWithHueRect` at hue `(i - 1) * golden angle`, sat 0.75.
fn tune_map(img: &Image, (tx, ty): (usize, usize), (cw, ch): (usize, usize)) -> Image {
    let px = img.format.channels();
    let row = img.width as usize * px;
    let mut out = vec![0u8; img.data.len()];
    for i in 1..256usize {
        let hue = ((i - 1) as f32 * NORMALIZED_GOLDEN_ANGLE) % 1.0;
        let (sx, sy) = ((i % 16) * cw, (i / 16) * ch);
        for y in 0..ch {
            let (src, dst) = ((ty + y) * row + tx * px, (sy + y) * row + sx * px);
            out[dst..dst + cw * px].copy_from_slice(&img.data[src..src + cw * px]);
            for p in out[dst..dst + cw * px].chunks_exact_mut(px) {
                // C++ color_cast<ColorHSLA>: only the lightness survives the new h, s
                let [r, g, b] = [p[0], p[1], p[2]].map(|c| c as f32 / 255.0);
                let l = 0.5 * (r.max(g).max(b) + r.min(g).min(b));
                let rgb = super::map::hsl_to_rgb([hue, 0.75, l]);
                for (c, v) in p.iter_mut().zip(rgb) {
                    *c = (v * 255.0) as u8;
                }
            }
        }
    }
    Image {
        width: img.width,
        height: img.height,
        format: img.format,
        data: out,
    }
}
