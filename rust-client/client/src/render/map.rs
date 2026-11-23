//! Map layers. C++: `CMapRenderer::Load`/`Render` (`src/game/map/map_renderer.cpp`) and
//! `CRenderLayerGroup`, `CRenderLayerTile`, `CRenderLayerQuads` (`render_layer.cpp`) as
//! the in-game client uses them: buffered tile and quad layers, `cl_overlay_entities 0`
//! (so the game, front, tele, speedup, switch and tune layers draw nothing),
//! `gfx_high_detail 1`, `gfx_noclip 0`, `cl_showquads 1`. Vertices are built like the
//! C++ buffers and shaders build them; only culling shortcuts are left out.

use ddnet::map::Map;
use ddnet::map::images::MapImage;
use ddnet::physics::vmath::PI;

use super::envelope::Envelopes;
use super::map_screen_to_world;
use crate::gfx::Gfx;
use crate::gfx::batch::{Batch, Blend, TextureId, Wrap};
use crate::image::Image;

const MAPITEMTYPE_GROUP: i32 = 4;
const MAPITEMTYPE_LAYER: i32 = 5;
const LAYERTYPE_TILES: i32 = 2;
const LAYERTYPE_QUADS: i32 = 3;
const TILESLAYERFLAG_GAME: i32 = 1;
/// Tele, speedup, front, switch, tune and game: entity layers, not drawn in game.
const TILESLAYERFLAG_PHYSICS: i32 = 0x3f;
const TILEFLAG_XFLIP: u8 = 1;
const TILEFLAG_YFLIP: u8 = 2;
const TILEFLAG_ROTATE: u8 = 8;

/// C++: `CalculateTexCoords` (render_layer.cpp): the texture corners (TL, TR, BR, BL)
/// of a tile for each of the 8 flip/rotate combinations.
fn tex_coords(table_flag: u8) -> ([u8; 4], [u8; 4]) {
    let (mut x, mut y) = ([0, 1, 1, 0], [0, 0, 1, 1]);
    if table_flag & TILEFLAG_XFLIP != 0 {
        x.rotate_left(2);
    }
    if table_flag & TILEFLAG_YFLIP != 0 {
        y.rotate_left(2);
    }
    if table_flag & (TILEFLAG_ROTATE >> 1) != 0 {
        x.rotate_left(3);
        y.rotate_left(3);
    }
    (x, y)
}

/// A tile quad: positions TL, TR, BR, BL, texture coordinates with the tile index as
/// array layer, and whether it is rotated. C++: `FillTmpTile` with scale 32.
struct TileQuad {
    pos: [[f32; 2]; 4],
    uv: [[f32; 3]; 4],
    rotated: bool,
}

fn tile_quad(x: i32, y: i32, offset: (i32, i32), index: u8, flags: u8) -> TileQuad {
    let table = (flags & (TILEFLAG_XFLIP | TILEFLAG_YFLIP)) + ((flags & TILEFLAG_ROTATE) >> 1);
    let (tx, ty) = tex_coords(table);
    let (l, t) = ((x * 32 + offset.0) as f32, (y * 32 + offset.1) as f32);
    let (r, b) = (
        (x * 32 + 32 + offset.0) as f32,
        (y * 32 + 32 + offset.1) as f32,
    );
    TileQuad {
        pos: [[l, t], [r, t], [r, b], [l, b]],
        uv: [0, 1, 2, 3].map(|i| [tx[i] as f32, ty[i] as f32, index as f32]),
        rotated: flags & TILEFLAG_ROTATE != 0,
    }
}

/// C++: the `tile_border` shader: a border tile stretched by `scale` (in tiles) and
/// moved by `offset` (in tiles); texture coordinates are scaled too (swapped for
/// rotated tiles), and the fragment shader wraps them (`fract`), so the tile repeats.
fn border_quad(q: &TileQuad, offset: (f32, f32), scale: (f32, f32)) -> TileQuad {
    let tex_scale = if q.rotated { (scale.1, scale.0) } else { scale };
    TileQuad {
        pos: q.pos.map(|p| {
            [
                p[0] * scale.0 + offset.0 * 32.0,
                p[1] * scale.1 + offset.1 * 32.0,
            ]
        }),
        uv: q.uv.map(|u| [u[0] * tex_scale.0, u[1] * tex_scale.1, u[2]]),
        rotated: q.rotated,
    }
}

struct Group {
    offset: (i32, i32),
    parallax: (i32, i32),
    /// `m_Version >= 2 && m_UseClipping`: x, y, w, h in world units.
    clip: Option<[i32; 4]>,
}

