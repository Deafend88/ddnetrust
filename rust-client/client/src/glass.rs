//! Glass (PLAN §10.3, 7.2): a dual-Kawase blur pyramid of the world image, and panel
//! backdrops drawn from it inside egui's render pass (egui-wgpu paint callbacks), so each
//! backdrop has the z-order and clip of the panel it belongs to. Liquid Glass (refraction
//! near the edge, a specular rim, a tint from the backdrop's brightness) is for the
//! Dynamic Island at full strength only (SPEC: less glass everywhere else).

use std::sync::atomic::{AtomicUsize, Ordering};

use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use wgpu::util::DeviceExt;

use crate::gfx::Size;
use crate::style::Glass;

const SHADER: &str = r"
struct VOut {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
}

// one triangle over the whole target
@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> VOut {
    let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return VOut(vec4f(uv * vec2f(2.0, -2.0) + vec2f(-1.0, 1.0), 0.0, 1.0), uv);
}

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;

// dual Kawase (Bjorge, SIGGRAPH 2015); the weights sum to 1, so flat areas keep their colour
@fragment
fn fs_down(in: VOut) -> @location(0) vec4f {
    let h = 0.5 / vec2f(textureDimensions(src));
    var s = textureSampleLevel(src, smp, in.uv, 0.0) * 4.0;
    s += textureSampleLevel(src, smp, in.uv - h, 0.0);
    s += textureSampleLevel(src, smp, in.uv + h, 0.0);
    s += textureSampleLevel(src, smp, in.uv + vec2f(h.x, -h.y), 0.0);
    s += textureSampleLevel(src, smp, in.uv - vec2f(h.x, -h.y), 0.0);
    return s / 8.0;
}

@fragment
fn fs_up(in: VOut) -> @location(0) vec4f {
    let h = 0.5 / vec2f(textureDimensions(src));
    var s = textureSampleLevel(src, smp, in.uv + vec2f(-2.0 * h.x, 0.0), 0.0);
    s += textureSampleLevel(src, smp, in.uv + vec2f(2.0 * h.x, 0.0), 0.0);
    s += textureSampleLevel(src, smp, in.uv + vec2f(0.0, -2.0 * h.y), 0.0);
    s += textureSampleLevel(src, smp, in.uv + vec2f(0.0, 2.0 * h.y), 0.0);
    s += textureSampleLevel(src, smp, in.uv + h, 0.0) * 2.0;
    s += textureSampleLevel(src, smp, in.uv - h, 0.0) * 2.0;
    s += textureSampleLevel(src, smp, in.uv + vec2f(h.x, -h.y), 0.0) * 2.0;
    s += textureSampleLevel(src, smp, in.uv - vec2f(h.x, -h.y), 0.0) * 2.0;
    return s / 12.0;
}

struct Panel {
    rect: vec4f,   // x0, y0, x1, y1 in pixels
    screen: vec2f, // target size in pixels
    radius: f32,   // pixels
    liquid: f32,   // 1 for Liquid Glass
}

@group(0) @binding(0) var blurred: texture_2d<f32>;
@group(0) @binding(1) var deepest: texture_2d<f32>;
@group(0) @binding(2) var lin: sampler;
@group(1) @binding(0) var<uniform> g: Panel;

// the rect grown by 1 px for the anti-aliased edge, as a triangle strip
@vertex
fn vs_panel(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let c = vec2f(f32(i & 1u), f32(i >> 1u));
    let p = mix(g.rect.xy - 1.0, g.rect.zw + 1.0, c);
    return vec4f(p / g.screen * vec2f(2.0, -2.0) + vec2f(-1.0, 1.0), 0.0, 1.0);
}

