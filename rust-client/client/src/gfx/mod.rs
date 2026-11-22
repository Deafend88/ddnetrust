//! GPU side of the renderer: wgpu device, textures, one pipeline per blend mode, the
//! offscreen world target and screenshot readback.
//!
//! Colours stay in gamma space like the C++ OpenGL backends: render targets and
//! textures are non-sRGB `Unorm` formats, so blending gives the same numbers.

pub mod batch;

use std::sync::Arc;
use std::sync::mpsc;
use std::task::{Context, Poll, Wake, Waker};

use batch::{Batch, Blend, TextureId, Vertex, Wrap};

const SHADER: &str = r"
struct Out {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
    @location(1) color: vec4f,
}

@vertex
fn vs(@location(0) pos: vec2f, @location(1) uv: vec2f, @location(2) color: vec4f) -> Out {
    return Out(vec4f(pos, 0.0, 1.0), uv, color);
}

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;

@fragment
fn fs(in: Out) -> @location(0) vec4f {
    return textureSample(tex, smp, in.uv) * in.color;
}
";

/// Runs a future to completion on this thread (wgpu's adapter/device requests).
pub fn block_on<F: Future>(fut: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut fut = std::pin::pin!(fut);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::park(),
        }
    }
}

struct TextureEntry {
    texture: wgpu::Texture,
    /// Bind groups for `Wrap::Repeat` and `Wrap::Clamp`.
    bind_groups: [wgpu::BindGroup; 2],
}

/// A size in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

pub struct Gfx {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// The format of every render target (window surface and world target).
    pub format: wgpu::TextureFormat,
    layout: wgpu::BindGroupLayout,
    pipelines: [wgpu::RenderPipeline; 3],
    textures: Vec<TextureEntry>,
    white: TextureId,
    vertex_buffer: Option<wgpu::Buffer>,
    index_buffer: Option<wgpu::Buffer>,
    /// The world is drawn here, then copied to the window (and read for screenshots).
    pub world: TextureId,
}

/// Picks a non-sRGB format the target supports, so blending matches OpenGL.
pub fn pick_format(formats: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    [
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8Unorm,
    ]
    .into_iter()
    .find(|f| formats.contains(f))
}