struct TileLayer {
    width: i32,
    height: i32,
    color: [f32; 4],
    color_env: i32,
    color_env_offset: i32,
    texture: Option<TextureId>,
    /// (index, flags) per tile.
    tiles: Vec<(u8, u8)>,
}

/// C++: `CQuad`.
struct Quad {
    points: [[i32; 2]; 5],
    colors: [[i32; 4]; 4],
    texcoords: [[i32; 2]; 4],
    pos_env: i32,
    pos_env_offset: i32,
    color_env: i32,
    color_env_offset: i32,
}

struct QuadLayer {
    quads: Vec<Quad>,
    texture: Option<TextureId>,
}

enum Entry {
    Group(Group),
    Tiles(TileLayer),
    Quads(QuadLayer),
}

/// Camera and time for one frame. C++: `CRenderLayerParams` plus the envelope clock.
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub center: (f32, f32),
    pub zoom: f32,
    /// Target size in pixels (C++ `ScreenWidth`/`ScreenHeight`).
    pub width: u32,
    pub height: u32,
    /// C++ `CEnvelopeState` time in ns.
    pub envelope_time: i64,
}

impl View {
    fn aspect(&self) -> f32 {
        self.width as f32 / self.height as f32
    }
}

pub struct MapRender {
    background: Vec<Entry>,
    foreground: Vec<Entry>,
    envelopes: Envelopes,
}

fn fx2f(v: i32) -> f32 {
    v as f32 / 1024.0
}

impl MapRender {
    /// C++: `CMapImages::OnMapLoad` textures plus `CMapRenderer::Load` for the
    /// background and foreground render types.
    pub fn new(gfx: &mut Gfx, map: &Map, images: &[Option<Image>]) -> Self {
        let usage = MapImage::usage(map);
        // (tile array, 2D) per image
        let textures: Vec<(Option<TextureId>, Option<TextureId>)> = images
            .iter()
            .zip(&usage)
            .map(|(img, &used)| {
                let Some(img) = img else { return (None, None) };
                let rgba = img.to_rgba();
                // ponytail: C++ resizes tilesets not divisible by 16 to powers of two;
                // such tilesets draw untextured here
                let array = (used & 1 != 0 && img.width % 16 == 0 && img.height % 16 == 0)
                    .then(|| gfx.create_tile_array(img.width, img.height, &rgba));
                let flat =
                    (used & 2 != 0).then(|| gfx.create_texture(img.width, img.height, &rgba));
                (array, flat)
            })
            .collect();
        let entries = |foreground: bool| load(map, &textures, foreground);
        Self {
            background: entries(false),
            foreground: entries(true),
            envelopes: Envelopes::from_map(map),
        }
    }

    pub fn render_background(&self, batch: &mut Batch, view: &View) {
        self.render(&self.background, batch, view);
    }

    pub fn render_foreground(&self, batch: &mut Batch, view: &View) {
        self.render(&self.foreground, batch, view);
    }

    /// C++: `CMapRenderer::Render`.
    fn render(&self, entries: &[Entry], batch: &mut Batch, view: &View) {
        let mut do_render_group = true;
        for e in entries {
            match e {
                Entry::Group(g) => {
                    do_render_group = group_do_render(g, batch, view);
                    if do_render_group {
                        group_render(g, batch, view);
                    }
                }
                _ if !do_render_group => {}
                Entry::Tiles(t) => self.render_tiles(t, batch, view),
                Entry::Quads(q) => self.render_quads(q, batch, view),
            }
        }
        batch.clip_disable();
        // C++ resets the screen to the interface (background) or to what it was
        let r = map_screen_to_world(
            view.center.0,
            view.center.1,
            100.0,
            100.0,
            100.0,
            0.0,
            0.0,
            view.aspect(),
            view.zoom,
        );
        batch.map_screen(r[0], r[1], r[2], r[3]);
    }

