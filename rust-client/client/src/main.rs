//! `ddnet-rs`: the graphical client. So far (M6.4) it connects to a local server and
//! draws the map's layers and the tees around the local tee (or a fixed view),
//! through the offscreen world target. F12 saves a screenshot into our own directory.
//!
//! Usage: ddnet-rs <server ip:port> [--data-dir PATH] [--vsync] [--size WxH] [--view X,Y]
//!        [--follow CLIENT_ID] [--zoom Z] [--screenshot-after SECONDS]

use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::Instant;

use ddnet::map::Map;
use ddnet::net;
use ddnet::net::client::{Client, PlayerInfo, State};
use ddnet::physics::collision::Collision;
use ddnet::predict::{Predictor, Rendered};
use ddnet::protocol::generated as g;
use ddnet_rs::assets;
use ddnet_rs::gfx::batch::Batch;
use ddnet_rs::gfx::{self, Gfx, Size, block_on};
use ddnet_rs::render::map::{MapRender, View, hsl_packed_to_rgb};
use ddnet_rs::render::players::Players;
use ddnet_rs::storage::{self, Storage};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

/// C++ `cl_background_color` default (packed HSL).
const BACKGROUND_COLOR: u32 = 128;
/// C++ `SERVER_TICK_SPEED`: nanoseconds per tick.
const NANOS_PER_TICK: i64 = 1_000_000_000 / 50;

struct Options {
    server: SocketAddr,
    data_dir: Option<std::path::PathBuf>,
    /// C++ `gfx_vsync` (default 0).
    vsync: bool,
    /// Window size in pixels (default: the window system's).
    size: Option<(u32, u32)>,
    /// A fixed camera center instead of the local tee.
    view: Option<(f32, f32)>,
    /// Center on this player instead of the local tee (screenshot comparisons).
    follow: Option<i32>,
    zoom: f32,
    /// Save one screenshot this many seconds after the map shows, then quit.
    screenshot_after: Option<f64>,
}

struct Running {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gfx: Gfx,
    map: Option<(MapRender, &'static Collision)>,
    predictor: Option<Predictor<'static>>,
    players: Players,
    map_shown_at: Option<Instant>,
}

struct App {
    options: Options,
    storage: Storage,
    socket: UdpSocket,
    client: Client,
    start: Instant,
    running: Option<Running>,
    /// C++ `m_LocalCharacterPos` (or the followed tee's): kept while the tee is gone.
    center: Option<(f32, f32)>,
    error: Option<String>,
}

impl App {
    fn start(&self, event_loop: &ActiveEventLoop) -> Result<Running, String> {
        let mut attributes = Window::default_attributes().with_title("DDNet-rs");
        if let Some((w, h)) = self.options.size {
            attributes = attributes.with_inner_size(winit::dpi::PhysicalSize::new(w, h));
        }
        let window = Arc::new(
            event_loop
                .create_window(attributes)
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
        config.present_mode = if self.options.vsync {
            wgpu::PresentMode::AutoVsync
        } else {
            wgpu::PresentMode::AutoNoVsync
        };
        surface.configure(&device, &config);
        let mut gfx = Gfx::new(device, queue, config.format, size_of(&config));
        let players = Players::new(&mut gfx, &self.storage);
        Ok(Running {
            window,
            surface,
            config,
            gfx,
            map: None,
            predictor: None,
            players,
            map_shown_at: None,
        })
    }

    /// The camera center (C++ `CCamera` without dyncam): the fixed view, else the
    /// rendered position of the followed or local tee (C++ `m_LocalCharacterPos`: the
    /// interpolated, or when predicting the predicted, position), else the last one.
    fn camera(&mut self, rendered: &[Rendered]) -> Option<(f32, f32)> {
        if let Some(v) = self.options.view {
            return Some(v);
        }
        let id = self.options.follow.or(self.client.local_id());
        if let Some(r) = rendered.iter().find(|r| Some(r.id) == id) {
            self.center = Some((r.pos.x, r.pos.y));
        }
        self.center
    }

    /// C++: `CEnvelopeState::Update` while playing: predicted tick and intra tick since
    /// the round start.
    fn envelope_time(&self) -> i64 {
        let Some(info) = self
            .client
            .snapshot()
            .iter()
            .find(|it| it.type_id == g::NETOBJTYPE_GAMEINFO)
        else {
            return 0;
        };
        let info = g::ObjGameInfo::from_ints(&info.data);
        let tick = (self.client.pred_tick() - 1 - info.round_start_tick) as i64;
        (self.client.pred_intra_tick as f64 * NANOS_PER_TICK as f64) as i64 + tick * NANOS_PER_TICK
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let now = net::now(self.start);
        if let Err(e) = net::pump(&self.socket, &mut self.client, now) {
            self.error = Some(format!("network: {e}"));
            return event_loop.exit();
        }
        if self.client.state() == State::Offline {
            self.error = Some(format!("disconnected: '{}'", self.client.conn.error()));
            return event_loop.exit();
        }
        let Some(r) = &mut self.running else { return };
        if r.map.is_none()
            && self.client.state() == State::Online
            && let Some(m) = self
                .client
                .map()
                .and_then(|m| Map::load(m.data.clone()).ok())
        {
            let images = assets::load_map_images(&self.storage, &m);
            // ponytail: one map per run; leaked so the predictor can borrow it, free it
            // when map changes are supported
            let collision: &'static Collision = Box::leak(Box::new(Collision::new(&m)));
            r.map = Some((MapRender::new(&mut r.gfx, &m, &images), collision));
            r.predictor = Some(Predictor::new(collision));
            r.map_shown_at = Some(Instant::now());
        }
        // C++: CGameClient::OnNewSnapshot/OnPredict, then UpdateRenderedCharacters
        let (rendered, local_freeze_end) = match &mut r.predictor {
            Some(p) => {
                p.update(&mut self.client);
                (p.rendered(&self.client), p.predicted_freeze_end())
            }
            None => (Vec::new(), None),
        };
        let camera = self.camera(&rendered);
        let envelope_time = self.envelope_time();
        let r = self.running.as_mut().unwrap();
        let frame = match r.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                r.surface.configure(&r.gfx.device, &r.config);
                return;
            }
            _ => return,
        };
        let size = size_of(&r.config);
        let mut batch = Batch::default();
        let [cr, cg, cb] = hsl_packed_to_rgb(BACKGROUND_COLOR);
        batch.clear(cr, cg, cb);
        if let (Some((map, collision)), Some(center)) = (&r.map, camera) {
            let view = View {
                center,
                zoom: self.options.zoom,
                width: size.width,
                height: size.height,
                envelope_time,
            };
            map.render_background(&mut batch, &view);
            r.players.render(
                &mut r.gfx,
                &self.storage,
                &mut batch,
                &self.client,
                collision,
                &rendered,
                local_freeze_end,
            );
            map.render_foreground(&mut batch, &view);
        }
        let world = r.gfx.world_view();
        r.gfx.render(&mut batch, &world, size);
        r.gfx
            .composite(&frame.texture.create_view(&Default::default()), size);
        r.window.pre_present_notify();
        r.gfx.queue.present(frame);
        if let (Some(after), Some(shown)) = (self.options.screenshot_after, r.map_shown_at)
            && shown.elapsed().as_secs_f64() >= after
        {
            // for reproducing the view with --view
            if let Some((x, y)) = camera {
                println!("camera center {x} {y}");
            }
            screenshot(&r.gfx);
            event_loop.exit();
        }
    }
}

