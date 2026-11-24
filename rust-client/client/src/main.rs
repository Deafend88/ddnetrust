//! `ddnet-rs`: the graphical client. It connects to a local server and draws the map,
//! tees and entities around the local tee, which you play with your C++ client's
//! settings and binds (read once from `settings_ddnet.cfg`, never written). F12 saves a
//! screenshot into our own directory; Esc frees the mouse, a click grabs it again.
//!
//! Usage: ddnet-rs <server ip:port> [--data-dir PATH] [--vsync] [--size WxH] [--view X,Y]
//!        [--follow CLIENT_ID] [--screenshot-after SECONDS] [--no-grab]

use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::Instant;

use ddnet::map::Map;
use ddnet::net;
use ddnet::net::client::{CameraState, Client, PlayerInfo, State};
use ddnet::physics::collision::Collision;
use ddnet::physics::vmath::vec2;
use ddnet::predict::{Predictor, Rendered};
use ddnet::protocol::generated as g;
use ddnet_rs::assets;
use ddnet_rs::camera::{self, Camera, Frame};
use ddnet_rs::config::{
    KEY_LAST, KEY_MOUSE_WHEEL_DOWN, KEY_MOUSE_WHEEL_LEFT, KEY_MOUSE_WHEEL_RIGHT,
    KEY_MOUSE_WHEEL_UP, Settings,
};
use ddnet_rs::controls::{self, Controls, GameState};
use ddnet_rs::gfx::batch::Batch;
use ddnet_rs::gfx::{self, Gfx, Size, block_on};
use ddnet_rs::render::items::{self, Items};
use ddnet_rs::render::map::{MapRender, View, hsl_packed_to_rgb};
use ddnet_rs::render::players::Players;
use ddnet_rs::storage::{self, Storage};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, KeyEvent, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

/// C++ `cl_background_color` default (packed HSL).
const BACKGROUND_COLOR: u32 = 128;
/// C++: `SERVER_TICK_SPEED`: nanoseconds per tick.
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
    /// Save one screenshot this many seconds after the map shows, then quit.
    screenshot_after: Option<f64>,
    /// Never grab the mouse (scripted runs; also implied by `--view` and `--follow`).
    no_grab: bool,
}