    /// C++: `CRenderLayerTile::Render` with `RenderTileLayer` and `RenderTileBorder`.
    fn render_tiles(&self, t: &TileLayer, batch: &mut Batch, view: &View) {
        batch.texture_set(t.texture);
        batch.blend(Blend::Normal);
        // C++: GetRenderColor
        let mut env = [1.0; 4];
        self.envelopes.eval(
            view.envelope_time,
            t.color_env_offset,
            t.color_env,
            &mut env,
            4,
        );
        let color: [f32; 4] = std::array::from_fn(|i| t.color[i] * env[i]);
        let colors = [color; 4];
        let [l, top, r, b] = batch.screen();
        let x0 = (l / 32.0).floor() as i32;
        let y0 = (top / 32.0).floor() as i32;
        let x1 = (r / 32.0).ceil() as i32;
        let y1 = (b / 32.0).ceil() as i32;
        let tile = |x: i32, y: i32| t.tiles[(y * t.width + x) as usize];

        // visible tiles
        let (cx0, cx1) = (x0.max(0), x1.clamp(0, t.width));
        let (cy0, cy1) = (y0.max(0), y1.clamp(0, t.height));
        for y in cy0..cy1 {
            for x in cx0..cx1 {
                let (index, flags) = tile(x, y);
                if index > 0 {
                    let q = tile_quad(x, y, (0, 0), index, flags);
                    batch.quad_raw(q.pos, q.uv, colors);
                }
            }
        }
        if !(x1 > t.width || y1 > t.height || x0 < 0 || y0 < 0) {
            return;
        }
        // C++: RenderTileBorder: the edge tiles stretched over what lies outside the map,
        // drawn with the repeating `tile_border` shader
        batch.set_border(true);
        let mut draw = |q: &TileQuad| batch.quad_raw(q.pos, q.uv, colors);
        let (w, h) = (t.width, t.height);
        let mut edge =
            |x: i32, y: i32, at: (i32, i32), offset: (i32, i32), o: (f32, f32), s: (f32, f32)| {
                let (index, flags) = tile(x, y);
                if index > 0 {
                    draw(&border_quad(
                        &tile_quad(at.0, at.1, offset, index, flags),
                        o,
                        s,
                    ));
                }
            };
        let (fw, fh) = (w as f32, h as f32);
        // C++ fills the right and bottom border tiles in `else if (x == W - 1)` and
        // `else if (y == H - 1)` branches, so a 1-tile wide (high) layer has none
        let (right, bottom) = (w > 1, h > 1);
        if x0 < 0 {
            if y0 < 0 {
                edge(
                    0,
                    0,
                    (0, 0),
                    (-32, -32),
                    (0.0, 0.0),
                    ((-x0) as f32, (-y0) as f32),
                );
            }
            if y1 > h && bottom {
                edge(
                    0,
                    h - 1,
                    (0, 0),
                    (-32, 0),
                    (0.0, fh),
                    ((-x0) as f32, (y1 - h) as f32),
                );
            }
        }
        if x1 > w && right {
            if y0 < 0 {
                edge(
                    w - 1,
                    0,
                    (0, 0),
                    (0, -32),
                    (fw, 0.0),
                    ((x1 - w) as f32, (-y0) as f32),
                );
            }
            if y1 > h && bottom {
                edge(
                    w - 1,
                    h - 1,
                    (0, 0),
                    (0, 0),
                    (fw, fh),
                    ((x1 - w) as f32, (y1 - h) as f32),
                );
            }
        }
        let (by0, by1, bx0, bx1) = (y0.max(0), h.min(y1), x0.max(0), w.min(x1));
        if by0 < h && by1 > 0 {
            for y in by0..by1 {
                if x1 > w && right {
                    edge(w - 1, y, (0, y), (0, 0), (fw, 0.0), ((x1 - w) as f32, 1.0));
                }
            }
            for y in by0..by1 {
                if x0 < 0 {
                    edge(0, y, (0, y), (-32, 0), (0.0, 0.0), ((-x0) as f32, 1.0));
                }
            }
        }
        if bx0 < w && bx1 > 0 {
            for x in bx0..bx1 {
                if y0 < 0 {
                    edge(x, 0, (x, 0), (0, -32), (0.0, 0.0), (1.0, (-y0) as f32));
                }
            }
            for x in bx0..bx1 {
                if y1 > h && bottom {
                    edge(x, h - 1, (x, 0), (0, 0), (0.0, fh), (1.0, (y1 - h) as f32));
                }
            }
        }
        batch.set_border(false);
    }