impl Gfx {
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        size: Size,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let blend = |factor| wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::SrcAlpha,
            dst_factor: factor,
            operation: wgpu::BlendOperation::Add,
        };
        let pipelines = [
            None,
            Some(blend(wgpu::BlendFactor::OneMinusSrcAlpha)),
            Some(blend(wgpu::BlendFactor::One)),
        ]
        .map(|b| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: Vertex::SIZE as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x2, 1 => Float32x2, 2 => Unorm8x4
                        ],
                    })],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: b.map(|c| wgpu::BlendState { color: c, alpha: c }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        });
        let mut gfx = Self {
            device,
            queue,
            format,
            layout,
            pipelines,
            textures: Vec::new(),
            white: TextureId(0),
            vertex_buffer: None,
            index_buffer: None,
            world: TextureId(0),
        };
        gfx.white = gfx.create_texture(1, 1, &[255; 4]);
        gfx.world = gfx.create_target(size);
        gfx
    }

    /// Creates a device without a window (tests, offscreen use). `None` without an adapter.
    pub fn headless(size: Size) -> Option<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
        let (device, queue) =
            block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        Some(Self::new(
            device,
            queue,
            wgpu::TextureFormat::Bgra8Unorm,
            size,
        ))
    }

    fn sampler(&self, wrap: Wrap, filter: wgpu::FilterMode) -> wgpu::Sampler {
        let mode = match wrap {
            Wrap::Repeat => wgpu::AddressMode::Repeat,
            Wrap::Clamp => wgpu::AddressMode::ClampToEdge,
        };
        self.device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: mode,
            address_mode_v: mode,
            mag_filter: filter,
            min_filter: filter,
            ..Default::default()
        })
    }

    fn add_texture(&mut self, texture: wgpu::Texture, filter: wgpu::FilterMode) -> TextureId {
        let view = texture.create_view(&Default::default());
        let bind_groups = [Wrap::Repeat, Wrap::Clamp].map(|wrap| {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler(wrap, filter)),
                    },
                ],
            })
        });
        self.textures.push(TextureEntry {
            texture,
            bind_groups,
        });
        TextureId(self.textures.len() - 1)
    }

    /// Uploads an RGBA8 image (linear filtering, like C++ textures without mipmaps).
    pub fn create_texture(&mut self, width: u32, height: u32, rgba: &[u8]) -> TextureId {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            texture.as_image_copy(),
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: None,
            },
            size,
        );
        self.add_texture(texture, wgpu::FilterMode::Linear)
    }

    /// A render target in `self.format`, sampled 1:1 (nearest) when composited.
    fn create_target(&mut self, size: Size) -> TextureId {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: size.width.max(1),
                height: size.height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        self.add_texture(texture, wgpu::FilterMode::Nearest)
    }

    /// Resizes the world target (window resize).
    pub fn resize(&mut self, size: Size) {
        let id = self.create_target(size);
        self.textures.swap(self.world.0, id.0);
        self.textures.pop();
    }

    pub fn world_size(&self) -> Size {
        let t = &self.textures[self.world.0].texture;
        Size {
            width: t.width(),
            height: t.height(),
        }
    }

    pub fn world_view(&self) -> wgpu::TextureView {
        self.textures[self.world.0]
            .texture
            .create_view(&Default::default())
    }

    /// Draws `batch` into `target` and clears it for the next frame.
    pub fn render(&mut self, batch: &mut Batch, target: &wgpu::TextureView) {
        let mut bytes = Vec::with_capacity(batch.vertices.len() * Vertex::SIZE);
        for v in &batch.vertices {
            v.write(&mut bytes);
        }
        let quads = batch.vertices.len() / 4;
        if !bytes.is_empty() {
            self.upload(&bytes, quads);
        }
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let load = match batch.clear {
                Some([r, g, b]) => wgpu::LoadOp::Clear(wgpu::Color {
                    r: r.into(),
                    g: g.into(),
                    b: b.into(),
                    a: 0.0, // C++ Clear: alpha 0
                }),
                None => wgpu::LoadOp::Load,
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let (Some(vb), Some(ib)) = (&self.vertex_buffer, &self.index_buffer)
                && quads > 0
            {
                pass.set_vertex_buffer(0, vb.slice(..));
                pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                for d in &batch.draws {
                    let pipeline = match d.blend {
                        Blend::None => 0,
                        Blend::Normal => 1,
                        Blend::Additive => 2,
                    };
                    let tex = &self.textures[d.texture.unwrap_or(self.white).0];
                    let wrap = (d.wrap == Wrap::Clamp) as usize;
                    pass.set_pipeline(&self.pipelines[pipeline]);
                    pass.set_bind_group(0, &tex.bind_groups[wrap], &[]);
                    pass.draw_indexed(d.first * 6..(d.first + d.count) * 6, 0, 0..1);
                }
            }
        }
        self.queue.submit([encoder.finish()]);
        *batch = Batch::default();
    }

    /// Writes the vertices and makes the quad index buffer large enough.
    fn upload(&mut self, bytes: &[u8], quads: usize) {
        let usage = wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST;
        if self
            .vertex_buffer
            .as_ref()
            .is_none_or(|b| b.size() < bytes.len() as u64)
        {
            self.vertex_buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: (bytes.len() as u64).next_power_of_two(),
                usage,
                mapped_at_creation: false,
            }));
        }
        self.queue
            .write_buffer(self.vertex_buffer.as_ref().unwrap(), 0, bytes);
        let index_bytes = quads as u64 * 6 * 4;
        if self
            .index_buffer
            .as_ref()
            .is_none_or(|b| b.size() < index_bytes)
        {
            let capacity = (quads as u32).next_power_of_two();
            let indices: Vec<u8> = (0..capacity)
                .flat_map(|q| [0, 1, 2, 0, 2, 3].map(|i| q * 4 + i))
                .flat_map(u32::to_le_bytes)
                .collect();
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: indices.len() as u64,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.queue.write_buffer(&buffer, 0, &indices);
            self.index_buffer = Some(buffer);
        }
    }

    /// Draws the world target 1:1 into `target` (the window).
    pub fn composite(&mut self, target: &wgpu::TextureView) {
        let mut b = Batch::default();
        b.map_screen(0.0, 0.0, 1.0, 1.0);
        b.texture_set(Some(self.world));
        b.blend(Blend::None);
        b.wrap(Wrap::Clamp);
        b.quads_begin();
        b.quads_draw_tl(&[batch::QuadItem {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        }]);
        self.render(&mut b, target);
    }

    /// Reads the world target back as tightly packed RGBA8 rows.
    pub fn read_world(&self) -> Vec<u8> {
        let Size { width, height } = self.world_size();
        let row = (4 * width).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            self.textures[self.world.0].texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        let (tx, rx) = mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let mapped = self
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .is_ok()
            && rx.recv().is_ok_and(|r| r.is_ok());
        let data = match buffer.slice(..).get_mapped_range() {
            Ok(data) if mapped => data,
            _ => return vec![0; (4 * width * height) as usize],
        };
        let bgra = self.format == wgpu::TextureFormat::Bgra8Unorm;
        let mut out = Vec::with_capacity((4 * width * height) as usize);
        for line in data.chunks(row as usize) {
            for px in line[..(4 * width) as usize].chunks(4) {
                let p = if bgra {
                    [px[2], px[1], px[0], px[3]]
                } else {
                    [px[0], px[1], px[2], px[3]]
                };
                out.extend_from_slice(&p);
            }
        }
        out
    }

    /// Saves the world target as an RGB PNG (C++ `screenshot`).
    pub fn screenshot(&self, path: &std::path::Path) -> std::io::Result<()> {
        let Size { width, height } = self.world_size();
        let rgb: Vec<u8> = self
            .read_world()
            .chunks(4)
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        let mut encoder = png::Encoder::new(std::fs::File::create(path)?, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .and_then(|mut w| w.write_image_data(&rgb))
            .map_err(std::io::Error::other)
    }
}

