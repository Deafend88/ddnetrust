//! `ddnet-rs`: the graphical client. So far (M6.2) a window that shows the game skin
//! and the default tee skin loaded from the DDNet data, drawn through the offscreen
//! world target. F12 saves a screenshot into our own directory.
//!
//! Usage: ddnet-rs [--data-dir PATH] [--vsync]

use std::sync::Arc;

use ddnet_rs::assets;
use ddnet_rs::gfx::batch::{Batch, QuadItem, TextureId};
use ddnet_rs::gfx::{self, Gfx, Size, block_on};
use ddnet_rs::image::Image;
use ddnet_rs::storage::{self, Storage};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

struct Running {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gfx: Gfx,
    /// Loaded images with their size, drawn side by side.
    shown: Vec<(TextureId, u32, u32)>,
}

struct App {
    /// C++ `gfx_vsync` (default 0).
    vsync: bool,
    storage: Storage,
    running: Option<Running>,
    error: Option<String>,
}

impl App {
    fn start(&self, event_loop: &ActiveEventLoop) -> Result<Running, String> {
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_title("DDNet-rs"))
                .map_err(|e| format!("window: {e}"))?,
        );
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("surface: {e}"))?;
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|e| format!("no graphics adapter: {e}"))?;
        let (device, queue) = block_on(adapter.request_device(&Default::default()))
            .map_err(|e| format!("device: {e}"))?;
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or("the surface does not fit this adapter")?;
        config.format = gfx::pick_format(&surface.get_capabilities(&adapter).formats)
            .ok_or("no non-sRGB 8-bit surface format")?;
        config.present_mode = if self.vsync {
            wgpu::PresentMode::AutoVsync
        } else {
            wgpu::PresentMode::AutoNoVsync
        };
        surface.configure(&device, &config);
        let mut gfx = Gfx::new(
            device,
            queue,
            config.format,
            Size {
                width: config.width,
                height: config.height,
            },
        );
        let images: Vec<Image> = [
            assets::load_asset(&self.storage, assets::GAME, "default").image,
            assets::load_skin(&self.storage, "default"),
        ]
        .into_iter()
        .flatten()
        .collect();
        if images.len() < 2 {
            eprintln!("game.png or skins/default.png missing: check the data dir");
        }
        let shown = images
            .iter()
            .map(|i| {
                (
                    gfx.create_texture(i.width, i.height, &i.to_rgba()),
                    i.width,
                    i.height,
                )
            })
            .collect();
        Ok(Running {
            window,
            surface,
            config,
            gfx,
            shown,
        })
    }
}

impl Running {
    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return; // minimized
        }
        (self.config.width, self.config.height) = (width, height);
        self.surface.configure(&self.gfx.device, &self.config);
        self.gfx.resize(Size { width, height });
    }

    fn frame(&mut self) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.gfx.device, &self.config);
                return;
            }
            _ => return,
        };
        let mut batch = Batch::default();
        let (w, h) = (self.config.width as f32, self.config.height as f32);
        batch.clear(0.5, 0.5, 0.5);
        batch.map_screen(0.0, 0.0, w, h);
        let mut x = 0.0;
        let slot = w / self.shown.len().max(1) as f32;
        for &(tex, tw, th) in &self.shown {
            let scale = (slot / tw as f32).min(h / th as f32);
            batch.texture_set(Some(tex));
            batch.quads_begin();
            batch.quads_draw_tl(&[QuadItem {
                x,
                y: 0.0,
                w: tw as f32 * scale,
                h: th as f32 * scale,
            }]);
            x += slot;
        }
        let world = self.gfx.world_view();
        self.gfx.render(&mut batch, &world);
        self.gfx
            .composite(&frame.texture.create_view(&Default::default()));
        self.window.pre_present_notify();
        self.gfx.queue.present(frame);
    }

    fn screenshot(&self) {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let Some(dir) = storage::save_dir().map(|d| d.join("screenshots")) else {
            eprintln!("screenshot failed: no directory to save to");
            return;
        };
        let path = dir.join(format!("screenshot_{secs}.png"));
        match std::fs::create_dir_all(&dir).and_then(|()| self.gfx.screenshot(&path)) {
            Ok(()) => println!("saved {}", path.display()),
            Err(e) => eprintln!("screenshot failed: {e}"),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_some() {
            return;
        }
        match self.start(event_loop) {
            Ok(r) => self.running = Some(r),
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(r) = &mut self.running else { return };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => r.resize(size.width, size.height),
            WindowEvent::RedrawRequested => r.frame(),
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(KeyCode::F12),
                        state: ElementState::Pressed,
                        repeat: false,
                        ..
                    },
                ..
            } => r.screenshot(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        // C++ gfx_refresh_rate 0: render as often as possible
        if let Some(r) = &self.running {
            r.window.request_redraw();
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let data_dir = args
        .iter()
        .position(|a| a == "--data-dir")
        .and_then(|i| args.get(i + 1))
        .map(std::path::PathBuf::from);
    let storage = match Storage::new(data_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("storage: {e}");
            std::process::exit(1);
        }
    };
    for p in &storage.paths {
        println!("storage path: '{}'", p.display());
    }
    if storage.data_dir.is_none() {
        eprintln!("no DDNet data dir found: pass --data-dir or set DDNET_DATA_DIR");
    }
    let mut app = App {
        vsync: args.iter().any(|a| a == "--vsync"),
        storage,
        running: None,
        error: None,
    };
    let event_loop = match EventLoop::new() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot start: {e}");
            std::process::exit(1);
        }
    };
    event_loop.set_control_flow(ControlFlow::Poll);
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("event loop: {e}");
    }
    if let Some(e) = app.error {
        eprintln!("cannot start: {e}");
        std::process::exit(1);
    }
}