    /// C++: `CRenderLayerQuads::RenderQuadLayer` and the `quad` shader.
    fn render_quads(&self, layer: &QuadLayer, batch: &mut Batch, view: &View) {
        batch.texture_set(layer.texture);
        batch.blend(Blend::Normal);
        batch.wrap(Wrap::Repeat);
        for q in &layer.quads {
            let mut color = [1.0; 4];
            if q.color_env >= 0 {
                self.envelopes.eval(
                    view.envelope_time,
                    q.color_env_offset,
                    q.color_env,
                    &mut color,
                    4,
                );
            }
            if color[3] <= 0.0 {
                continue; // C++ draws it fully transparent
            }
            let mut pos = [0.0; 4];
            self.envelopes
                .eval(view.envelope_time, q.pos_env_offset, q.pos_env, &mut pos, 3);
            let (ox, oy, rotation) = (pos[0], pos[1], pos[2] / 180.0 * PI);
            let center = [fx2f(q.points[4][0]), fx2f(q.points[4][1])];
            // C++ vertex order: points 0, 1, 3, 2 (TL, TR, BR, BL)
            let corner = [0, 1, 3, 2];
            let positions = corner.map(|i| {
                let mut p = [fx2f(q.points[i][0]), fx2f(q.points[i][1])];
                if rotation != 0.0 {
                    let (x, y) = (p[0] - center[0], p[1] - center[1]);
                    p = [
                        x * rotation.cos() - y * rotation.sin() + center[0],
                        x * rotation.sin() + y * rotation.cos() + center[1],
                    ];
                }
                [p[0] + ox, p[1] + oy]
            });
            let uv = corner.map(|i| [fx2f(q.texcoords[i][0]), fx2f(q.texcoords[i][1]), 0.0]);
            let colors = corner.map(|i| {
                // C++: (unsigned char) of the stored int, normalized, times the envelope
                let c = q.colors[i].map(|v| v as u8 as f32 / 255.0);
                std::array::from_fn(|k| c[k] * color[k])
            });
            batch.quad_raw(positions, uv, colors);
        }
    }
}

/// C++: `CRenderLayerGroup::DoRender`: the group's clip rectangle in pixels, or `false`
/// when it is entirely off screen.
fn group_do_render(g: &Group, batch: &mut Batch, view: &View) -> bool {
    batch.clip_disable();
    let Some([cx, cy, cw, ch]) = g.clip else {
        return true;
    };
    // C++: MapScreenToInterface
    let r = map_screen_to_world(
        view.center.0,
        view.center.1,
        100.0,
        100.0,
        100.0,
        0.0,
        0.0,
        view.aspect(),
        view.zoom,
    );
    batch.map_screen(r[0], r[1], r[2], r[3]);
    let (sw, sh) = (r[2] - r[0], r[3] - r[1]);
    let left = cx as f32 - r[0];
    let top = cy as f32 - r[1];
    let right = cx.wrapping_add(cw) as f32 - r[0];
    let bottom = cy.wrapping_add(ch) as f32 - r[1];
    if right < 0.0 || left > sw || bottom < 0.0 || top > sh {
        return false;
    }
    let (pw, ph) = (view.width as f32, view.height as f32);
    let x = (left * pw / sw).round() as i32;
    let y = (top * ph / sh).round() as i32;
    batch.clip_enable(
        x,
        y,
        (right * pw / sw).round() as i32 - x,
        (bottom * ph / sh).round() as i32 - y,
    );
    true
}

/// C++: `CRenderLayerGroup::Render`: the group's parallax view.
fn group_render(g: &Group, batch: &mut Batch, view: &View) {
    let parallax_zoom = g.parallax.0.max(g.parallax.1).clamp(0, 100) as f32;
    let r = map_screen_to_world(
        view.center.0,
        view.center.1,
        g.parallax.0 as f32,
        g.parallax.1 as f32,
        parallax_zoom,
        g.offset.0 as f32,
        g.offset.1 as f32,
        view.aspect(),
        view.zoom,
    );
    batch.map_screen(r[0], r[1], r[2], r[3]);
}

/// C++: `CMapRenderer::Load` for `RENDERTYPE_BACKGROUND` (up to the game layer) or
/// `RENDERTYPE_FOREGROUND` (from the game layer on). Invalid layers are skipped.
fn load(
    map: &Map,
    textures: &[(Option<TextureId>, Option<TextureId>)],
    foreground: bool,
) -> Vec<Entry> {
    let df = &map.datafile;
    let layers = df.get_type(MAPITEMTYPE_LAYER);
    let mut entries = Vec::new();
    let mut passed_game_layer = false;
    for g in df.get_type(MAPITEMTYPE_GROUP) {
        let g = &df.items[g].data; // at least the v1 fields, checked by Map::load
        let mut group = Some(Group {
            offset: (g[1], g[2]),
            parallax: (g[3], g[4]),
            clip: (g[0] >= 2 && g.get(7).is_some_and(|&c| c != 0))
                .then(|| Some([*g.get(8)?, *g.get(9)?, *g.get(10)?, *g.get(11)?]))
                .flatten(),
        });
        for l in layers.start + g[5] as usize..layers.start + (g[5] + g[6]) as usize {
            let l = &df.items[l].data;
            let tiles = l[1] == LAYERTYPE_TILES;
            let is_game = tiles && l[6] & TILESLAYERFLAG_GAME != 0;
            passed_game_layer |= is_game;
            if !foreground && passed_game_layer {
                return entries;
            }
            if foreground && !passed_game_layer {
                continue;
            }
            if let Some(group) = group.take() {
                entries.push(Entry::Group(group));
            }
            // detail layers (layer flag 1) draw too: gfx_high_detail 1
            // physics layers are entity layers: nothing to draw in game, but their
            // group was pushed above like C++ does
            if tiles {
                if l[6] & TILESLAYERFLAG_PHYSICS == 0
                    && let Some(t) = tile_layer(map, l, textures)
                {
                    entries.push(Entry::Tiles(t));
                }
            } else if l[1] == LAYERTYPE_QUADS
                && let Some(q) = quad_layer(map, l, textures)
            {
                entries.push(Entry::Quads(q));
            }
        }
    }
    entries
}

