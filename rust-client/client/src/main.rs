//! `ddnet-rs`: the graphical client. A start screen connects to a local server by
//! address; in game you play with your C++ client's settings and binds (read once from
//! `settings_ddnet.cfg`, never written) and Esc opens a small menu. Settings changed in
//! the menu go to our own `%APPDATA%\DDNet-rs\settings.cfg`. F12 saves a screenshot.
//!
//! Usage: ddnet-rs [server ip:port] [--data-dir PATH] [--vsync] [--size WxH] [--view X,Y]
//!        [--follow CLIENT_ID] [--screenshot-after SECONDS] [--no-grab] [--fire WEAPON] [--esc-tab TAB]
//!
//! `--fire` (scripted captures): 2 s after the map shows, says `/weapons` (a practice
//! command on a local test server) and then keeps shooting that weapon (0 hammer, 1 gun,
//! ...) up and to the left, like `ddnet-headless --fire`.

use std::net::{SocketAddr, UdpSocket};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use ddnet::clients::{Clients, SnapContext};
use ddnet::map::Map;
use ddnet::net;
use ddnet::net::client::{CameraState, Client, PlayerInfo, State};
use ddnet::physics::collision::Collision;
use ddnet::physics::vmath::vec2;
use ddnet::predict::{Predictor, Rendered, SoundEvent};
use ddnet::protocol::generated as g;
use ddnet_rs::assets;
use ddnet_rs::camera::{self, Camera, Frame};
use ddnet_rs::chat::{self, Chat};
use ddnet_rs::config::{
    KEY_ESCAPE, KEY_LAST, KEY_LSHIFT, KEY_MOUSE_WHEEL_DOWN, KEY_MOUSE_WHEEL_LEFT,
    KEY_MOUSE_WHEEL_RIGHT, KEY_MOUSE_WHEEL_UP, KEY_RSHIFT, Settings, split_line,
};
use ddnet_rs::controls::{self, Controls, GameState, PlayerState};
use ddnet_rs::esc_menu::{self, EscMenu};
use ddnet_rs::gfx::batch::Batch;
use ddnet_rs::gfx::{self, Gfx, Size, block_on};
use ddnet_rs::hud::{self, HudData, Widget};
use ddnet_rs::hud_editor::Editor;
use ddnet_rs::island::Island;
use ddnet_rs::killfeed::Killfeed;
use ddnet_rs::mapsounds::MapSounds;
use ddnet_rs::media::{self, Media};
use ddnet_rs::movement::Movement;
use ddnet_rs::nowplaying::NowPlayingCard;
use ddnet_rs::render::items::{self, Items};
use ddnet_rs::render::map::{MapRender, View, hsl_packed_to_rgb};
use ddnet_rs::render::players::Players;
use ddnet_rs::sounds::{self, Sounds};
use ddnet_rs::statusbar::StatusBar;
use ddnet_rs::storage::{self, Storage};
use ddnet_rs::style::{self, Glass};
use ddnet_rs::teehead::TeeHeads;
use ddnet_rs::ui::{Action, Menu, Ui};
use ddnet_rs::watermark::Watermark;
use ddnet_rs::{freezebars, nameplates, scoreboard};
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
    /// Keep shooting this weapon (scripted captures).
    fire: Option<i32>,
    /// Open the Esc menu on this tab once in the game (screenshots).
    esc_tab: Option<usize>,
}

