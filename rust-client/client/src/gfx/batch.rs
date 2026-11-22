//! The quad part of C++ `IGraphics` (`src/engine/client/graphics_threaded.{h,cpp}`):
//! `MapScreen`, `QuadsBegin`, `SetColor`/`SetColor4`, `QuadsSetSubset(Free)`,
//! `QuadsSetRotation`, `QuadsDraw`/`QuadsDrawTL`/`QuadsDrawFreeform`, blend and wrap
//! modes. It only builds vertices and draw ranges; `Gfx::render` sends them to the GPU.
//! Quads are 4 vertices, drawn as triangles (0, 1, 2) and (0, 2, 3) like the C++ GL
//! backends with `gfx_quad_as_triangle 0`.

/// C++: `CCommandBuffer::SVertex`, with the position already mapped to clip space. The
/// third texture coordinate is the array layer of 2D-array textures. Colours are
/// floats: `set_color` quads carry the C++ 8-bit values, map layers multiply those by
/// envelope colours as the C++ shaders do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 2],
    pub uv: [f32; 3],
    pub color: [f32; 4],
}

impl Vertex {
    pub const SIZE: usize = 36;

    pub fn write(&self, out: &mut Vec<u8>) {
        for f in self.pos.iter().chain(&self.uv).chain(&self.color) {
            out.extend_from_slice(&f.to_le_bytes());
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    None,
    /// C++ `BlendNormal`: `glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA)`.
    Normal,
    /// C++ `BlendAdditive`: `glBlendFunc(GL_SRC_ALPHA, GL_ONE)`.
    Additive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wrap {
    Repeat,
    Clamp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureId(pub usize);

/// One GPU draw: `count` quads from `first`, with the state they were added with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Draw {
    pub texture: Option<TextureId>,
    pub blend: Blend,
    pub wrap: Wrap,
    /// C++ `ClipEnable(x, y, w, h)` in target pixels, before clamping.
    pub clip: Option<[i32; 4]>,
    pub first: u32,
    pub count: u32,
}

/// C++: `CQuadItem`.
#[derive(Clone, Copy, Debug)]
pub struct QuadItem {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// C++: `CFreeformItem`: corners top-left, top-right, bottom-left, bottom-right.
#[derive(Clone, Copy, Debug)]
pub struct FreeformItem(pub [[f32; 2]; 4]);

/// C++: `NormalizeColorComponent`: rounded to the nearest of 256 steps.
pub fn normalize_color(c: [f32; 4]) -> [u8; 4] {
    c.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
}

/// An 8-bit colour as the GPU reads it (normalized).
fn unorm(c: [u8; 4]) -> [f32; 4] {
    c.map(|v| v as f32 / 255.0)
}

/// The frame's quads and draws plus the C++ graphics state that applies to new quads.
pub struct Batch {
    /// C++: `m_State.m_ScreenTL`/`m_ScreenBR`.
    screen: [f32; 4],
    texture: Option<TextureId>,
    blend: Blend,
    wrap: Wrap,
    clip: Option<[i32; 4]>,
    rotation: f32,
    /// C++: `m_aTexture`: [0] top-left, [1] top-right, [2] bottom-right, [3] bottom-left
    /// for `QuadsSetSubset`; `QuadsSetSubsetFree` fills them in freeform corner order.
    uv: [[f32; 2]; 4],
    /// C++: `m_aColor` (8-bit, normalized), same slots as `uv` (`SetColor4` puts
    /// bottom-right in [2]).
    colors: [[f32; 4]; 4],
    /// C++ `Clear` colour (alpha 0) for the start of the frame, if any.
    pub clear: Option<[f32; 3]>,
    pub vertices: Vec<Vertex>,
    pub draws: Vec<Draw>,
}

impl Default for Batch {
    fn default() -> Self {
        Self {
            screen: [0.0, 0.0, 1.0, 1.0],
            texture: None,
            blend: Blend::Normal,
            wrap: Wrap::Repeat,
            clip: None,
            rotation: 0.0,
            uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            colors: [[1.0; 4]; 4],
            clear: None,
            vertices: Vec::new(),
            draws: Vec::new(),
        }
    }
}

impl Batch {
    /// C++: `Clear`.
    pub fn clear(&mut self, r: f32, g: f32, b: f32) {
        self.clear = Some([r, g, b]);
    }

    /// C++: `MapScreen(TopLeftX, TopLeftY, BottomRightX, BottomRightY)`.
    pub fn map_screen(&mut self, tl_x: f32, tl_y: f32, br_x: f32, br_y: f32) {
        self.screen = [tl_x, tl_y, br_x, br_y];
    }

    /// C++: `TextureSet` (`None` = `TextureClear`, untextured).
    pub fn texture_set(&mut self, texture: Option<TextureId>) {
        self.texture = texture;
    }

    pub fn blend(&mut self, blend: Blend) {
        self.blend = blend;
    }

    pub fn wrap(&mut self, wrap: Wrap) {
        self.wrap = wrap;
    }

    /// C++: `ClipEnable` (pixels of the render target).
    pub fn clip_enable(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.clip = Some([x, y, w, h]);
    }

    pub fn clip_disable(&mut self) {
        self.clip = None;
    }

    /// The current `MapScreen` rectangle (left, top, right, bottom).
    pub fn screen(&self) -> [f32; 4] {
        self.screen
    }

    /// C++: `QuadsBegin`: full subset, no rotation, white.
    pub fn quads_begin(&mut self) {
        self.set_subset(0.0, 0.0, 1.0, 1.0);
        self.rotation = 0.0;
        self.set_color([1.0; 4]);
    }

    pub fn set_rotation(&mut self, angle: f32) {
        self.rotation = angle;
    }

    pub fn set_color(&mut self, c: [f32; 4]) {
        self.colors = [unorm(normalize_color(c)); 4];
    }

    /// C++: `SetColor4(TopLeft, TopRight, BottomLeft, BottomRight)`.
    pub fn set_color4(&mut self, tl: [f32; 4], tr: [f32; 4], bl: [f32; 4], br: [f32; 4]) {
        self.colors = [tl, tr, br, bl].map(|c| unorm(normalize_color(c)));
    }

    /// C++: `QuadsSetSubset`.
    pub fn set_subset(&mut self, tl_u: f32, tl_v: f32, br_u: f32, br_v: f32) {
        self.uv = [[tl_u, tl_v], [br_u, tl_v], [br_u, br_v], [tl_u, br_v]];
    }

    /// C++: `QuadsSetSubsetFree` (without the 3D texture index).
    pub fn set_subset_free(&mut self, uv: [[f32; 2]; 4]) {
        self.uv = uv;
    }

    /// C++: `QuadsDraw`: items are centered on (x, y).
    pub fn quads_draw(&mut self, items: &[QuadItem]) {
        for q in items {
            self.quads_draw_tl(&[QuadItem {
                x: q.x - q.w / 2.0,
                y: q.y - q.h / 2.0,
                ..*q
            }]);
        }
    }

    /// C++: `QuadsDrawTLImpl` (4-vertex path), rotated around the quad's center.
    pub fn quads_draw_tl(&mut self, items: &[QuadItem]) {
        for q in items {
            let mut pos = [
                [q.x, q.y],
                [q.x + q.w, q.y],
                [q.x + q.w, q.y + q.h],
                [q.x, q.y + q.h],
            ];
            if self.rotation != 0.0 {
                let center = [q.x + q.w / 2.0, q.y + q.h / 2.0];
                let (cos, sin) = (self.rotation.cos(), self.rotation.sin());
                for p in &mut pos {
                    // C++: Rotate
                    let o = [p[0] - center[0], p[1] - center[1]];
                    *p = [
                        o[0] * cos - o[1] * sin + center[0],
                        o[0] * sin + o[1] * cos + center[1],
                    ];
                }
            }
            self.push_quad(pos, [0, 1, 2, 3]);
        }
    }

    /// C++: `QuadsDrawFreeform` (4-vertex path): vertices P0, P1, P3, P2, each with the
    /// texture coordinate and colour slot of its corner index (P3 takes slot 3, P2 slot 2).
    pub fn quads_draw_freeform(&mut self, items: &[FreeformItem]) {
        for f in items {
            let p = f.0;
            self.push_quad([p[0], p[1], p[3], p[2]], [0, 1, 3, 2]);
        }
    }

    fn push_quad(&mut self, pos: [[f32; 2]; 4], slots: [usize; 4]) {
        let uv = slots.map(|s| [self.uv[s][0], self.uv[s][1], 0.0]);
        let colors = slots.map(|s| self.colors[s]);
        self.quad_raw(pos, uv, colors);
    }

    /// One quad in screen coordinates with its own texture coordinates (z: array
    /// layer) and float colours, vertices in the order TL, TR, BR, BL, drawn with the
    /// current screen, texture, blend, wrap and clip. The map renderer builds its
    /// vertices like the C++ buffered tile and quad layers and passes them here.
    pub fn quad_raw(&mut self, pos: [[f32; 2]; 4], uv: [[f32; 3]; 4], colors: [[f32; 4]; 4]) {
        let [l, t, r, b] = self.screen;
        for ((p, uv), color) in pos.iter().zip(uv).zip(colors) {
            // C++ GL: glOrtho(TL.x, BR.x, BR.y, TL.y): y points down
            self.vertices.push(Vertex {
                pos: [
                    (p[0] - l) / (r - l) * 2.0 - 1.0,
                    1.0 - (p[1] - t) / (b - t) * 2.0,
                ],
                uv,
                color,
            });
        }
        let quad = self.vertices.len() as u32 / 4 - 1;
        let state = (self.texture, self.blend, self.wrap, self.clip);
        match self.draws.last_mut() {
            Some(d)
                if (d.texture, d.blend, d.wrap, d.clip) == state && d.first + d.count == quad =>
            {
                d.count += 1
            }
            _ => self.draws.push(Draw {
                texture: state.0,
                blend: state.1,
                wrap: state.2,
                clip: state.3,
                first: quad,
                count: 1,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_color_rounds_and_clamps() {
        assert_eq!(normalize_color([0.5, 1.5, -1.0, 0.25]), [128, 255, 0, 64]);
    }

    #[test]
    fn quad_vertices_follow_cpp_corner_slots() {
        let mut b = Batch::default();
        b.map_screen(0.0, 0.0, 100.0, 50.0);
        b.quads_begin();
        b.set_color4(
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            [1.0; 4],
        );
        b.set_subset(0.25, 0.5, 0.75, 1.0);
        b.quads_draw_tl(&[QuadItem {
            x: 0.0,
            y: 0.0,
            w: 50.0,
            h: 25.0,
        }]);
        let v = &b.vertices;
        // TL, TR, BR, BL in clip space (y up)
        assert_eq!(
            v.iter().map(|v| v.pos).collect::<Vec<_>>(),
            [[-1.0, 1.0], [0.0, 1.0], [0.0, 0.0], [-1.0, 0.0]]
        );
        assert_eq!(v[2].uv, [0.75, 1.0, 0.0]);
        assert_eq!(v[3].uv, [0.25, 1.0, 0.0]);
        assert_eq!(v[2].color, [1.0; 4]); // bottom-right
        assert_eq!(v[3].color, [0.0, 0.0, 1.0, 1.0]); // bottom-left
    }

    #[test]
    fn freeform_uses_slot_of_its_corner() {
        let mut b = Batch::default();
        b.quads_begin();
        b.set_subset_free([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]);
        b.set_color4(
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            [1.0; 4],
        );
        b.quads_draw_freeform(&[FreeformItem([
            [0.0, 0.0],
            [1.0, 0.0],
            [0.0, 1.0],
            [1.0, 1.0],
        ])]);
        let v = &b.vertices;
        // third vertex is P3 (bottom-right) with uv slot 3 and colour slot 3, which
        // SetColor4 filled with the bottom-left colour: the C++ quirk is kept
        assert_eq!(v[2].pos, [1.0, -1.0]);
        assert_eq!(v[2].uv, [1.0, 1.0, 0.0]);
        assert_eq!(v[2].color, [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn rotation_is_around_the_center() {
        let mut b = Batch::default();
        b.map_screen(-1.0, 1.0, 1.0, -1.0); // identity
        b.quads_begin();
        b.set_rotation(std::f32::consts::PI);
        b.quads_draw(&[QuadItem {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        }]);
        let tl = b.vertices[0].pos;
        assert!(
            (tl[0] - 0.5).abs() < 1e-6 && (tl[1] - 0.5).abs() < 1e-6,
            "{tl:?}"
        );
    }

    #[test]
    fn draws_merge_while_state_is_unchanged() {
        let mut b = Batch::default();
        let q = QuadItem {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        };
        b.quads_draw_tl(&[q, q]);
        b.blend(Blend::Additive);
        b.quads_draw_tl(&[q]);
        assert_eq!(b.draws.len(), 2);
        assert_eq!((b.draws[0].count, b.draws[1].first), (2, 2));
    }
}