/// A 4x4 RGBA texture with a different opaque colour per texel (row-major).
pub fn pattern_texture() -> Vec<u8> {
    (0..16u8)
        .flat_map(|i| [i * 16, 255 - i * 16, (i % 4) * 80, 255])
        .collect()
}

/// The 6.1 test scene in a 64x32 screen: blend modes, a 1:1 texture, a texture subset,
/// repeat and clamp wrapping, rotation, a freeform quad and corner colours.
pub fn test_pattern(b: &mut Batch, texture: TextureId) {
    use batch::{FreeformItem, QuadItem};
    let q = |x: f32, y: f32, w: f32, h: f32| QuadItem { x, y, w, h };
    b.clear(0.0, 0.0, 1.0);
    b.map_screen(0.0, 0.0, 64.0, 32.0);
    b.quads_begin();
    for (i, blend) in [Blend::None, Blend::Normal, Blend::Additive]
        .into_iter()
        .enumerate()
    {
        b.blend(blend);
        b.set_color(if i == 2 {
            [0.0, 1.0, 0.0, 0.5]
        } else {
            [1.0, 0.0, 0.0, 0.5]
        });
        b.quads_draw_tl(&[q(8.0 * i as f32, 0.0, 8.0, 8.0)]);
    }
    b.blend(Blend::None);
    b.texture_set(Some(texture));
    b.quads_begin();
    b.quads_draw_tl(&[q(24.0, 0.0, 4.0, 4.0)]);
    b.set_subset(0.5, 0.0, 1.0, 0.5);
    b.quads_draw_tl(&[q(28.0, 0.0, 2.0, 2.0)]);
    for (i, wrap) in [Wrap::Repeat, Wrap::Clamp].into_iter().enumerate() {
        b.wrap(wrap);
        b.set_subset(0.0, 0.0, 2.0, 2.0);
        b.quads_draw_tl(&[q(48.0, 8.0 + 8.0 * i as f32, 8.0, 8.0)]);
    }
    b.texture_set(None);
    b.quads_begin();
    b.set_color([1.0, 1.0, 0.0, 1.0]);
    b.set_rotation(std::f32::consts::FRAC_PI_2);
    b.quads_draw(&[q(36.0, 8.0, 8.0, 2.0)]);
    b.quads_begin();
    b.set_color([1.0, 0.0, 1.0, 1.0]);
    b.quads_draw_freeform(&[FreeformItem([
        [40.0, 0.0],
        [48.0, 0.0],
        [40.0, 8.0],
        [48.0, 8.0],
    ])]);
    b.set_color4(
        [1.0, 0.0, 0.0, 1.0],
        [0.0, 1.0, 0.0, 1.0],
        [0.0, 0.0, 0.0, 1.0],
        [1.0, 1.0, 1.0, 1.0],
    );
    b.quads_draw_tl(&[q(0.0, 16.0, 16.0, 16.0)]);
}