/// A design tile layer (`CMapItemLayerTilemap`, upgraded by `Map::load`), or `None` if
/// its data is invalid (C++ `IsValid`).
fn tile_layer(
    map: &Map,
    l: &[i32],
    textures: &[(Option<TextureId>, Option<TextureId>)],
) -> Option<TileLayer> {
    let (width, height) = (l[4], l[5]);
    let data = map.client_data(l[14])?;
    let count = (width as usize).checked_mul(height as usize)?;
    let tiles = data
        .get(..count * 4)?
        .as_chunks::<4>()
        .0
        .iter()
        .map(|t| (t[0], t[1]))
        .collect();
    Some(TileLayer {
        width,
        height,
        color: [l[7], l[8], l[9], l[10]].map(|c| c as f32 / 255.0),
        color_env: l[11],
        color_env_offset: l[12],
        texture: usize::try_from(l[13]).ok().and_then(|i| textures.get(i)?.0),
        tiles,
    })
}

/// A quad layer (`CMapItemLayerQuads`), or `None` without valid quads (C++ `IsValid`).
fn quad_layer(
    map: &Map,
    l: &[i32],
    textures: &[(Option<TextureId>, Option<TextureId>)],
) -> Option<QuadLayer> {
    let (num, data_index) = (*l.get(4)?, *l.get(5)?);
    let image = l.get(6).copied().unwrap_or(-1);
    let data = map.client_data(data_index)?;
    // C++: m_NumQuads > 0 && DataSize / sizeof(CQuad) >= m_NumQuads
    if num <= 0 || data.len() / 152 < num as usize {
        return None;
    }
    let ints: Vec<i32> = data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&b| i32::from_le_bytes(b))
        .collect();
    let quads = ints
        .as_chunks::<38>()
        .0
        .iter()
        .take(num as usize)
        .map(|q| Quad {
            points: std::array::from_fn(|i| [q[2 * i], q[2 * i + 1]]),
            colors: std::array::from_fn(|i| {
                [q[10 + 4 * i], q[11 + 4 * i], q[12 + 4 * i], q[13 + 4 * i]]
            }),
            texcoords: std::array::from_fn(|i| [q[26 + 2 * i], q[27 + 2 * i]]),
            pos_env: q[34],
            pos_env_offset: q[35],
            color_env: q[36],
            color_env_offset: q[37],
        })
        .collect();
    Some(QuadLayer {
        quads,
        texture: usize::try_from(image).ok().and_then(|i| textures.get(i)?.1),
    })
}

/// C++: `color_cast<ColorRGBA>(ColorHSLA(packed))` for a packed config colour without
/// alpha, e.g. `cl_background_color 128`.
pub fn hsl_packed_to_rgb(packed: u32) -> [f32; 3] {
    hsl_to_rgb(unpack_hsl(packed))
}

/// C++: `ColorHSLA(unsigned)` without alpha: (h, s, l).
pub fn unpack_hsl(packed: u32) -> [f32; 3] {
    [
        ((packed >> 16) & 0xff) as f32 / 255.0,
        ((packed >> 8) & 0xff) as f32 / 255.0,
        (packed & 0xff) as f32 / 255.0,
    ]
}

/// C++: `color_cast<ColorRGBA>(const ColorHSLA &)` (color.h).
pub fn hsl_to_rgb([h, s, l]: [f32; 3]) -> [f32; 3] {
    let h1 = h * 6.0;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    // C++ std::fmod(float, int) and what follows are computed in double
    let x = (c as f64 * (1.0 - ((h1 as f64) % 2.0 - 1.0).abs())) as f32;
    let (r, g, b) = match ddnet::physics::vmath::trunc_i32(h1) {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        5 | 6 => (c, 0.0, x),
        _ => (0.0, 0.0, 0.0),
    };
    let m = l - c / 2.0;
    [r + m, g + m, b + m]
}