// signed distance to a rounded rect centred at 0 (negative inside)
fn sd(p: vec2f, half: vec2f, r: f32) -> f32 {
    let q = abs(p) - half + r;
    return length(max(q, vec2f(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

// Liquid Glass: how far samples move inward at the edge, and over what depth (pixels)
const REFRACT: f32 = 10.0;
const REFRACT_BAND: f32 = 14.0;

@fragment
fn fs_panel(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let c = (g.rect.xy + g.rect.zw) * 0.5;
    let half = (g.rect.zw - g.rect.xy) * 0.5;
    let r = min(g.radius, min(half.x, half.y));
    let p = pos.xy - c;
    let d = sd(p, half, r);
    let cover = clamp(0.5 - d, 0.0, 1.0);
    if g.liquid < 0.5 {
        let col = textureSampleLevel(blurred, lin, pos.xy / g.screen, 0.0).rgb;
        return vec4f(col * cover, cover);
    }
    // outward normal of the distance field
    let e = vec2f(0.5, 0.0);
    let n = normalize(vec2f(sd(p + e.xy, half, r) - sd(p - e.xy, half, r),
                            sd(p + e.yx, half, r) - sd(p - e.yx, half, r)) + 1e-6);
    let band = min(REFRACT_BAND, min(half.x, half.y));
    let t = clamp(1.0 + d / band, 0.0, 1.0); // 1 at the edge, 0 at the band's depth
    let at = pos.xy - n * REFRACT * t * t;
    var col = textureSampleLevel(blurred, lin, at / g.screen, 0.0).rgb;
    // the backdrop's mean over the panel: darker over bright backdrops, so text stays legible
    var mean = vec3f(0.0);
    for (var i = -2; i <= 2; i++) {
        let q = c + vec2f(f32(i) * half.x * 0.4, 0.0);
        mean += textureSampleLevel(deepest, lin, q / g.screen, 0.0).rgb;
    }
    let lum = dot(mean / 5.0, vec3f(0.299, 0.587, 0.114));
    col *= 1.0 - mix(0.2, 0.55, lum);
    // specular rim, lit from the top left, within ~2 px of the edge
    let rim = pow(t, 24.0) * max(dot(n, normalize(vec2f(-1.0, -1.0))), 0.0) * 0.5;
    return vec4f((col + rim) * cover, cover);
}
";

/// The pyramid's format: 16-bit float, so blurred gradients do not band.
const LEVEL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Pyramid depth per strength: subtle is a light blur (1/8 size), full a strong one.
fn levels(glass: Glass) -> usize {
    match glass {
        Glass::Off => 0,
        Glass::Subtle => 3,
        Glass::Full => 5,
    }
}

pub struct GlassRenderer {
    blur_layout: wgpu::BindGroupLayout,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    panel_layouts: [wgpu::BindGroupLayout; 2],
    panel: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    /// Pyramid levels 1.. (half size each), with their views; rebuilt on resize.
    pyramid: Vec<wgpu::TextureView>,
    pyramid_size: Size,
}

impl GlassRenderer {
    /// `format`: the target egui draws into.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("glass"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let layout = |entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries,
            })
        };
        let blur_layout = layout(&[texture(0), sampler(1)]);
        let panel_layouts = [
            layout(&[texture(0), texture(1), sampler(2)]),
            layout(&[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }]),
        ];
        let pipeline = |layouts: &[Option<&wgpu::BindGroupLayout>],
                        vs: &str,
                        fs: &str,
                        format,
                        blend,
                        topology| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: layouts,
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let list = wgpu::PrimitiveTopology::TriangleList;
        let down = pipeline(
            &[Some(&blur_layout)],
            "vs_full",
            "fs_down",
            LEVEL_FORMAT,
            None,
            list,
        );
        let up = pipeline(
            &[Some(&blur_layout)],
            "vs_full",
            "fs_up",
            LEVEL_FORMAT,
            None,
            list,
        );
        // premultiplied alpha, like egui's own pipeline
        let panel = pipeline(
            &[Some(&panel_layouts[0]), Some(&panel_layouts[1])],
            "vs_panel",
            "fs_panel",
            format,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            wgpu::PrimitiveTopology::TriangleStrip,
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            blur_layout,
            down,
            up,
            panel_layouts,
            panel,
            sampler,
            pyramid: Vec::new(),
            pyramid_size: Size {
                width: 0,
                height: 0,
            },
        }
    }

    fn source(&self, device: &wgpu::Device, view: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.blur_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    /// Records the blur of `world` (of `size`) into `encoder` and returns what the panel
    /// callbacks of this frame draw with; `None` when glass is off.
    pub fn blur(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        world: &wgpu::TextureView,
        size: Size,
        glass: Glass,
    ) -> Option<GlassFrame> {
        let n = levels(glass);
        if n == 0 {
            return None;
        }
        if self.pyramid_size != size {
            self.pyramid = (1..=levels(Glass::Full))
                .map(|i| {
                    device
                        .create_texture(&wgpu::TextureDescriptor {
                            label: Some("glass pyramid"),
                            size: wgpu::Extent3d {
                                width: (size.width >> i).max(1),
                                height: (size.height >> i).max(1),
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: LEVEL_FORMAT,
                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                                | wgpu::TextureUsages::TEXTURE_BINDING,
                            view_formats: &[],
                        })
                        .create_view(&Default::default())
                })
                .collect();
            self.pyramid_size = size;
        }
        let mut pass = |pipeline: &wgpu::RenderPipeline, src: &wgpu::TextureView, dst| {
            let source = self.source(device, src);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("glass blur"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: dst,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &source, &[]);
            pass.draw(0..3, 0..1);
        };
        // down to level n, then back up to level 1; level n keeps the deepest average
        pass(&self.down, world, &self.pyramid[0]);
        for i in 1..n {
            pass(&self.down, &self.pyramid[i - 1], &self.pyramid[i]);
        }
        for i in (1..n).rev() {
            pass(&self.up, &self.pyramid[i], &self.pyramid[i - 1]);
        }
        let textures = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.panel_layouts[0],
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.pyramid[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.pyramid[n - 1]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        Some(GlassFrame {
            pipeline: self.panel.clone(),
            uniform_layout: self.panel_layouts[1].clone(),
            textures,
            draws: Vec::new(),
        })
    }
}

/// One frame's glass resources, kept in egui-wgpu's `callback_resources`.
pub struct GlassFrame {
    pipeline: wgpu::RenderPipeline,
    uniform_layout: wgpu::BindGroupLayout,
    textures: wgpu::BindGroup,
    /// One uniform bind group per panel, in paint order.
    draws: Vec<wgpu::BindGroup>,
}

/// A glass backdrop for an egui shape: `rect` and `radius` in points, like the panel it
/// sits under. `liquid`: Liquid Glass (the island at full strength).
pub fn shape(rect: egui::Rect, radius: f32, liquid: bool) -> egui::Shape {
    egui::Shape::Callback(egui_wgpu::Callback::new_paint_callback(
        rect,
        Backdrop {
            rect,
            radius,
            liquid,
            slot: AtomicUsize::new(usize::MAX),
        },
    ))
}

struct Backdrop {
    rect: egui::Rect,
    radius: f32,
    liquid: bool,
    /// Index of this backdrop's bind group in `GlassFrame::draws` (set in `prepare`).
    slot: AtomicUsize,
}

impl CallbackTrait for Backdrop {
    fn prepare(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        // glass off: no frame, nothing drawn
        let Some(frame) = resources.get_mut::<GlassFrame>() else {
            return Vec::new();
        };
        let ppp = screen.pixels_per_point;
        let [w, h] = screen.size_in_pixels;
        let r = self.rect;
        let values = [
            r.min.x * ppp,
            r.min.y * ppp,
            r.max.x * ppp,
            r.max.y * ppp,
            w as f32,
            h as f32,
            self.radius * ppp,
            self.liquid as u32 as f32,
        ];
        // ponytail: one small buffer per panel per frame; a shared buffer with dynamic
        // offsets if panels ever number in the hundreds
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("glass panel"),
            contents: &values
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>(),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &frame.uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        self.slot.store(frame.draws.len(), Ordering::Relaxed);
        frame.draws.push(group);
        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        let Some(frame) = resources.get::<GlassFrame>() else {
            return;
        };
        let Some(group) = frame.draws.get(self.slot.load(Ordering::Relaxed)) else {
            return;
        };
        // egui sets the viewport to the callback rect; the shader works in target pixels
        let [w, h] = info.screen_size_px;
        pass.set_viewport(0.0, 0.0, w as f32, h as f32, 0.0, 1.0);
        pass.set_pipeline(&frame.pipeline);
        pass.set_bind_group(0, &frame.textures, &[]);
        pass.set_bind_group(1, group, &[]);
        pass.draw(0..4, 0..1);
    }
}
