//! `ddnet-rs`: the graphical client. A start screen connects to a local server by
//! address; in game you play with your C++ client's settings and binds (read once from
//! `settings_ddnet.cfg`, never written) and Esc opens a small menu. Settings changed in
//! the menu go to our own `%APPDATA%\DDNet-rs\settings.cfg`. F12 saves a screenshot.
//!
//! Usage: ddnet-rs [server ip:port] [--data-dir PATH] [--vsync] [--size WxH] [--view X,Y]
//!        [--follow CLIENT_ID] [--screenshot-after SECONDS] [--no-grab]

use std::net::{SocketAddr, UdpSocket};
use std::rc::Rc;
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
    KEY_ESCAPE, KEY_LAST, KEY_MOUSE_WHEEL_DOWN, KEY_MOUSE_WHEEL_LEFT, KEY_MOUSE_WHEEL_RIGHT,
    KEY_MOUSE_WHEEL_UP, Settings, split_line,
};
use ddnet_rs::controls::{self, Controls, GameState};
use ddnet_rs::gfx::batch::Batch;
use ddnet_rs::gfx::{self, Gfx, Size, block_on};
use ddnet_rs::hud::{self, HudData, Widget};
use ddnet_rs::hud_editor::Editor;
use ddnet_rs::island::Island;
use ddnet_rs::killfeed::Killfeed;
use ddnet_rs::media::{self, Media};
use ddnet_rs::movement::Movement;
use ddnet_rs::nowplaying::NowPlayingCard;
use ddnet_rs::render::items::{self, Items};
use ddnet_rs::render::map::{MapRender, View, hsl_packed_to_rgb};
use ddnet_rs::render::players::Players;
use ddnet_rs::statusbar::StatusBar;
use ddnet_rs::storage::{self, Storage};
use ddnet_rs::style::{self, Glass};
use ddnet_rs::ui::{Action, Menu, Ui};
use ddnet_rs::watermark::Watermark;
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
    /// Connect at start instead of showing the start screen.
    server: Option<SocketAddr>,
    data_dir: Option<std::path::PathBuf>,
    /// Forces vsync on (else `gfx_vsync`).
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

/// The window and what draws into it.
struct Running {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gfx: Gfx,
    players: Players,
    ui: Ui,
    /// The `rs_glass` and zoom factor the UI was set up with.
    ui_glass: i32,
    ui_zoom: f32,
}

/// One connection to a server and its game state (C++: what `CClient::Disconnect` and
/// `CGameClient::OnReset` clear).
struct Session {
    socket: UdpSocket,
    /// The server address (the watermark shows it).
    server: String,
    client: Client,
    /// The map's collision is shared with the predictor; both drop with the session.
    map: Option<(MapRender, Rc<Collision>)>,
    predictor: Option<Predictor>,
    items: Items,
    map_shown_at: Option<Instant>,
    camera: Camera,
    controls: Controls,
    /// C++ `m_LocalCharacterPos` (or the followed tee's): kept while the tee is gone.
    center: Option<(f32, f32)>,
}

impl Session {
    fn connect(addr: SocketAddr, settings: &Settings, start: Instant) -> Result<Self, String> {
        let socket = net::connect_socket(addr).map_err(|e| format!("socket: {e}"))?;
        let mut client = Client::new(player_info(settings), "", net::random_bytes);
        client.connect(net::now(start));
        // C++: CGameClient::OnReset -> CCamera::OnReset
        let mut camera = Camera::default();
        camera.on_reset(&settings.config);
        Ok(Self {
            socket,
            server: addr.to_string(),
            client,
            map: None,
            predictor: None,
            items: Items::default(),
            map_shown_at: None,
            camera,
            controls: Controls::default(),
            center: None,
        })
    }

    fn disconnect(&mut self, start: Instant) {
        self.client.disconnect(None, net::now(start));
        let _ = net::pump(&self.socket, &mut self.client, net::now(start));
    }

    /// What `CCamera`'s zoom commands and `UpdateCamera` read this frame.
    fn camera_frame(&self, start: Instant, frame_time: f32) -> Frame {
        Frame {
            local_time: start.elapsed().as_secs_f32(),
            frame_time,
            allow_zoom: (self.predictor.as_ref()).is_some_and(|p| p.allow_zoom || p.spectating),
        }
    }

