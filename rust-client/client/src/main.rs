//! `ddnet-rs`: the graphical client. M6.1: a window that draws the renderer's test
//! pattern through the offscreen world target. F12 saves a screenshot.
//!
//! Usage: ddnet-rs [--vsync]

use std::sync::Arc;

use ddnet_rs::gfx::batch::Batch;
use ddnet_rs::gfx::{self, Gfx, Size, block_on};
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
    pattern: gfx::batch::TextureId,
}

#[derive(Default)]
struct App {
    /// C++ `gfx_vsync` (default 0).
    vsync: bool,
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
        let pattern = gfx.create_texture(4, 4, &gfx::pattern_texture());
        Ok(Running {
            window,
            surface,
            config,
            gfx,
            pattern,
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
        gfx::test_pattern(&mut batch, self.pattern);
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
        // ponytail: saved in the working directory until 6.2 adds our own storage dir
        let dir = std::path::Path::new("screenshots");
        let path = dir.join(format!("screenshot_{secs}.png"));
        match std::fs::create_dir_all(dir).and_then(|()| self.gfx.screenshot(&path)) {
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
    let mut app = App {
        vsync: std::env::args().skip(1).any(|a| a == "--vsync"),
        ..App::default()
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