fn size_of(config: &wgpu::SurfaceConfiguration) -> Size {
    Size {
        width: config.width,
        height: config.height,
    }
}

fn screenshot(gfx: &Gfx) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let Some(dir) = storage::save_dir().map(|d| d.join("screenshots")) else {
        eprintln!("screenshot failed: no directory to save to");
        return;
    };
    let path = dir.join(format!("screenshot_{secs}.png"));
    match std::fs::create_dir_all(&dir).and_then(|()| gfx.screenshot(&path)) {
        Ok(()) => println!("saved {}", path.display()),
        Err(e) => eprintln!("screenshot failed: {e}"),
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
        match event {
            WindowEvent::CloseRequested => {
                self.client.disconnect(None, net::now(self.start));
                let _ = net::pump(&self.socket, &mut self.client, net::now(self.start));
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(r) = &mut self.running
                    && size.width > 0
                    && size.height > 0
                {
                    (r.config.width, r.config.height) = (size.width, size.height);
                    r.surface.configure(&r.gfx.device, &r.config);
                    r.gfx.resize(size_of(&r.config));
                }
            }
            WindowEvent::RedrawRequested => self.frame(event_loop),
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(KeyCode::F12),
                        state: ElementState::Pressed,
                        repeat: false,
                        ..
                    },
                ..
            } => {
                if let Some(r) = &self.running {
                    screenshot(&r.gfx);
                }
            }
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

fn parse_args() -> Result<Options, String> {
    let mut args = std::env::args().skip(1);
    let server: SocketAddr = args
        .next()
        .and_then(|a| a.parse().ok())
        .ok_or("usage: ddnet-rs <server ip:port> [--data-dir PATH] [--vsync] [--size WxH] [--view X,Y] [--follow CLIENT_ID] [--zoom Z] [--screenshot-after SECONDS]")?;
    if !server.ip().is_loopback() {
        return Err(format!(
            "refusing {server}: only local (loopback) servers are allowed for now"
        ));
    }
    let mut o = Options {
        server,
        data_dir: None,
        vsync: false,
        size: None,
        view: None,
        follow: None,
        zoom: 1.0,
        screenshot_after: None,
    };
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--data-dir" => o.data_dir = Some(value()?.into()),
            "--vsync" => o.vsync = true,
            "--view" => {
                let v = value()?;
                let (x, y) = v.split_once(',').ok_or("--view X,Y")?;
                o.view = Some((
                    x.parse().map_err(|_| "--view X,Y")?,
                    y.parse().map_err(|_| "--view X,Y")?,
                ));
            }
            "--zoom" => o.zoom = value()?.parse().map_err(|_| "--zoom Z")?,
            "--follow" => o.follow = Some(value()?.parse().map_err(|_| "--follow CLIENT_ID")?),
            "--size" => {
                let v = value()?;
                let (w, h) = v.split_once('x').ok_or("--size WxH")?;
                o.size = Some((
                    w.parse().map_err(|_| "--size WxH")?,
                    h.parse().map_err(|_| "--size WxH")?,
                ));
            }
            "--screenshot-after" => {
                o.screenshot_after =
                    Some(value()?.parse().map_err(|_| "--screenshot-after SECONDS")?)
            }
            _ => return Err(format!("unknown option {flag}")),
        }
    }
    Ok(o)
}

fn main() {
    let options = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let storage = match Storage::new(options.data_dir.clone()) {
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
    let socket = match net::connect_socket(options.server) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("socket: {e}");
            std::process::exit(1);
        }
    };
    let start = Instant::now();
    let mut client = Client::new(PlayerInfo::default(), "", net::random_bytes);
    client.connect(net::now(start));
    let mut app = App {
        options,
        storage,
        socket,
        client,
        start,
        running: None,
        center: None,
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
        eprintln!("{e}");
        std::process::exit(1);
    }
}