    /// C++: `CGameClient::UpdatePositions` (`m_LocalCharacterPos`): the rendered
    /// position of the followed or local tee, else the last one.
    fn local_pos(&mut self, rendered: &[Rendered], follow: Option<i32>) -> Option<(f32, f32)> {
        let id = follow.or(self.client.local_id());
        if let Some(r) = rendered.iter().find(|r| Some(r.id) == id) {
            self.center = Some((r.pos.x, r.pos.y));
        }
        self.center
    }

    /// C++: `CEnvelopeState::Update` while playing: predicted tick and intra tick since
    /// the round start.
    fn envelope_time(&self) -> i64 {
        let Some(info) =
            (self.client.snapshot().iter()).find(|it| it.type_id == g::NETOBJTYPE_GAMEINFO)
        else {
            return 0;
        };
        let info = g::ObjGameInfo::from_ints(&info.data);
        let tick = (self.client.pred_tick() - 1 - info.round_start_tick) as i64;
        (self.client.pred_intra_tick as f64 * NANOS_PER_TICK as f64) as i64 + tick * NANOS_PER_TICK
    }
}

struct App {
    options: Options,
    storage: Storage,
    settings: Settings,
    start: Instant,
    running: Option<Running>,
    session: Option<Session>,
    menu: Menu,
    /// The HUD widgets.
    hud: Vec<Box<dyn Widget>>,
    editor: Editor,
    /// The media thread (now playing, 7.10); `None` while `rs_media` is 0.
    media: Option<Media>,
    /// The island is expanded into the media player (F7).
    player_open: bool,
    /// The lyrics switch as last sent to the media thread.
    lyrics_on: Option<bool>,
    /// C++ `CClient::m_FrameTimeAverage`.
    frame_time_average: f32,
    /// Frames since the map showed (`--screenshot-after` prints the mean frame time).
    shown_frames: u32,
    /// C++ `m_aCurrentKeyStates`.
    pressed: Vec<bool>,
    /// Raw mouse motion since the last frame (C++ `SDL_GetRelativeMouseState`).
    mouse_delta: (f64, f64),
    /// C++ `m_InputGrabbed` (relative mouse) and `m_MouseFocus`.
    grabbed: bool,
    focused: bool,
    /// The present mode in use (C++ `gfx_vsync`).
    vsync: bool,
    last_frame: Instant,
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
        config.present_mode = present_mode(self.vsync);
        surface.configure(&device, &config);
        let mut gfx = Gfx::new(device, queue, config.format, size_of(&config));
        let players = Players::new(&mut gfx, &self.storage);
        let c = &self.settings.config;
        let ui = Ui::new(
            &window,
            &gfx,
            config.format,
            Glass::from_setting(c.rs_glass),
        );
        Ok(Running {
            ui_glass: c.rs_glass,
            ui_zoom: 0.0, // set on the first frame
            window,
            surface,
            config,
            gfx,
            players,
            ui,
        })
    }

    /// C++ `CInput::MouseModeRelative` / `MouseModeAbsolute`: hide and confine the
    /// cursor (Locked where Confined is unsupported), or free it.
    fn set_grab(&mut self, grab: bool) {
        self.grabbed = grab;
        self.mouse_delta = (0.0, 0.0);
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

    /// The start screen shows without a connection, the Esc menu on request.
    fn menu_visible(&self) -> bool {
        self.session.is_none() || self.menu.open
    }

    /// A menu, the HUD editor or the media player has the mouse and keyboard (C++ menu
    /// mode).
    fn ui_active(&self) -> bool {
        self.menu_visible() || self.editor.open || self.player_open
    }

    fn open_editor(&mut self) {
        self.menu.open = false;
        self.player_open = false;
        self.editor.open = self.session.is_some();
    }

    /// Closing the editor saves the layout.
    fn close_editor(&mut self) {
        self.editor.close();
        self.save_settings();
    }

    fn connect(&mut self, addr: SocketAddr) {
        self.menu.error = None;
        self.menu.open = false;
        match Session::connect(addr, &self.settings, self.start) {
            Ok(s) => self.session = Some(s),
            Err(e) => self.menu.error = Some(e),
        }
    }

    fn disconnect(&mut self) {
        if let Some(s) = &mut self.session {
            s.disconnect(self.start);
        }
        self.end_session();
    }

    /// Drops the session (its collision with it) and frees its map textures.
    fn end_session(&mut self) {
        if let Some(s) = self.session.take()
            && let (Some((map, _)), Some(r)) = (s.map, &mut self.running)
        {
            map.free(&mut r.gfx);
        }
        self.menu.open = false;
        self.player_open = false;
        if self.editor.open {
            self.close_editor();
        }
    }

    /// One key event. With the settings key reader waiting it binds the key; otherwise
    /// it goes through `CBinds::OnInput` and the commands it runs. C++: the key state
    /// changes when the event is added (`CInput::AddKeyEvent`), before binds see it.
    fn key_event(&mut self, key: usize, press: bool, release: bool) {
        if press && let Some(command) = self.menu.reading_key.take() {
            if key != KEY_ESCAPE {
                self.settings.binds.rebind(command, key);
            }
            return;
        }
        if press && key == KEY_ESCAPE {
            if self.editor.open {
                self.close_editor();
            } else if self.player_open {
                self.player_open = false;
            } else {
                self.menu.open = !self.menu.open;
            }
        }
        if press {
            self.pressed[key] = true;
        }
        if release {
            self.pressed[key] = false;
        }
        let menus_active = self.ui_active();
        let Some(s) = &mut self.session else { return };
        let pressed = &self.pressed;
        let lines =
            (self.settings.binds).on_input(key, press, release, menus_active, |k| pressed[k]);
        let frame = s.camera_frame(self.start, 0.0);
        let game = GameState {
            frozen_input: (s.predictor.as_ref())
                .is_some_and(|p| p.bug_ddrace_input() && p.spectating),
        };
        let (mut edit_hud, mut player) = (false, false);
        for (stroke, line) in lines {
            let config = &self.settings.config;
            s.controls
                .execute(stroke, &line, game, &mut s.camera, &frame, config);
            for (command, args) in split_line(&line).into_iter().filter(|_| stroke) {
                match command.as_str() {
                    "hud_editor" => edit_hud = true,
                    "media_player" => player = true,
                    "media_play_pause" => {
                        media_send(self.media.as_ref(), media::Command::PlayPause)
                    }
                    "media_next" => media_send(self.media.as_ref(), media::Command::Next),
                    "media_prev" => media_send(self.media.as_ref(), media::Command::Prev),
                    // C++: CClient::Con_Rcon, Con_RconAuth
                    "rcon" => s.client.rcon(&args, net::now(self.start)),
                    "rcon_auth" => s.client.rcon_auth(&args, net::now(self.start)),
                    _ => {}
                }
            }
        }
        if edit_hud {
            self.open_editor();
        }
        // F7: the island expands into the media player, like a menu
        if player && !self.editor.open && self.media.is_some() {
            self.player_open = !self.player_open;
            self.menu.open = false;
        }
    }

    /// Our own settings file; the C++ client's files are never written.
    fn save_settings(&self) {
        let Some(dir) = storage::save_dir() else {
            return;
        };
        let text = self.settings.serialize();
        if let Err(e) = std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(dir.join("settings.cfg"), text))
        {
            eprintln!("saving settings failed: {e}");
        }
    }

    /// C++: `CClient::Update` (and `CGameClient::OnUpdate`'s mouse motion): mouse,
    /// input, network and the map; an error ends the session.
    fn update_session(&mut self) -> Result<(), String> {
        let menu_visible = self.ui_active();
        let size = self.running.as_ref().map(|r| size_of(&r.config));
        let (start, config) = (self.start, &self.settings.config);
        let Some(s) = &mut self.session else {
            return Ok(());
        };
        // C++: CGameClient::OnUpdate: the relative mouse motion of this frame
        let world_paused = (s.predictor.as_ref()).is_some_and(|p| p.world_paused);
        let (dx, dy) = std::mem::take(&mut self.mouse_delta);
        if self.grabbed && self.focused && (dx != 0.0 || dy != 0.0) {
            s.controls
                .on_cursor_move(dx as f32, dy as f32, config, world_paused);
        }
        // C++: CClient::Update (sends SnapInput, OnNewSnapshot reports the camera)
        let bug_input = (s.predictor.as_ref()).is_some_and(|p| p.bug_ddrace_input());
        s.client.input = s.controls.snap_input(menu_visible, bug_input);
        s.client.camera = CameraState {
            zoom: s.camera.zoom,
            zooming: s.camera.zooming,
            zoom_target: s.camera.zoom_smoothing_target,
            deadzone: camera::deadzone(config),
            follow_factor: camera::follow_factor(config),
            aspect: size.map_or(16.0 / 9.0, |s| s.width as f32 / s.height as f32),
        };
        net::pump(&s.socket, &mut s.client, net::now(start))
            .map_err(|e| format!("network: {e}"))?;
        if s.client.state() == State::Offline {
            return Err(format!("disconnected: '{}'", s.client.conn.error()));
        }
        if s.map.is_none()
            && s.client.state() == State::Online
            && let Some(r) = &mut self.running
            && let Some(m) = (s.client.map()).and_then(|m| Map::load(m.data.clone()).ok())
        {
            let images = assets::load_map_images(&self.storage, &m);
            let collision = Rc::new(Collision::new(&m));
            s.map = Some((
                MapRender::new(&mut r.gfx, &m, &images),
                Rc::clone(&collision),
            ));
            s.predictor = Some(Predictor::new(collision, &m));
            s.map_shown_at = Some(Instant::now());
        }
        Ok(())
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let frame_time = self.last_frame.elapsed().as_secs_f32().max(0.0001);
        self.last_frame = Instant::now();
        // C++: CClient::Run
        self.frame_time_average = self.frame_time_average * 0.9 + frame_time * 0.1;
        let o = &self.options;
        let want_grab = self.session.is_some()
            && !self.ui_active()
            && self.focused
            && !o.no_grab
            && o.view.is_none()
            && o.follow.is_none();
        if want_grab != self.grabbed {
            self.set_grab(want_grab);
        }
        let want_vsync = self.options.vsync || self.settings.config.gfx_vsync != 0;
        if want_vsync != self.vsync
            && let Some(r) = &mut self.running
        {
            self.vsync = want_vsync;
            r.config.present_mode = present_mode(want_vsync);
            r.surface.configure(&r.gfx.device, &r.config);
        }
        if let Err(e) = self.update_session() {
            self.end_session();
            self.menu.error = Some(e);
        }
        // the media thread runs while `rs_media` is on (started on the first frame, with
        // the saved source); dropping it ends the thread
        let media_on = self.settings.config.rs_media != 0;
        if media_on != self.media.is_some() {
            let source = &self.settings.config.rs_media_source;
            self.media =
                media_on.then(|| Media::start(Some(source.clone()).filter(|s| !s.is_empty())));
            self.lyrics_on = None;
            self.player_open &= media_on;
        }
        // lyrics are fetched unless their place is "off" (island settings)
        let lyrics = ddnet_rs::island::lyrics_place(&self.settings.hud) != "off";
        if self.lyrics_on != Some(lyrics) {
            media_send(self.media.as_ref(), media::Command::Lyrics(lyrics));
            self.lyrics_on = Some(lyrics);
        }
        // the player's source choice lasts across restarts
        let chosen = self
            .media
            .as_ref()
            .map(|m| m.state().chosen.unwrap_or_default());
        if let Some(chosen) = chosen
            && chosen != self.settings.config.rs_media_source
        {
            self.settings.config.rs_media_source = chosen;
            self.save_settings();
        }
        let Some(r) = &mut self.running else { return };
        let mut batch = Batch::default();
        let [cr, cg, cb] = hsl_packed_to_rgb(BACKGROUND_COLOR);
        batch.clear(cr, cg, cb);
        let mut center = None;
        let mut rendered = Vec::new();
        let mut events = Vec::new();
        let mut map_shown_at = None;
        if let Some(s) = &mut self.session {
            // C++: CGameClient::OnNewSnapshot/OnPredict, then UpdateRenderedCharacters
            let local_freeze_end = match &mut s.predictor {
                Some(p) => {
                    p.update(&mut s.client);
                    rendered = p.rendered(&s.client);
                    p.predicted_freeze_end()
                }
                None => None,
            };
            // this frame's game messages go to the HUD (and do not pile up)
            events = std::mem::take(&mut s.client.messages);
            // C++: CGameClient::OnRender: UpdatePositions, UpdateCamera, CCamera::OnRender
            let local = s.local_pos(&rendered, self.options.follow);
            let frame = s.camera_frame(self.start, frame_time);
            let config = &self.settings.config;
            s.camera.update(s.controls.mouse_pos, &frame, config);
            if let Some((x, y)) = local {
                s.camera.on_render(&mut s.controls, vec2(x, y), config);
            }
            center = match (self.options.view, local) {
                (Some(v), _) => Some(v),
                (None, Some(_)) => Some((s.camera.center.x, s.camera.center.y)),
                _ => None,
            };
            map_shown_at = s.map_shown_at;
            self.shown_frames += map_shown_at.is_some() as u32;
            let size = size_of(&r.config);
            if let (Some((map, collision)), Some(center)) = (&s.map, center) {
                let view = View {
                    center,
                    zoom: s.camera.zoom,
                    width: size.width,
                    height: size.height,
                    envelope_time: s.envelope_time(),
                };
                map.render_background(&mut batch, &view);
                if let Some(predictor) = &s.predictor {
                    s.items.render(
                        &mut batch,
                        &items::Frame {
                            client: &s.client,
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
                    &s.client,
                    collision,
                    &rendered,
                    local_freeze_end,
                );
                map.render_foreground(&mut batch, &view);
            }
        }
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
        let world = r.gfx.world_view();
        r.gfx.render(&mut batch, &world, size);
        let target = surface_frame.texture.create_view(&Default::default());
        r.gfx.composite(&target, size);
        let mut action = None;
        let connected = self.session.is_some();
        // the settings may have changed the look
        let c = &self.settings.config;
        if c.rs_glass != r.ui_glass {
            r.ui.painter.set_glass(Glass::from_setting(c.rs_glass));
            r.ui_glass = c.rs_glass;
        }
        // one point = one pixel of a 1080p window, whatever the OS DPI
        let scale = style::ui_scale(size.height, c.rs_ui_scale);
        let zoom = scale / r.window.scale_factor() as f32;
        if zoom != r.ui_zoom {
            r.ui.painter.ctx.set_zoom_factor(zoom);
            r.ui_zoom = zoom;
        }
        let menu_visible = !connected || self.menu.open;
        // the HUD draws once the map is there, under the menus
        let follow = self.options.follow;
        let hud_on = !self.hud.is_empty();
        let player_name = player_info(&self.settings).name;
        let data = (self.session.as_ref())
            .filter(|s| hud_on && s.map.is_some())
            .map(|s| {
                let id = follow.or(s.client.local_id());
                HudData {
                    client: &s.client,
                    server: &s.server,
                    player_name: &player_name,
                    frame_time_average: self.frame_time_average,
                    glass: Glass::from_setting(self.settings.config.rs_glass),
                    editing: self.editor.open,
                    media: self.media.as_ref(),
                    player_open: self.player_open,
                    predictor: s.predictor.as_ref(),
                    tee: rendered.iter().find(|t| Some(t.id) == id),
                    events: &events,
                }
            });
        let mut editor_done = false;
        if menu_visible || data.is_some() {
            let (menu, settings, widgets) = (&mut self.menu, &mut self.settings, &mut self.hud);
            let editor = &mut self.editor;
            r.ui.frame(&r.window, &r.gfx, &target, size, |ctx| {
                match &data {
                    Some(data) if editor.open => {
                        editor_done = editor.show(ctx, widgets, data, &mut settings.hud);
                    }
                    Some(data) => hud::show(ctx, widgets, data, &settings.hud),
                    None => {}
                }
                if menu_visible {
                    action = menu.show(ctx, connected, settings);
                }
            });
        }
        if !menu_visible && !self.editor.open && !self.player_open {
            r.ui.hidden();
        }
        r.window.pre_present_notify();
        r.gfx.queue.present(surface_frame);
        if let (Some(after), Some(shown)) = (self.options.screenshot_after, map_shown_at)
            && shown.elapsed().as_secs_f64() >= after
        {
            let secs = shown.elapsed().as_secs_f64();
            println!(
                "frame time {:.3} ms (mean of {} frames)",
                secs * 1000.0 / f64::from(self.shown_frames.max(1)),
                self.shown_frames
            );
            // for reproducing the view with --view
            if let Some((x, y)) = center {
                println!("camera center {x} {y}");
            }
            screenshot(&r.gfx);
            self.disconnect();
            return event_loop.exit();
        }
        match action {
            Some(Action::Connect(addr)) => self.connect(addr),
            Some(Action::Disconnect) => self.disconnect(),
            Some(Action::Resume) => self.menu.open = false,
            Some(Action::SettingsDone) => self.save_settings(),
            Some(Action::EditHud) => self.open_editor(),
            Some(Action::Quit) => {
                self.disconnect();
                event_loop.exit();
            }
            None => {}
        }
        if editor_done {
            self.close_editor();
        }
    }
}

fn present_mode(vsync: bool) -> wgpu::PresentMode {
    if vsync {
        wgpu::PresentMode::AutoVsync
    } else {
        wgpu::PresentMode::AutoNoVsync
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
        // egui gets input only while a menu is drawn (and no key is being read), but the
        // window focus always: egui draws keyboard focus only in a focused window
        let to_egui = matches!(event, WindowEvent::Focused(_))
            || (self.ui_active() && self.menu.reading_key.is_none());
        if let Some(r) = &mut self.running
            && to_egui
        {
            r.ui.on_window_event(&r.window, &event);
        }
        match event {
            WindowEvent::CloseRequested => {
                self.disconnect();
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
            // C++: SDL_WINDOWEVENT_FOCUS_LOST: key states cleared (the grab follows focus
            // in `frame`)
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                if !focused {
                    self.pressed.fill(false);
                }
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
            } => {
                let press = state == ElementState::Pressed;
                if code == KeyCode::F12 && press && !repeat {
                    if let Some(r) = &self.running {
                        screenshot(&r.gfx);
                    }
                } else if let Some(key) = controls::key_id(code) {
                    self.key_event(key, press, !press);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(key) = controls::mouse_key_id(button) {
                    let press = state == ElementState::Pressed;
                    self.key_event(key, press, !press);
                }
            }
            // C++: TranslateMouseWheelEventKey, one press+release per wheel event
            WindowEvent::MouseWheel { delta, .. } => {
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
    let mut o = Options {
        server: None,
        data_dir: None,
        vsync: false,
        size: None,
        view: None,
        follow: None,
        screenshot_after: None,
        no_grab: false,
    };
    let mut args = std::env::args().skip(1).peekable();
    if let Some(a) = args.next_if(|a| !a.starts_with("--")) {
        let server: SocketAddr = a.parse().map_err(|_| "usage: ddnet-rs [server ip:port] [--data-dir PATH] [--vsync] [--size WxH] [--view X,Y] [--follow CLIENT_ID] [--screenshot-after SECONDS] [--no-grab]")?;
        if !server.ip().is_loopback() {
            return Err(format!(
                "refusing {server}: only local (loopback) servers are allowed for now"
            ));
        }
        o.server = Some(server);
    }
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
fn load_settings(storage: &Storage, ours: Option<&[u8]>) -> Settings {
    let cpp = storage.read("settings_ddnet.cfg");
    let files: Vec<&[u8]> = [cpp.as_deref(), ours].into_iter().flatten().collect();
    let mut settings = Settings::load(&files);
    settings.add_our_binds();
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
    let mut options = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    // our settings may name the data dir (after --data-dir and DDNET_DATA_DIR)
    let ours = storage::save_dir().and_then(|d| std::fs::read(d.join("settings.cfg")).ok());
    let data_dir = Settings::load(ours.as_deref().as_slice())
        .config
        .rs_data_dir;
    if options.data_dir.is_none()
        && std::env::var_os("DDNET_DATA_DIR").is_none()
        && !data_dir.is_empty()
    {
        options.data_dir = Some(data_dir.into());
    }
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
    let settings = load_settings(&storage, ours.as_deref());
    let start = Instant::now();
    let vsync = options.vsync || settings.config.gfx_vsync != 0;
    let hud: Vec<Box<dyn Widget>> = vec![
        Box::new(Watermark::default()),
        Box::new(StatusBar::new(&storage)),
        Box::new(Movement),
        Box::new(Killfeed::new(&storage)),
        Box::new(Island::default()),
        Box::new(NowPlayingCard::default()),
    ];
    // the C++ HUD switches are the first values of our HUD items
    let mut settings = settings;
    let (config, layout) = (&settings.config, &mut settings.hud);
    layout.import_cpp(
        |var| config.int(var).unwrap_or(0) != 0,
        |id| {
            (hud.iter())
                .find(|w| w.id() == id)
                .map(|w| w.default_placement())
        },
    );
    let mut app = App {
        menu: Menu::new(options.server, &storage),
        hud,
        editor: Editor::default(),
        media: None,
        player_open: false,
        lyrics_on: None,
        frame_time_average: 0.0001,
        shown_frames: 0,
        options,
        storage,
        settings,
        start,
        running: None,
        session: None,
        pressed: vec![false; KEY_LAST],
        mouse_delta: (0.0, 0.0),
        grabbed: false,
        focused: true,
        vsync,
        last_frame: Instant::now(),
        error: None,
    };
    if let Some(addr) = app.options.server {
        app.connect(addr);
        if let Some(e) = app.menu.error.take() {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
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

fn media_send(media: Option<&Media>, command: media::Command) {
    if let Some(m) = media {
        m.send(command);
    }
}