/// The window and what draws into it.
struct Running {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gfx: Gfx,
    players: Players,
    /// The asset packs the sprites were loaded from.
    packs: assets::Packs,
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
    /// `--fire`: `/weapons` was said.
    weapons_asked: bool,
    camera: Camera,
    controls: Controls,
    /// C++ `m_LocalCharacterPos` (or the followed tee's): kept while the tee is gone.
    center: Option<(f32, f32)>,
    /// C++ `m_aClients` and the sorted player lists, updated on each new snapshot.
    clients: Clients,
    /// The snapshot tick `clients` was last updated for.
    clients_tick: Option<i32>,
    /// C++ `CMapSounds` of the session's map.
    map_sounds: Option<MapSounds>,
    /// The session went online (C++ `OnStateChange(STATE_ONLINE)` resets the sounds).
    online: bool,
    /// C++ `CMotd::m_aServerMotd`.
    motd: String,
    /// C++ `m_LastRaceTick` (for `cl_confirm_disconnect_time`).
    last_race_tick: i32,
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
            weapons_asked: false,
            camera,
            controls: Controls::default(),
            center: None,
            clients: Clients::default(),
            clients_tick: None,
            map_sounds: None,
            online: false,
            motd: String::new(),
            last_race_tick: -1,
        })
    }

    fn disconnect(&mut self, start: Instant) {
        self.client.disconnect(None, net::now(start));
        let _ = net::pump(&self.socket, &mut self.client, net::now(start));
    }

    /// What `CCamera`'s zoom commands and `UpdateCamera` read this frame.
    /// C++ `m_Teams.Team(id)` (team 0 before the map loads).
    fn team(&self, id: i32) -> i32 {
        match &self.predictor {
            Some(p) if id >= 0 => p.team(id),
            _ => 0,
        }
    }

    /// C++ `IsOtherTeam(id)`.
    fn is_other_team(&self, id: i32) -> bool {
        self.predictor.as_ref().is_some_and(|p| p.is_other_team(id))
    }

    /// The chat's view of the game (C++ `CChat` reading `CGameClient`).
    fn chat_ctx<'a>(
        &'a self,
        config: &ddnet_rs::config::Config,
        team: &'a dyn Fn(i32) -> i32,
        other: &'a dyn Fn(i32) -> bool,
    ) -> chat::Ctx<'a> {
        chat::Ctx {
            clients: &self.clients,
            local_ids: [self.clients.local_id, -1],
            team,
            is_other_team: other,
            team_play: self.team_play(),
            demo: false,
            config: chat::ChatConfig::new(config),
        }
    }

    /// The first snapshot item of `type_id` (C++ `m_Snap.m_pGameInfoObj` and friends).
    fn item(&self, type_id: i32) -> Option<&ddnet::protocol::snapshot::Item> {
        self.client
            .snapshot()
            .iter()
            .find(|it| it.type_id == type_id)
    }

    /// C++ `m_Snap.m_SpecInfo`: spectating, and the followed player (-1 free view).
    fn spec_info(&self) -> (bool, i32) {
        let spectating = self.predictor.as_ref().is_some_and(|p| p.spectating);
        let id = (self.item(g::NETOBJTYPE_SPECTATORINFO)).map_or(-1, |it| {
            g::ObjSpectatorInfo::from_ints(&it.data).spectator_id
        });
        (spectating, if spectating { id } else { -1 })
    }

    /// What C++ `CScoreboard::IsActive` reads (`held`: `+scoreboard`).
    fn scoreboard_state(&self, held: bool, on_death: bool) -> scoreboard::ActiveState {
        let local = self.clients.local_id;
        scoreboard::ActiveState {
            held,
            statboard: false,
            local_info: local >= 0,
            spectating: self.spec_info().0,
            local_character: (self.client.snapshot().iter())
                .any(|it| it.type_id == g::NETOBJTYPE_CHARACTER && it.id == local),
            on_death,
            game_state_flags: (self.item(g::NETOBJTYPE_GAMEINFO))
                .map(|it| g::ObjGameInfo::from_ints(&it.data).game_state_flags),
        }
    }

    /// C++ `CControls::m_aTargetPos` (controls.cpp:424): where the mouse points in the
    /// world.
    fn target_pos(&self) -> ddnet::physics::vmath::Vec2 {
        let mouse = self.controls.mouse_pos;
        let spectating = self.spec_info().0;
        if self.item(g::NETOBJTYPE_GAMEINFO).is_some() && !spectating {
            let delta = self.camera.dyncam_target_offset - self.camera.dyncam_current_offset;
            let (x, y) = self.center.unwrap_or((0.0, 0.0));
            let zoom = self.camera.zoom;
            vec2(x, y) + mouse - delta + vec2(delta.x / zoom, delta.y / zoom)
        } else if let (true, Some(it)) = (spectating, self.item(g::NETOBJTYPE_SPECTATORINFO)) {
            let s = g::ObjSpectatorInfo::from_ints(&it.data);
            vec2(s.x as f32, s.y as f32) + mouse
        } else {
            mouse
        }
    }

    /// The name plates and freeze bars of this frame (C++ `CNamePlates::OnRender`,
    /// `CFreezeBars::OnRender`) for the world rect `screen` (top left, bottom right).
    fn plates_and_bars(
        &self,
        rendered: &[Rendered],
        config: &ddnet_rs::config::Config,
        screen: (ddnet::physics::vmath::Vec2, ddnet::physics::vmath::Vec2),
        now: i64,
    ) -> (Vec<nameplates::Plate>, Vec<freezebars::Bar>) {
        let Some(p) = &self.predictor else {
            return Default::default();
        };
        let mut chars = vec![nameplates::CharView::default(); 128];
        for r in rendered.iter().filter(|r| (0..128).contains(&r.id)) {
            chars[r.id as usize] = nameplates::CharView {
                active: true,
                direction: r.cur.direction,
                jumped: r.cur.jumped,
                strong_weak_id: r.ext.map(|e| e.strong_weak_id),
            };
        }
        let (team, other) = (|id| self.team(id), |id| self.is_other_team(id));
        let c = config;
        let input = self.client.input;
        let ctx = nameplates::Ctx {
            screen,
            clients: &self.clients,
            is_other_team: &other,
            team: &team,
            team_play: self.team_play(),
            target_pos: self.target_pos(),
            local_ids: [self.clients.local_id, -1],
            dummy: 0,
            inputs: [(input.direction, input.jump), (0, 0)],
            chars: &chars,
            spec: self.spec_info(),
            config: nameplates::PlateConfig {
                name_plates: c.cl_nameplates != 0,
                own: c.cl_nameplates_own != 0,
                friend_mark: c.cl_nameplates_friendmark != 0,
                ids: c.cl_nameplates_ids != 0,
                size: c.cl_nameplates_size,
                ids_separate_line: c.cl_nameplates_ids_seperate_line != 0,
                ids_size: c.cl_nameplates_ids_size,
                clan: c.cl_nameplates_clan != 0,
                clan_size: c.cl_nameplates_clan_size,
                strong: c.cl_nameplates_strong,
                strong_size: c.cl_nameplates_strong_size,
                direction_size: c.cl_direction_size,
                always: c.cl_nameplates_always != 0,
                offset: c.cl_nameplates_offset,
                show_others_alpha: c.cl_show_others_alpha,
                teamcolors: c.cl_nameplates_teamcolors != 0,
                show_direction: c.cl_show_direction,
                debug: false,
            },
        };
        let pos =
            |id: i32| (rendered.iter().find(|r| r.id == id)).map_or(vec2(0.0, 0.0), |r| r.pos);
        let plates = nameplates::plates(&ctx, &pos);
        if c.cl_show_freeze_bars == 0 {
            return (plates, Vec::new());
        }
        // C++ IsPlayerInfoAvailable: this and the previous snapshot have the player info
        let prev = (self.client.prev_game_tick()).and_then(|t| self.client.snapshot_at(t));
        let had_info = |id: i32| {
            prev.is_some_and(|items| {
                (items.iter()).any(|it| it.type_id == g::NETOBJTYPE_PLAYERINFO && it.id == id)
            })
        };
        let timing = self.client.timing(now);
        let prediction_tick = p.prediction_tick(&timing);
        let mut tees: Vec<freezebars::Tee> = (rendered.iter())
            .filter(|r| (0..128).contains(&r.id))
            .map(|r| freezebars::Tee {
                id: r.id,
                core: &p.predicted[r.id as usize],
                has_display_info: r.ext.is_some_and(|e| e.jumped_total != -1),
                ext_freeze_end: r.ext.map_or(0, |e| e.freeze_end),
                tick: if r.predicted_local {
                    timing.pred_tick
                } else if r.predicted {
                    prediction_tick
                } else {
                    timing.game_tick
                },
                pos: r.pos,
                other_team: self.is_other_team(r.id),
                available: self.clients.infos[r.id as usize].is_some() && had_info(r.id),
            })
            .collect();
        tees.sort_by_key(|t| t.id);
        let cfg = freezebars::BarConfig {
            alpha_inside_freeze: c.cl_freezebars_alpha_inside_freeze,
            show_others_alpha: c.cl_show_others_alpha,
        };
        let bars = freezebars::bars(&tees, self.clients.local_id, screen, &cfg);
        (plates, bars)
    }

    /// C++: the `m_LastRaceTick` part of `OnNewSnapshot` (gameclient.cpp:2459).
    fn update_race_tick(&mut self) {
        let Some(p) = &self.predictor else { return };
        let (version, flags, flags2) = p.game_info;
        // C++ m_GameInfo.m_Race: the flag from GameInfoEx version 3 on, else the game
        // type's race (what GetGameInfo computes before the version-2 entity flags)
        let race = if version >= 3 {
            flags & g::GAMEINFOFLAG_RACE != 0
        } else {
            ddnet_rs::render::entities::EntitiesInfo::from_game_info((
                version.min(1),
                flags,
                flags2,
            ))
            .race
        };
        let local = self.client.local_id();
        let has_char = |items: &[ddnet::protocol::snapshot::Item]| {
            (items.iter()).any(|it| it.type_id == g::NETOBJTYPE_CHARACTER && Some(it.id) == local)
        };
        let prev = (self.client.prev_game_tick()).and_then(|t| self.client.snapshot_at(t));
        if let Some(info) = self.item(g::NETOBJTYPE_GAMEINFO)
            && race
            && self.client.state() == State::Online
            && !self.spec_info().0
            && has_char(self.client.snapshot())
            && prev.is_some_and(has_char)
        {
            let info = g::ObjGameInfo::from_ints(&info.data);
            let race_flag = info.game_state_flags & g::GAMESTATEFLAG_RACETIME != 0;
            self.last_race_tick = if race_flag { -info.warmup_timer } else { -1 };
        }
    }

    /// C++ `CGameClient::CurrentRaceTime` (seconds).
    fn current_race_time(&self) -> i32 {
        if self.last_race_tick < 0 {
            return 0;
        }
        // C++ GameTickSpeed(): SERVER_TICK_SPEED
        (self.client.game_tick().unwrap_or(0) - self.last_race_tick) / 50
    }

    /// What the Esc bar's Game tab reads (C++ `CMenus::RenderGame`).
    fn esc_state(&self) -> esc_menu::GameState {
        let c = &self.clients;
        let local = usize::try_from(c.local_id).ok().map(|id| &c.clients[id]);
        esc_menu::GameState {
            has_local_id: local.is_some(),
            local_team: c.infos.iter().flatten().find(|i| i.local).map(|i| i.team),
            game_info: self.item(g::NETOBJTYPE_GAMEINFO).is_some(),
            paused: local.is_some_and(|l| l.paused),
            spec: local.is_some_and(|l| l.spec),
            team_play: self.team_play(),
            received_ddnet_player: c.received_ddnet_player,
            show_ddrace_buttons: true,
            dummy_connecting: false,
        }
    }

    /// C++ `IsTeamPlay()`: `GAMEFLAG_TEAMS` in the snapshot's game info.
    fn team_play(&self) -> bool {
        (self.client.snapshot().iter())
            .find(|it| it.type_id == g::NETOBJTYPE_GAMEINFO)
            .is_some_and(|it| {
                g::ObjGameInfo::from_ints(&it.data).game_flags & g::GAMEFLAG_TEAMS != 0
            })
    }

    fn camera_frame(&self, start: Instant, frame_time: f32) -> Frame {
        Frame {
            local_time: start.elapsed().as_secs_f32(),
            frame_time,
            allow_zoom: (self.predictor.as_ref()).is_some_and(|p| p.allow_zoom || p.spectating),
        }
    }

    /// C++: `CGameClient::UpdatePositions` (`m_LocalCharacterPos`): the rendered
    /// position of the followed or local tee, else the last one. The followed tee falls
    /// back to ours while the server does not send it (it sends far tees only with
    /// `sv_show_all_default 1`); without that the world stayed grey.
    fn local_pos(&mut self, rendered: &[Rendered], follow: Option<i32>) -> Option<(f32, f32)> {
        let find = |id: Option<i32>| rendered.iter().find(|r| Some(r.id) == id);
        if let Some(r) = find(follow).or_else(|| find(self.client.local_id())) {
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
    /// The in-game menu (Esc).
    esc: EscMenu,
    /// The HUD widgets.
    hud: Vec<Box<dyn Widget>>,
    /// C++ `CChat` (kept across connections like its sent history; reset on connect).
    chat: Chat,
    /// C++ `CSounds` and the sound engine.
    sounds: Sounds,
    /// What `CMenus::UpdateMusicState` last saw (offline, `snd_enable`, `snd_enable_music`).
    music_state: Option<(bool, i32, i32)>,
    /// Tee heads for the chat lines and the scoreboard.
    chat_heads: TeeHeads,
    /// C++ `CScoreboard::m_Active` (`+scoreboard` held).
    scoreboard_held: bool,
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
        let packs = assets::Packs::from_config(&self.settings.config);
        let players = Players::new(&mut gfx, &self.storage, &packs);
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
            packs,
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

    /// What the Esc bar's choice does (C++ `RenderGame`'s buttons and `SetActive(false)`).
    fn on_esc_action(&mut self, a: esc_menu::Action) {
        use esc_menu::Action as A;
        let now = net::now(self.start);
        match a {
            A::SetTeam(team) => {
                if let Some(s) = &mut self.session {
                    s.client.send_switch_team(team, now);
                }
                self.menu.open = false;
            }
            A::Kill => {
                if let Some(s) = &mut self.session {
                    s.client.send_kill(now);
                }
                self.menu.open = false;
            }
            A::Pause => {
                // C++: Console()->ExecuteLine("say /pause")
                self.chat.send_chat(0, "/pause", now);
                self.menu.open = false;
            }
            A::Disconnect | A::Quit => self.disconnect(),
            A::Settings => self.menu.open_settings_page(),
            A::EditHud => self.open_editor(),
            A::ToggleChatIgnore(id) => {
                let s = self.session.as_mut();
                if let Some(c) = s.and_then(|s| s.clients.clients.get_mut(id as usize)) {
                    c.chat_ignore = !c.chat_ignore;
                }
            }
            A::ToggleFriend(id) => {
                let Some(s) = &mut self.session else { return };
                let Some(c) = s.clients.clients.get_mut(id as usize) else {
                    return;
                };
                let friends = &mut self.settings.friends;
                if c.friend {
                    friends.remove(&c.name, &c.clan);
                } else {
                    friends.add(&c.name, &c.clan);
                }
                // C++ updates m_Friend with the next snapshot; show it at once
                let ignore_clan = self.settings.config.cl_friends_ignore_clan != 0;
                c.friend = friends.is_friend(&c.name, &c.clan, true, ignore_clan);
                self.save_settings();
            }
        }
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
            Ok(s) => {
                // C++: CChat::OnStateChange -> Reset
                self.chat.reset();
                self.session = Some(s)
            }
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
        let Some(mut s) = self.session.take() else {
            return self.close_menus();
        };
        // C++: CMapSounds::OnStateChange -> Clear
        if let Some(ms) = s.map_sounds.take() {
            ms.clear(&mut self.sounds);
        }
        if let (Some((map, _)), Some(r)) = (s.map, &mut self.running) {
            map.free(&mut r.gfx);
        }
        self.close_menus();
    }

    fn close_menus(&mut self) {
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
        // C++: the chat comes before menus and binds in the input stack and takes every
        // key event while it is open (chat.cpp:271); typing goes to its text field
        if self.chat.is_active()
            && let Some(s) = &self.session
        {
            if press {
                self.pressed[key] = true;
            }
            if release {
                self.pressed[key] = false;
            }
            if press {
                let k = match key {
                    43 => chat::Key::Tab,
                    40 | 88 => chat::Key::Enter,
                    KEY_ESCAPE => chat::Key::Escape,
                    82 => chat::Key::Up,
                    81 => chat::Key::Down,
                    KEY_LSHIFT | KEY_RSHIFT => chat::Key::Shift,
                    _ => chat::Key::Other,
                };
                let shift = self.pressed[KEY_LSHIFT] || self.pressed[KEY_RSHIFT];
                let config = &self.settings.config;
                let (team, other) = (|id| s.team(id), |id| s.is_other_team(id));
                let ctx = s.chat_ctx(config, &team, &other);
                let now = net::now(self.start);
                // C++ GameClient()->OnRelease(): the scoreboard lets go too
                if self
                    .chat
                    .on_key(k, shift, &ctx, config.cl_chat_reset != 0, now)
                {
                    self.scoreboard_held = false;
                }
            }
            return;
        }
        if press && key == KEY_ESCAPE {
            if self.editor.open {
                self.close_editor();
            } else if self.player_open {
                self.player_open = false;
            } else if self.session.is_some() && self.menu.open && self.menu.settings_open() {
                // in game the settings page goes back to the Esc bar
                self.menu.close_settings();
                self.save_settings();
            } else {
                self.menu.open = !self.menu.open;
                self.esc.reset();
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
        let now = net::now(self.start);
        for (stroke, line) in lines {
            let config = &self.settings.config;
            s.controls
                .execute(stroke, &line, game, &mut s.camera, &frame, config);
            for (command, args) in split_line(&line) {
                // C++: CChat::ConShowChat, the one chat command with a release
                if command == "+show_chat" {
                    self.chat.show = stroke;
                }
                // C++: CScoreboard::ConKeyScoreboard
                if command == "+scoreboard" {
                    self.scoreboard_held = stroke;
                }
                if !stroke {
                    continue;
                }
                let mut a = Vec::new();
                match command.as_str() {
                    // C++: CChat::ConChat, ConSay, ConSayTeam, ConEcho, ConClearChat
                    "chat" if ddnet_rs::config::parse_args(&args, "s?r", &mut a) => {
                        self.chat.con_chat(&a, config.cl_chat_reset != 0, false)
                    }
                    "say" | "say_team" if ddnet_rs::config::parse_args(&args, "r", &mut a) => self
                        .chat
                        .send_chat(i32::from(command == "say_team"), &a[0], now),
                    "echo" if ddnet_rs::config::parse_args(&args, "r", &mut a) => {
                        let (team, other) = (|id| s.team(id), |id| s.is_other_team(id));
                        let ctx = s.chat_ctx(config, &team, &other);
                        self.chat.echo(&a[0], &ctx, now);
                    }
                    "clear_chat" => self.chat.clear_lines(),
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
        // C++ SnapInput: chatting first, then a menu
        let state = if self.chat.is_active() {
            PlayerState::Chatting
        } else if menu_visible {
            PlayerState::InMenu
        } else {
            PlayerState::Playing
        };
        let on_death = config.cl_scoreboard_on_death != 0;
        let scoreboard = scoreboard::is_active(&s.scoreboard_state(self.scoreboard_held, on_death));
        s.client.input = s.controls.snap_input(state, scoreboard, bug_input);
        if let (Some(weapon), Some(shown)) = (self.options.fire, s.map_shown_at) {
            let elapsed = shown.elapsed().as_secs_f64();
            if elapsed > 2.0 && !std::mem::replace(&mut s.weapons_asked, true) {
                s.client.say(0, "/weapons", net::now(start));
            }
            // C++ CControls: the fire counter counts presses and releases
            s.client.input.fire = (elapsed * 10.0) as i32 & ddnet::physics::core::INPUT_STATE_MASK;
            s.client.input.wanted_weapon = weapon + 1;
            // into the open: up and to the left
            (s.client.input.target_x, s.client.input.target_y) = (-200, -60);
        }
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
            return Err(format!("disconnected: '{}'", s.client.conn().error()));
        }
        // C++: CSounds::OnStateChange(STATE_ONLINE) -> OnReset
        if s.client.state() == State::Online && !std::mem::replace(&mut s.online, true) {
            self.sounds.reset();
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
            let mut warning = false;
            s.map_sounds = Some(MapSounds::load(
                &m,
                &self.storage,
                &mut self.sounds,
                &mut warning,
            ));
            if warning {
                eprintln!("Some map sounds could not be loaded.");
            }
            s.map_shown_at = Some(Instant::now());
            if let Some(tab) = self.options.esc_tab {
                self.menu.open = true;
                self.esc.tab = tab;
            }
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
        // C++: the cl_asset_* change callbacks reload the sprites (and our HUD icons)
        let packs = assets::Packs::from_config(&self.settings.config);
        if packs != r.packs {
            r.players.set_packs(&mut r.gfx, &self.storage, &packs);
            if packs.game != r.packs.game {
                // ponytail: rebuilt widgets drop the kill feed's lines; keep them if it
                // matters (pack changes happen in the menu)
                for w in &mut self.hud {
                    match w.id() {
                        "status" => *w = Box::new(StatusBar::new(&self.storage, &packs.game)),
                        "killfeed" => *w = Box::new(Killfeed::new(&self.storage, &packs.game)),
                        _ => {}
                    }
                }
            }
            r.packs = packs;
        }
        let mut batch = Batch::default();
        // C++ OnRender's clear colour: cl_background_entities_color with an entities overlay
        let bg = match self.settings.config.cl_overlay_entities {
            0 => BACKGROUND_COLOR,
            _ => self.settings.config.cl_background_entities_color,
        };
        let [cr, cg, cb] = hsl_packed_to_rgb(bg);
        batch.clear(cr, cg, cb);
        let mut center = None;
        let mut rendered = Vec::new();
        let mut events = Vec::new();
        let mut map_shown_at = None;
        let mut world_view = None;
        let (mut plates, mut bars) = (Vec::new(), Vec::new());
        if let Some(s) = &mut self.session {
            // C++: CGameClient::OnNewSnapshot/OnPredict, then UpdateRenderedCharacters
            let now = net::now(self.start);
            // this frame's game messages: C++ handles them as they arrive, before the
            // snapshot switch (CClient::Update pumps the network first)
            events = std::mem::take(&mut s.client.messages);
            for msg in &events {
                // C++: Sv_Record / Sv_RecordLegacy carry the map's best time
                let cfg = &self.settings.config;
                match msg {
                    g::GameMsg::SvRecord(m) => s.clients.on_record(m.server_time_best),
                    g::GameMsg::SvRecordLegacy(m) => s.clients.on_record(m.server_time_best),
                    g::GameMsg::SvMotd(m) => s.motd = esc_menu::motd_text(&m.message),
                    // gameclient.cpp:1148
                    g::GameMsg::SvSoundGlobal(m) if cfg.snd_game != 0 => {
                        use ddnet::protocol::sounds::*;
                        let ctf = [
                            SOUND_CTF_DROP,
                            SOUND_CTF_RETURN,
                            SOUND_CTF_CAPTURE,
                            SOUND_CTF_GRAB_EN,
                            SOUND_CTF_GRAB_PL,
                        ];
                        if ctf.contains(&m.sound_id) {
                            self.sounds.enqueue(sounds::CHN_GLOBAL, m.sound_id);
                        } else {
                            self.sounds.play(sounds::CHN_GLOBAL, m.sound_id, 1.0, cfg);
                        }
                    }
                    // gameclient.cpp:1228
                    g::GameMsg::SvMapSoundGlobal(m) => {
                        if let Some(ms) = &s.map_sounds {
                            ms.play(&mut self.sounds, sounds::CHN_GLOBAL, m.sound_id, None, cfg);
                        }
                    }
                    _ => {}
                }
            }
            {
                let config = &self.settings.config;
                let (team, other) = (|id| s.team(id), |id| s.is_other_team(id));
                let ctx = s.chat_ctx(config, &team, &other);
                for msg in &events {
                    if let Some(line) = self.chat.on_message(msg, &ctx, now) {
                        store_save(&line, s.client.map().map_or("", |m| m.name.as_str()));
                    }
                }
            }
            let local_freeze_end = match &mut s.predictor {
                Some(p) => {
                    // C++ reads g_Config live
                    p.config = self.settings.config.predict_config();
                    s.client.prediction_margin = self.settings.config.cl_prediction_margin;
                    p.update(&mut s.client, now);
                    let cfg = &self.settings.config;
                    for e in std::mem::take(&mut p.sound_events) {
                        match e {
                            SoundEvent::World(set, pos) => {
                                self.sounds.play_at(sounds::CHN_WORLD, set, 1.0, pos, cfg);
                            }
                            SoundEvent::MapWorld(id, pos) => {
                                if let Some(ms) = &s.map_sounds {
                                    let ch = sounds::CHN_WORLD;
                                    ms.play(&mut self.sounds, ch, id, Some(pos), cfg);
                                }
                            }
                        }
                    }
                    rendered = p.rendered(&s.client.timing(now));
                    p.predicted_freeze_end()
                }
                None => None,
            };
            // C++: OnNewSnapshot's client data and player lists
            if s.client.game_tick() != s.clients_tick
                && let Some(p) = &s.predictor
            {
                s.clients_tick = s.client.game_tick();
                let ctx = SnapContext {
                    allow_x_skins: p.allow_x_skins,
                    friends: &self.settings.friends,
                    foes: &self.settings.foes,
                    friends_ignore_clan: self.settings.config.cl_friends_ignore_clan != 0,
                };
                s.clients
                    .on_new_snapshot(s.client.snapshot(), &ctx, p.time_score);
                s.update_race_tick();
            }
            // C++: CChat::OnRender's queued send; the lines to send go out as Cl_Say
            self.chat.on_render(now);
            for (team, line) in self.chat.sends.drain(..) {
                s.client.say(team, &line, now);
            }
            // C++ CChat::AddLine's sounds (chat.cpp:891-936)
            for kind in self.chat.sounds.drain(..) {
                use ddnet::protocol::sounds::*;
                let set = match kind {
                    chat::Sound::Server => SOUND_CHAT_SERVER,
                    chat::Sound::Highlight => SOUND_CHAT_HIGHLIGHT,
                    chat::Sound::Client => SOUND_CHAT_CLIENT,
                };
                self.sounds
                    .play(sounds::CHN_GUI, set, 1.0, &self.settings.config);
            }
            // C++: CGameClient::OnRender: UpdatePositions, UpdateCamera, CCamera::OnRender
            let local = s.local_pos(&rendered, self.options.follow);
            let frame = s.camera_frame(self.start, frame_time);
            let config = &self.settings.config;
            s.camera.update(s.controls.mouse_pos, &frame, config);
            if let Some((x, y)) = local {
                s.camera.on_render(&mut s.controls, vec2(x, y), config);
            }
            // C++: CMapSounds::OnRender (online only)
            if s.client.state() == State::Online && s.map_sounds.is_some() {
                let round_start = s
                    .item(g::NETOBJTYPE_GAMEINFO)
                    .map(|it| ddnet::items::pad(&it.data, 3)[2]);
                let t = s.client.timing(now);
                let ticks = (t.prev_game_tick, t.game_tick, t.intra_tick, round_start);
                let (center, time) = (s.camera.center, s.envelope_time());
                if let Some(ms) = &mut s.map_sounds {
                    ms.on_render(&mut self.sounds, ticks, center, time, config);
                }
            }
            center = match (self.options.view, local) {
                (Some(v), _) => Some(v),
                (None, Some(_)) => Some((s.camera.center.x, s.camera.center.y)),
                _ => None,
            };
            map_shown_at = s.map_shown_at;
            self.shown_frames += map_shown_at.is_some() as u32;
            let size = size_of(&r.config);
            // C++: CMapImages::GetEntities with the server's mod and mask
            if let (Some((map, _)), Some(p)) = (&mut s.map, &s.predictor) {
                let info = ddnet_rs::render::entities::EntitiesInfo::from_game_info(p.game_info);
                let cfg = &self.settings.config;
                let text = (cfg.cl_text_entities != 0).then_some(cfg.cl_text_entities_size);
                map.set_entities(
                    &mut r.gfx,
                    &self.storage,
                    &cfg.cl_assets_entities,
                    &info,
                    text,
                );
            }
            if let (Some((map, collision)), Some(center)) = (&s.map, center) {
                let view = View {
                    center,
                    zoom: s.camera.zoom,
                    width: size.width,
                    height: size.height,
                    envelope_time: s.envelope_time(),
                    overlay: self.settings.config.cl_overlay_entities,
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
                            timing: s.client.timing(now),
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
                    (s.predictor.as_ref()).is_some_and(|p| p.antiping_gunfire()),
                );
                for &pos in &r.players.skids {
                    self.sounds.skid(pos, 1.0, &self.settings.config);
                }
                map.render_foreground(&mut batch, &view);
                // C++ CHud::RenderCursor (the last thing the HUD draws, over the map)
                let local = s.client.local_id();
                // C++ m_LocalCharacterPos: our own tee (not a --follow'ed one)
                let own = rendered.iter().find(|t| Some(t.id) == local);
                if let (Some(p), Some(id), Some(own)) = (&s.predictor, local, own)
                    && !p.spectating
                {
                    let cam = &s.camera;
                    let target = ddnet_rs::render::cursor::target_pos(
                        own.pos,
                        s.controls.mouse_pos,
                        cam.dyncam_target_offset,
                        cam.dyncam_current_offset,
                        cam.zoom,
                    );
                    let weapon = p.predicted[id as usize].active_weapon;
                    let aspect = size.width as f32 / size.height as f32;
                    let sprites = r.players.sprites();
                    ddnet_rs::render::cursor::render(
                        &mut batch, sprites, weapon, target, cam.center, aspect,
                    );
                }
                // the game layer's world rect (C++ Graphics()->GetScreen() there)
                let aspect = size.width as f32 / size.height as f32;
                let rect = ddnet_rs::render::map_screen_to_world(
                    center.0,
                    center.1,
                    100.0,
                    100.0,
                    100.0,
                    0.0,
                    0.0,
                    aspect,
                    s.camera.zoom,
                );
                let screen = (vec2(rect[0], rect[1]), vec2(rect[2], rect[3]));
                (plates, bars) = s.plates_and_bars(&rendered, &self.settings.config, screen, now);
                world_view = Some(rect);
            }
        }
        // C++ CMenus::UpdateMusicState (on its triggers) and CSounds::OnRender
        let cfg = &self.settings.config;
        let music = (self.session.is_none(), cfg.snd_enable, cfg.snd_enable_music);
        if self.music_state != Some(music) {
            self.music_state = Some(music);
            let should_play = music.0 && music.1 != 0 && music.2 != 0;
            let menu = ddnet::protocol::sounds::SOUND_MENU;
            let playing = self.sounds.is_playing(menu);
            if should_play && !playing {
                self.sounds.enqueue(sounds::CHN_MUSIC, menu);
            } else if !should_play && playing {
                self.sounds.stop(menu);
            }
        }
        let listener = (self.session.as_ref()).map_or(vec2(0.0, 0.0), |s| s.camera.center);
        self.sounds.on_render(listener, self.focused, cfg);
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
        let (mut action, mut esc_action) = (None, None);
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
        // the chat draws once the map is there, over the HUD
        let chat_session = self.session.as_ref().filter(|s| s.map.is_some());
        if menu_visible || data.is_some() || chat_session.is_some() {
            let (menu, settings, widgets) = (&mut self.menu, &mut self.settings, &mut self.hud);
            let esc = &mut self.esc;
            let (editor, chat, heads) = (&mut self.editor, &mut self.chat, &mut self.chat_heads);
            let now = net::now(self.start);
            let held = self.scoreboard_held;
            let aspect = size.width as f32 / size.height as f32;
            r.ui.frame(&r.window, &r.gfx, &target, size, |ctx| {
                // world things under the HUD: freeze bars, then name plates
                if let (Some(world), Some(_)) = (world_view, chat_session) {
                    let view = nameplates::WorldView {
                        world,
                        screen: ctx.content_rect(),
                    };
                    freezebars::show(ctx, &bars, &view);
                    nameplates::show(ctx, &plates, &view);
                }
                match &data {
                    Some(data) if editor.open => {
                        editor_done = editor.show(ctx, widgets, data, &mut settings.hud);
                    }
                    Some(data) => hud::show(ctx, widgets, data, &settings.hud),
                    None => {}
                }
                if let Some(s) = chat_session
                    && !editor.open
                {
                    let team = |id| s.team(id);
                    let on_death = settings.config.cl_scoreboard_on_death != 0;
                    let board = scoreboard::is_active(&s.scoreboard_state(held, on_death));
                    let glass = Glass::from_setting(settings.config.rs_glass);
                    if board {
                        let (spectating, followed) = s.spec_info();
                        let local = s.clients.local_id;
                        let mut view = scoreboard::View {
                            glass,
                            clients: &s.clients,
                            ddteam: &team,
                            max_team_size: s.predictor.as_ref().map_or(128, |p| p.max_team_size),
                            time_score: s.predictor.as_ref().is_some_and(|p| p.time_score),
                            game_info: (s.item(g::NETOBJTYPE_GAMEINFO))
                                .map(|it| g::ObjGameInfo::from_ints(&it.data)),
                            game_data: (s.item(g::NETOBJTYPE_GAMEDATA))
                                .map(|it| g::ObjGameData::from_ints(&it.data)),
                            map: s.client.map().map_or("", |m| m.name.as_str()),
                            highlight: if spectating && followed >= 0 {
                                followed
                            } else {
                                local
                            },
                            heads,
                            ping_color: settings.config.cl_enable_ping_color != 0,
                        };
                        scoreboard::show(ctx, &mut view);
                    }
                    let view = chat::View {
                        glass,
                        now,
                        config: &settings.config,
                        clients: &s.clients,
                        team: &team,
                        // C++: only on a wide screen (aspect > 1.7)
                        scoreboard_open: board && aspect > 1.7,
                    };
                    chat::show(ctx, chat, &view, heads);
                }
                if menu_visible {
                    action = menu.show(ctx, connected, settings);
                }
                if let Some(s) = chat_session
                    && menu_visible
                    && !menu.settings_open()
                {
                    let player_name = player_info(settings).name;
                    let mut view = esc_menu::View {
                        glass: Glass::from_setting(settings.config.rs_glass),
                        game: s.esc_state(),
                        clients: &s.clients,
                        show_chat_friends: settings.config.cl_show_chat_friends != 0,
                        heads,
                        server: s.client.server_info(),
                        address: &s.server,
                        player_name: &player_name,
                        map: s.client.map().map_or("", |m| m.name.as_str()),
                        game_info: (s.item(g::NETOBJTYPE_GAMEINFO))
                            .map(|it| g::ObjGameInfo::from_ints(&it.data)),
                        motd: &s.motd,
                        confirm_disconnect: {
                            let limit = settings.config.cl_confirm_disconnect_time;
                            limit >= 0 && s.current_race_time() / 60 >= limit
                        },
                    };
                    esc_action = esc.show(ctx, &mut view);
                }
            });
        }
        if !menu_visible && !self.editor.open && !self.player_open && !self.chat.is_active() {
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
            Some(Action::SettingsDone) => self.save_settings(),
            Some(Action::Quit) => {
                self.disconnect();
                event_loop.exit();
            }
            None => {}
        }
        if let Some(a) = esc_action {
            self.on_esc_action(a);
            if a == esc_menu::Action::Quit {
                event_loop.exit();
            }
        }
        if editor_done {
            self.close_editor();
        }
    }
}

/// C++: the file part of `CChat::StoreSave` (chat.cpp:656): a "Team successfully saved"
/// line appended to `ddnet-saves.txt` in our own save dir (PLAN §13.13 answer 6).
fn store_save(line: &str, map: &str) {
    let (Some((name, code)), Some(dir)) = (chat::parse_save(line), storage::save_dir()) else {
        return;
    };
    let path = dir.join("ddnet-saves.txt");
    let mut text = String::new();
    if !path.exists() {
        text += &chat::csv_line(&["Time", "Player", "Map", "Code"]);
    }
    let time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    text += &chat::csv_line(&[&time, &name, map, &code]);
    use std::io::Write;
    let file = std::fs::create_dir_all(&dir).and_then(|()| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
    });
    if let Err(e) = file.and_then(|mut f| f.write_all(text.as_bytes())) {
        eprintln!("saving the save code failed: {e}");
    }
}

/// The keys `CChat::OnInput` handles itself (Tab, Enter, Escape, Up, Down): kept from
/// the text field, which would otherwise move focus or the cursor with them.
fn chat_key(event: &WindowEvent) -> bool {
    use KeyCode::*;
    matches!(
        event,
        WindowEvent::KeyboardInput {
            event: KeyEvent {
                physical_key: PhysicalKey::Code(
                    Tab | Enter | NumpadEnter | Escape | ArrowUp | ArrowDown
                ),
                ..
            },
            ..
        }
    )
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
            || (self.ui_active() && self.menu.reading_key.is_none())
            || (self.chat.is_active() && !chat_key(&event));
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
        fire: None,
        esc_tab: None,
    };
    let mut args = std::env::args().skip(1).peekable();
    if let Some(a) = args.next_if(|a| !a.starts_with("--")) {
        let server: SocketAddr = a.parse().map_err(|_| "usage: ddnet-rs [server ip:port] [--data-dir PATH] [--vsync] [--size WxH] [--view X,Y] [--follow CLIENT_ID] [--screenshot-after SECONDS] [--no-grab] [--fire WEAPON] [--esc-tab TAB]")?;
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
            "--fire" => o.fire = Some(value()?.parse().map_err(|_| "--fire WEAPON")?),
            "--esc-tab" => o.esc_tab = Some(value()?.parse().map_err(|_| "--esc-tab TAB")?),
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
        Box::new(StatusBar::new(&storage, &settings.config.cl_asset_game)),
        Box::new(Movement),
        Box::new(Killfeed::new(&storage, &settings.config.cl_asset_game)),
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
    let sounds = Sounds::new(&storage, &settings.config);
    let mut app = App {
        sounds,
        music_state: None,
        menu: Menu::new(options.server, &storage),
        esc: EscMenu::default(),
        hud,
        chat: Chat::default(),
        chat_heads: TeeHeads::new(&storage),
        scoreboard_held: false,
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