struct Running {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gfx: Gfx,
    map: Option<(MapRender, &'static Collision)>,
    predictor: Option<Predictor<'static>>,
    players: Players,
    items: Items,
    map_shown_at: Option<Instant>,
}

struct App {
    options: Options,
    storage: Storage,
    settings: Settings,
    socket: UdpSocket,
    client: Client,
    start: Instant,
    running: Option<Running>,
    camera: Camera,
    controls: Controls,
    /// C++ `m_aCurrentKeyStates`.
    pressed: Vec<bool>,
    /// Raw mouse motion since the last frame (C++ `SDL_GetRelativeMouseState`).
    mouse_delta: (f64, f64),
    /// C++ `m_InputGrabbed` (relative mouse) and `m_MouseFocus`.
    grabbed: bool,
    focused: bool,
    last_frame: Instant,
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
            items: Items::default(),
            map_shown_at: None,
        })
    }

    /// C++ `CInput::MouseModeRelative` / `MouseModeAbsolute`: hide and confine the
    /// cursor while playing.
    fn set_grab(&mut self, grab: bool) {
        self.grabbed = grab;
        self.apply_cursor(grab);
        self.mouse_delta = (0.0, 0.0);
    }

    /// Hides and confines the cursor (Locked where Confined is unsupported), or frees it.
    fn apply_cursor(&self, grab: bool) {
        let Some(r) = &self.running else { return };
        r.window.set_cursor_visible(!grab);
        let mode = if grab {
            CursorGrabMode::Confined
        } else {
            CursorGrabMode::None
        };
        if r.window.set_cursor_grab(mode).is_err() && grab {
            let _ = r.window.set_cursor_grab(CursorGrabMode::Locked);
        }
    }

    /// What `CCamera`'s zoom commands and `UpdateCamera` read this frame.
    fn camera_frame(&self, frame_time: f32) -> Frame {
        Frame {
            local_time: self.start.elapsed().as_secs_f32(),
            frame_time,
            allow_zoom: self
                .running
                .as_ref()
                .and_then(|r| r.predictor.as_ref())
                .is_some_and(|p| p.allow_zoom || p.spectating),
        }
    }

    /// One key event through `CBinds::OnInput` and the commands it runs. C++: the key
    /// state changes when the event is added (`CInput::AddKeyEvent`), before binds see it.
    fn key_event(&mut self, key: usize, press: bool, release: bool) {
        if press {
            self.pressed[key] = true;
        }
        if release {
            self.pressed[key] = false;
        }
        let pressed = &self.pressed;
        let lines = self
            .settings
            .binds
            .on_input(key, press, release, |k| pressed[k]);
        let frame = self.camera_frame(0.0);
        let game = GameState {
            frozen_input: self
                .running
                .as_ref()
                .and_then(|r| r.predictor.as_ref())
                .is_some_and(|p| p.bug_ddrace_input() && p.spectating),
        };
        for (stroke, line) in lines {
            let config = &self.settings.config;
            self.controls
                .execute(stroke, &line, game, &mut self.camera, &frame, config);
        }
    }

    /// C++: `CGameClient::UpdatePositions` (`m_LocalCharacterPos`): the fixed view, else
    /// the rendered position of the followed or local tee, else the last one.
    fn local_pos(&mut self, rendered: &[Rendered]) -> Option<(f32, f32)> {
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
        let frame_time = self.last_frame.elapsed().as_secs_f32().max(0.0001);
        self.last_frame = Instant::now();
        // C++: CGameClient::OnUpdate: the relative mouse motion of this frame
        let world_paused = (self.running.as_ref())
            .and_then(|r| r.predictor.as_ref())
            .is_some_and(|p| p.world_paused);
        let (dx, dy) = std::mem::take(&mut self.mouse_delta);
        if self.grabbed && self.focused && (dx != 0.0 || dy != 0.0) {
            let config = &self.settings.config;
            self.controls
                .on_cursor_move(dx as f32, dy as f32, config, world_paused);
        }
        // C++: CClient::Update (sends SnapInput, OnNewSnapshot reports the camera)
        self.client.input = self.controls.snap_input();
        let size = self.running.as_ref().map(|r| size_of(&r.config));
        self.client.camera = CameraState {
            zoom: self.camera.zoom,
            zooming: self.camera.zooming,
            zoom_target: self.camera.zoom_smoothing_target,
            deadzone: camera::deadzone(&self.settings.config),
            follow_factor: camera::follow_factor(&self.settings.config),
            aspect: size.map_or(16.0 / 9.0, |s| s.width as f32 / s.height as f32),
        };
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
        // C++: CGameClient::OnRender: UpdatePositions, UpdateCamera, CCamera::OnRender
        let local = self.local_pos(&rendered);
        let frame = self.camera_frame(frame_time);
        let config = &self.settings.config;
        self.camera.update(self.controls.mouse_pos, &frame, config);
        if let Some((x, y)) = local {
            self.camera
                .on_render(&mut self.controls, vec2(x, y), config);
        }
        let center = match (self.options.view, local) {
            (Some(v), _) => Some(v),
            (None, Some(_)) => Some((self.camera.center.x, self.camera.center.y)),
            _ => None,
        };
        let envelope_time = self.envelope_time();
        let r = self.running.as_mut().unwrap();
        let surface_frame = match r.surface.get_current_texture() {
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
        if let (Some((map, collision)), Some(center)) = (&r.map, center) {
            let view = View {
                center,
                zoom: self.camera.zoom,
                width: size.width,
                height: size.height,
                envelope_time,
            };
            map.render_background(&mut batch, &view);
            if let Some(predictor) = &r.predictor {
                r.items.render(
                    &mut batch,
                    &items::Frame {
                        client: &self.client,
                        predictor,
                        rendered: &rendered,
                        collision,
                        sprites: r.players.sprites(),
                    },
                );
            }
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
        r.gfx.composite(
            &surface_frame.texture.create_view(&Default::default()),
            size,
        );
        r.window.pre_present_notify();
        r.gfx.queue.present(surface_frame);
        if let (Some(after), Some(shown)) = (self.options.screenshot_after, r.map_shown_at)
            && shown.elapsed().as_secs_f64() >= after
        {
            // for reproducing the view with --view
            if let Some((x, y)) = center {
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
            Ok(r) => {
                self.running = Some(r);
                // scripted views (screenshot comparisons) leave the mouse alone
                let o = &self.options;
                if !o.no_grab && o.view.is_none() && o.follow.is_none() {
                    self.set_grab(true);
                }
            }
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
            // C++: SDL_WINDOWEVENT_FOCUS_LOST/GAINED: key states cleared, the mouse let
            // go while unfocused and grabbed again on return
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                if !focused {
                    self.pressed.fill(false);
                }
                self.apply_cursor(focused && self.grabbed);
            }
            // synthetic events: winit's own key presses/releases on focus changes
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(code),
                        state,
                        repeat,
                        ..
                    },
                is_synthetic: false,
                ..
            } => match code {
                KeyCode::F12 if state == ElementState::Pressed && !repeat => {
                    if let Some(r) = &self.running {
                        screenshot(&r.gfx);
                    }
                }
                // ponytail: Esc frees the mouse until the menus (6.8) take it over
                KeyCode::Escape if state == ElementState::Pressed && !repeat => {
                    self.set_grab(false)
                }
                _ => {
                    if let Some(key) = controls::key_id(code) {
                        let press = state == ElementState::Pressed;
                        self.key_event(key, press, !press);
                    }
                }
            },
            WindowEvent::MouseInput { state, button, .. } => {
                if !self.grabbed {
                    if state == ElementState::Pressed {
                        self.set_grab(true);
                    }
                } else if let Some(key) = controls::mouse_key_id(button) {
                    let press = state == ElementState::Pressed;
                    self.key_event(key, press, !press);
                }
            }
            // C++: TranslateMouseWheelEventKey, one press+release per wheel event
            WindowEvent::MouseWheel { delta, .. } if self.grabbed => {
                let (x, y) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x, y),
                    MouseScrollDelta::PixelDelta(p) => (p.x as f32, p.y as f32),
                };
                let key = if y > 0.0 {
                    KEY_MOUSE_WHEEL_UP
                } else if y < 0.0 {
                    KEY_MOUSE_WHEEL_DOWN
                } else if x > 0.0 {
                    KEY_MOUSE_WHEEL_RIGHT
                } else if x < 0.0 {
                    KEY_MOUSE_WHEEL_LEFT
                } else {
                    return;
                };
                self.key_event(key, true, true);
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        // C++: SDL relative mouse mode, raw motion while grabbed and focused
        if let DeviceEvent::MouseMotion { delta } = event
            && self.grabbed
            && self.focused
        {
            self.mouse_delta.0 += delta.0;
            self.mouse_delta.1 += delta.1;
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
        .ok_or("usage: ddnet-rs <server ip:port> [--data-dir PATH] [--vsync] [--size WxH] [--view X,Y] [--follow CLIENT_ID] [--screenshot-after SECONDS] [--no-grab]")?;
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
        screenshot_after: None,
        no_grab: false,
    };
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--data-dir" => o.data_dir = Some(value()?.into()),
            "--vsync" => o.vsync = true,
            "--no-grab" => o.no_grab = true,
            "--view" => {
                let v = value()?;
                let (x, y) = v.split_once(',').ok_or("--view X,Y")?;
                o.view = Some((
                    x.parse().map_err(|_| "--view X,Y")?,
                    y.parse().map_err(|_| "--view X,Y")?,
                ));
            }
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

/// The C++ client's `settings_ddnet.cfg` (first storage path that has it, read only),
/// then our own `settings.cfg`, which overrides it.
fn load_settings(storage: &Storage) -> Settings {
    let cpp = storage.read("settings_ddnet.cfg");
    let ours = storage::save_dir().and_then(|d| std::fs::read(d.join("settings.cfg")).ok());
    let files: Vec<&[u8]> = [&cpp, &ours]
        .into_iter()
        .flatten()
        .map(|f| &f[..])
        .collect();
    let settings = Settings::load(&files);
    if cpp.is_some() {
        println!(
            "settings: read settings_ddnet.cfg (read only); skipped {} commands DDNet-rs does not use",
            settings.skipped
        );
    }
    settings
}

/// C++: what `CGameClient::SendStartInfo` takes from the config.
fn player_info(settings: &Settings) -> PlayerInfo {
    let c = &settings.config;
    let default = PlayerInfo::default();
    PlayerInfo {
        name: if c.player_name.is_empty() {
            default.name
        } else {
            c.player_name.clone()
        },
        skin: c.player_skin.clone(),
        use_custom_color: c.player_use_custom_color,
        color_body: c.player_color_body as i32,
        color_feet: c.player_color_feet as i32,
        ..default
    }
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
    let settings = load_settings(&storage);
    let socket = match net::connect_socket(options.server) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("socket: {e}");
            std::process::exit(1);
        }
    };
    let start = Instant::now();
    let mut client = Client::new(player_info(&settings), "", net::random_bytes);
    client.connect(net::now(start));
    // C++: CGameClient::OnReset -> CCamera::OnReset
    let mut camera = Camera::default();
    camera.on_reset(&settings.config);
    let mut app = App {
        options,
        storage,
        settings,
        socket,
        client,
        start,
        running: None,
        camera,
        controls: Controls::default(),
        pressed: vec![false; KEY_LAST],
        mouse_delta: (0.0, 0.0),
        grabbed: false,
        focused: true,
        last_frame: Instant::now(),
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
