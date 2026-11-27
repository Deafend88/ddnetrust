//! Minimal menus (M6.8): connect by address, basic settings, quit. Drawn with egui over
//! the composited world; the full menu design (C++ `CMenus`) comes later.

use std::net::SocketAddr;

use winit::window::Window;

use crate::config::{Binds, Config, Settings, escape};
use crate::gfx::{Gfx, Size};
use crate::glass::{GlassFrame, GlassRenderer};
use crate::icons;
use crate::storage::Storage;
use crate::style::{self, Glass, Weight, color, size, space};
use crate::widgets as w;

/// egui and its wgpu painter, without a window (the gallery draws headless).
pub struct Painter {
    pub ctx: egui::Context,
    renderer: egui_wgpu::Renderer,
    glass_renderer: GlassRenderer,
    glass: Glass,
}

impl Painter {
    pub fn new(gfx: &Gfx, format: wgpu::TextureFormat, glass: Glass) -> Self {
        let ctx = egui::Context::default();
        style::install(&ctx, glass);
        let renderer = egui_wgpu::Renderer::new(&gfx.device, format, Default::default());
        Self {
            ctx,
            renderer,
            glass_renderer: GlassRenderer::new(&gfx.device, format),
            glass,
        }
    }

    /// Changes the glass strength (`rs_glass`): the style's fills and the blur.
    pub fn set_glass(&mut self, glass: Glass) {
        style::install(&self.ctx, glass);
        self.glass = glass;
    }

    /// Runs `build` for one frame on `input` and draws the result onto `target`; glass
    /// panels show the blurred world target of `gfx` (`target` or the window).
    pub fn paint(
        &mut self,
        gfx: &Gfx,
        target: &wgpu::TextureView,
        size: Size,
        input: egui::RawInput,
        mut build: impl FnMut(&egui::Context),
    ) -> egui::PlatformOutput {
        let mut output = self.ctx.run_pass(input, |ctx| build(ctx));
        let ppp = output.pixels_per_point;
        let prims = self.ctx.tessellate(std::mem::take(&mut output.shapes), ppp);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [size.width, size.height],
            pixels_per_point: ppp,
        };
        // egui checks that every delta was applied (drained)
        for (id, delta) in output.textures_delta.set.drain() {
            for d in delta {
                self.renderer
                    .update_texture(&gfx.device, &gfx.queue, id, &d);
            }
        }
        let mut encoder = gfx.device.create_command_encoder(&Default::default());
        // the blur goes first: the panels' callbacks read it from `prepare` on
        let frame = self.glass_renderer.blur(
            &gfx.device,
            &mut encoder,
            &gfx.world_view(),
            gfx.world_size(),
            self.glass,
        );
        match frame {
            Some(f) => self.renderer.callback_resources.insert(f),
            None => self.renderer.callback_resources.remove::<GlassFrame>(),
        };
        let mut buffers =
            self.renderer
                .update_buffers(&gfx.device, &gfx.queue, &mut encoder, &prims, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            self.renderer.render(&mut pass, &prims, &screen);
        }
        buffers.push(encoder.finish());
        gfx.queue.submit(buffers);
        for id in output.textures_delta.free.drain() {
            self.renderer.free_texture(&id);
        }
        std::mem::take(&mut output.platform_output)
    }
}

/// The painter with its winit input.
pub struct Ui {
    pub painter: Painter,
    state: egui_winit::State,
}

impl Ui {
    pub fn new(window: &Window, gfx: &Gfx, format: wgpu::TextureFormat, glass: Glass) -> Self {
        let painter = Painter::new(gfx, format, glass);
        let state = egui_winit::State::new(
            painter.ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            None,
            None,
            None,
        );
        Self { painter, state }
    }

    /// Feeds a window event to egui; true if egui wants it for itself (typing, clicks).
    pub fn on_window_event(&mut self, window: &Window, event: &winit::event::WindowEvent) -> bool {
        self.state.on_window_event(window, event).consumed
    }

    /// Called on frames without menus: egui runs no passes then, so its focus would
    /// survive and land on whatever widget has the same auto id when a menu shows again
    /// (C++ menus open with nothing focused).
    pub fn hidden(&self) {
        self.painter.ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
    }

    /// Runs `build` for one frame and draws the result onto `target`.
    pub fn frame(
        &mut self,
        window: &Window,
        gfx: &Gfx,
        target: &wgpu::TextureView,
        size: Size,
        build: impl FnMut(&egui::Context),
    ) {
        let input = self.state.take_egui_input(window);
        let output = self.painter.paint(gfx, target, size, input, build);
        self.state.handle_platform_output(window, output);
    }
}

/// The main menu's width: one column of full-width controls.
const MENU_WIDTH: f32 = 320.0;

/// The bindable controls shown in the settings, as (label, command).
const CONTROLS: [(&str, &str); 7] = [
    ("Move left", "+left"),
    ("Move right", "+right"),
    ("Jump", "+jump"),
    ("Hook", "+hook"),
    ("Fire", "+fire"),
    ("HUD editor", "hud_editor"),
    ("Open media player", "media_player"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Main,
    Settings,
}

/// What the player chose in the menu this frame.
pub enum Action {
    Connect(SocketAddr),
    Disconnect,
    Resume,
    Quit,
    /// Leaving the settings page: save them.
    SettingsDone,
    EditHud,
}

pub struct Menu {
    /// The in-game menu (Esc); without a connection the start screen always shows.
    pub open: bool,
    page: Page,
    address: String,
    /// The last connection error, shown on the start screen.
    pub error: Option<String>,
    /// The command waiting for a key press in the settings (C++ key reader).
    pub reading_key: Option<&'static str>,
    skins: Vec<String>,
    /// The prediction details (secondary antiping settings) are shown.
    prediction_details: bool,
    storage: Storage,
    /// The asset kind whose pack grid is open (an index of `ASSET_KINDS`).
    picking: Option<usize>,
    /// The packs of each kind, listed when its grid first opens.
    packs: [Option<Vec<String>>; ASSET_KINDS.len()],
    /// Pack thumbnails by (kind, name); `None` where the image does not load.
    thumbs: std::collections::HashMap<(usize, String), Option<egui::TextureHandle>>,
}

/// The asset kinds the picker offers: (label, variable, asset kind; `None` for the
/// entities). The HUD pack is left out: our HUD draws its own icons (DECISIONS 13.4).
type AssetKind = (
    &'static str,
    &'static str,
    Option<(&'static str, &'static str)>,
);
const ASSET_KINDS: [AssetKind; 5] = [
    ("Game", "cl_asset_game", Some(crate::assets::GAME)),
    (
        "Emoticons",
        "cl_asset_emoticons",
        Some(crate::assets::EMOTICONS),
    ),
    (
        "Particles",
        "cl_asset_particles",
        Some(crate::assets::PARTICLES),
    ),
    ("Extras", "cl_asset_extras", Some(crate::assets::EXTRAS)),
    ("Entities", "cl_assets_entities", None),
];

/// A pack's image scaled to `height` px (box filter, alpha-weighted), for a thumbnail.
fn thumbnail(img: &crate::image::Image, height: usize) -> egui::ColorImage {
    let rgba = img.to_rgba();
    let (w0, h0) = (img.width as usize, img.height as usize);
    let h = height.min(h0).max(1);
    let w = (w0 * h / h0).max(1);
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let (mut sum, mut n) = ([0u64; 4], 0u64);
            for sy in y * h0 / h..((y + 1) * h0 / h).max(y * h0 / h + 1) {
                for sx in x * w0 / w..((x + 1) * w0 / w).max(x * w0 / w + 1) {
                    let p = &rgba[(sy * w0 + sx) * 4..][..4];
                    let a = p[3] as u64;
                    for c in 0..3 {
                        sum[c] += p[c] as u64 * a;
                    }
                    sum[3] += a;
                    n += 1;
                }
            }
            let a = sum[3].max(1);
            out.extend([0, 1, 2].map(|c| (sum[c] / a) as u8));
            out.push((sum[3] / n) as u8);
        }
    }
    egui::ColorImage::from_rgba_unmultiplied([w, h], &out)
}

impl Menu {
    pub fn new(address: Option<SocketAddr>, storage: &Storage) -> Self {
        Self {
            open: false,
            page: Page::Main,
            address: address.map_or("127.0.0.1:8303".into(), |a| a.to_string()),
            error: None,
            reading_key: None,
            skins: crate::assets::list_skins(storage),
            prediction_details: false,
            storage: storage.clone(),
            picking: None,
            packs: Default::default(),
            thumbs: Default::default(),
        }
    }

    /// Opens the settings page, with the prediction details shown or not (the gallery's
    /// settings review).
    pub fn open_settings(&mut self, prediction_details: bool) {
        self.page = Page::Settings;
        self.prediction_details = prediction_details;
    }

    /// Opens the settings page with the pack grid of asset kind `kind` open (the
    /// gallery's asset picker review).
    pub fn open_assets(&mut self, kind: usize) {
        self.page = Page::Settings;
        self.picking = Some(kind);
    }

    /// The start screen (`connected` false) or the in-game menu, and the settings page.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        connected: bool,
        settings: &mut Settings,
    ) -> Option<Action> {
        let mut action = None;
        let glass = Glass::from_setting(settings.config.rs_glass);
        // the card is a glass panel: no egui window frame or title bar
        egui::Window::new("menu")
            .title_bar(false)
            .resizable(false)
            .frame(egui::Frame::NONE)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                w::panel(ui, glass, |ui| {
                    let title = match self.page {
                        Page::Main => "DDNet-rs",
                        Page::Settings => "Settings",
                    };
                    w::text(ui, title, size::TITLE, Weight::SemiBold, color::TEXT);
                    ui.add_space(space::S);
                    match self.page {
                        Page::Main => action = self.main_page(ui, connected),
                        Page::Settings => {
                            self.settings_page(ui, settings);
                            if w::button(ui, None, "Back").clicked() {
                                self.page = Page::Main;
                                self.reading_key = None;
                                action = Some(Action::SettingsDone);
                            }
                        }
                    }
                })
            });
        action
    }

    fn main_page(&mut self, ui: &mut egui::Ui, connected: bool) -> Option<Action> {
        let mut action = None;
        // one column of full-width controls, the primary action first
        ui.set_width(MENU_WIDTH);
        ui.with_layout(
            egui::Layout::top_down_justified(egui::Align::Center),
            |ui| {
                if connected {
                    if w::accent_button(ui, Some(icons::GAME), "Back to the game").clicked() {
                        action = Some(Action::Resume);
                    }
                    if w::button(ui, Some(icons::MOVE), "Edit HUD").clicked() {
                        action = Some(Action::EditHud);
                    }
                    if w::button(ui, Some(icons::DISCONNECT), "Disconnect").clicked() {
                        action = Some(Action::Disconnect);
                    }
                } else {
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut self.address)
                            .hint_text("Server address")
                            .desired_width(f32::INFINITY),
                    );
                    let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if w::accent_button(ui, Some(icons::CONNECT), "Connect").clicked() || enter {
                        match self.address.trim().parse::<SocketAddr>() {
                            Ok(a) if a.ip().is_loopback() => action = Some(Action::Connect(a)),
                            Ok(_) => {
                                self.error = Some("Only local servers (127.0.0.1) for now.".into())
                            }
                            Err(_) => {
                                self.error = Some("Enter an address like 127.0.0.1:8303.".into())
                            }
                        }
                    }
                    if let Some(e) = &self.error {
                        ui.colored_label(color::DANGER, e);
                    }
                }
                if w::button(ui, Some(icons::GEAR), "Settings").clicked() {
                    self.page = Page::Settings;
                }
                if w::button(ui, Some(icons::POWER), "Quit").clicked() {
                    action = Some(Action::Quit);
                }
            },
        );
        action
    }

    /// Every change goes through the C++ console syntax, so it gets the same clamping
    /// and truncation as a config file line.
    fn settings_page(&mut self, ui: &mut egui::Ui, s: &mut Settings) {
        let c = &s.config;
        let mut lines = Vec::new();
        // scrolls when the window is small; 200 pt leave room for the title bar, the
        // window margins and the Back button below
        let max_height = ui.ctx().content_rect().height() - 200.0;
        egui::ScrollArea::vertical()
            .max_height(max_height)
            .show(ui, |ui| {
                egui::Grid::new("settings").num_columns(2).show(ui, |ui| {
                    ui.label("Name");
                    let mut name = c.player_name.clone();
                    if ui.text_edit_singleline(&mut name).changed() {
                        lines.push(format!("player_name \"{}\"", escape(&name)));
                    }
                    ui.end_row();
                    ui.label("Skin");
                    egui::ComboBox::from_id_salt("skin")
                        .selected_text(&c.player_skin)
                        .show_ui(ui, |ui| {
                            for skin in &self.skins {
                                if ui.selectable_label(*skin == c.player_skin, skin).clicked() {
                                    lines.push(format!("player_skin \"{}\"", escape(skin)));
                                }
                            }
                        });
                    ui.end_row();
                    self.asset_rows(ui, c, &mut lines);
                    ui.label("Mouse sensitivity");
                    let mut sens = c.inp_mousesens;
                    if ui
                        .add(egui::DragValue::new(&mut sens).range(1..=100000))
                        .changed()
                    {
                        lines.push(format!("inp_mousesens {sens}"));
                    }
                    ui.end_row();
                    for (label, name, value) in [
                        (
                            "Custom colours",
                            "player_use_custom_color",
                            c.player_use_custom_color,
                        ),
                        ("Dynamic camera", "cl_dyncam", c.cl_dyncam),
                        ("Vsync", "gfx_vsync", c.gfx_vsync),
                        ("Media player", "rs_media", c.rs_media),
                    ] {
                        ui.label(label);
                        let mut on = value != 0;
                        if w::toggle(ui, &mut on).changed() {
                            lines.push(format!("{name} {}", on as i32));
                        }
                        ui.end_row();
                    }
                    self.antiping_rows(ui, c, &mut lines);
                    ui.label("Glass");
                    let mut glass = c.rs_glass.clamp(0, 2) as usize;
                    if w::segmented(ui, &mut glass, &["Off", "Subtle", "Full"]).clicked() {
                        lines.push(format!("rs_glass {glass}"));
                    }
                    ui.end_row();
                    ui.label("UI scale");
                    let mut scale = c.rs_ui_scale;
                    // applied when done, not on every drag step (it rescales the UI)
                    let r = ui.add(egui::DragValue::new(&mut scale).range(50..=200).suffix("%"));
                    if r.drag_stopped() || r.lost_focus() {
                        lines.push(format!("rs_ui_scale {scale}"));
                    }
                    ui.end_row();
                    ui.label("Data dir")
                        .on_hover_text("DDNet's data folder; used after a restart");
                    let mut dir = c.rs_data_dir.clone();
                    if ui.text_edit_singleline(&mut dir).changed() {
                        lines.push(format!("rs_data_dir \"{}\"", escape(&dir)));
                    }
                    ui.end_row();
                    for (label, command) in CONTROLS {
                        ui.label(label);
                        let text = if self.reading_key == Some(command) {
                            "press a key (Esc cancels)".to_string()
                        } else {
                            s.binds
                                .slot_of(command)
                                .map_or("none".into(), |(k, m)| Binds::key_bind_name(k, m))
                        };
                        if ui.button(text).clicked() {
                            self.reading_key = Some(command);
                        }
                        ui.end_row();
                    }
                });
            });
        for line in lines {
            s.execute_line(&line);
        }
    }

    /// The thumbnail of a pack (loaded once): the kind's image as the game loads it,
    /// for entities the DDNet image.
    fn thumb(
        &mut self,
        ctx: &egui::Context,
        kind: usize,
        name: &str,
    ) -> Option<egui::TextureHandle> {
        let storage = &self.storage;
        let key = (kind, name.to_string());
        self.thumbs
            .entry(key)
            .or_insert_with(|| {
                let image = match ASSET_KINDS[kind].2 {
                    Some(asset) => crate::assets::load_asset(storage, asset, name).image,
                    None => {
                        let mod_type = crate::render::entities::MOD_DDNET;
                        crate::render::entities::load_image(storage, name, mod_type).1
                    }
                };
                // twice the largest size it is shown at, for high-DPI screens
                let height = (2.0 * size::COVER_PLAYER) as usize;
                image.map(|i| {
                    let t = thumbnail(&i, height);
                    ctx.load_texture(format!("pack {kind} {name}"), t, Default::default())
                })
            })
            .clone()
    }

    /// One pack as a thumbnail `height` points high, ringed in the accent when chosen;
    /// its name on hover. Packs whose image does not load show their name instead.
    fn pack_button(
        &mut self,
        ui: &mut egui::Ui,
        kind: usize,
        name: &str,
        height: f32,
        chosen: bool,
    ) -> egui::Response {
        let r = match self.thumb(ui.ctx(), kind, name) {
            Some(t) => {
                let [w, h] = t.size().map(|v| v as f32);
                let size = egui::vec2(height * w / h, height);
                let image = egui::Image::new(&t)
                    .fit_to_exact_size(size)
                    .corner_radius(style::radius::SMALL)
                    .bg_fill(color::SURFACE_RAISED)
                    .sense(egui::Sense::click());
                ui.add(image)
            }
            None => w::button(ui, None, name),
        };
        if chosen {
            let radius = egui::CornerRadius::same(style::radius::SMALL);
            let ring = egui::Stroke::new(space::XS / 2.0, color::ACCENT);
            (ui.painter()).rect_stroke(r.rect, radius, ring, egui::StrokeKind::Outside);
        }
        r.on_hover_text(name)
            .on_hover_cursor(egui::CursorIcon::PointingHand)
    }

    /// The asset packs (C++ menus_settings_assets.cpp): per kind the chosen pack; a click
    /// opens the grid of every pack of that kind. Rows of the settings grid.
    fn asset_rows(&mut self, ui: &mut egui::Ui, c: &Config, lines: &mut Vec<String>) {
        let glass = Glass::from_setting(c.rs_glass);
        for (kind, (label, var, asset)) in ASSET_KINDS.into_iter().enumerate() {
            let dir = asset.map_or("entities", |a| a.0);
            let chosen = c.string(var).unwrap_or_default();
            ui.label(label);
            let tile = self.pack_button(ui, kind, &chosen, size::CONTROL, false);
            if tile.clicked() {
                self.picking = (self.picking != Some(kind)).then_some(kind);
            }
            let open = self.picking == Some(kind);
            let list = match open {
                true => self.packs[kind]
                    .get_or_insert_with(|| crate::assets::list_packs(&self.storage, dir))
                    .clone(),
                false => Vec::new(),
            };
            let shown = w::popover(&tile, open, glass, None, |ui| {
                ui.set_max_width(MENU_WIDTH * 2.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(space::S, space::S);
                    for name in &list {
                        let r =
                            self.pack_button(ui, kind, name, size::COVER_PLAYER, *name == chosen);
                        if r.clicked() {
                            lines.push(format!("{var} \"{}\"", escape(name)));
                        }
                    }
                });
            });
            // a click outside the grid (or a choice) closes it
            if let Some((_, rect)) = shown {
                let outside = ui.input(|i| {
                    i.pointer.any_click()
                        && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p))
                });
                if (outside && !tile.clicked()) || !lines.is_empty() {
                    self.picking = None;
                }
            }
            ui.end_row();
        }
    }

    /// The antiping group (C++ menus_settings_ddnet.cpp:160): the switch, then what it
    /// predicts; the fine-tuning (smoothing, limits, margin, freeze) behind "Prediction
    /// details". Rows of the settings grid; changes go to `lines`.
    fn antiping_rows(&mut self, ui: &mut egui::Ui, c: &Config, lines: &mut Vec<String>) {
        let toggle = |ui: &mut egui::Ui, lines: &mut Vec<String>, name: &str, value: i32| {
            let mut on = value != 0;
            if w::toggle(ui, &mut on).changed() {
                lines.push(format!("{name} {}", on as i32));
            }
            ui.end_row();
        };
        // rows that belong to the row above sit indented
        let sub = |ui: &mut egui::Ui, text: &str| {
            ui.horizontal(|ui| {
                ui.add_space(space::L);
                ui.label(text)
            })
            .inner
        };
        ui.label("Antiping").on_hover_text(
            "Shows other players and your shots ahead of the server, so your ping is hidden",
        );
        toggle(ui, lines, "cl_antiping", c.cl_antiping);
        if c.cl_antiping != 0 {
            sub(ui, "Other players").on_hover_text("Interact: only players you hook, hit or touch");
            let mut players = c.cl_antiping_players.clamp(0, 2) as usize;
            if w::segmented(ui, &mut players, &["Off", "All", "Interact"]).clicked() {
                lines.push(format!("cl_antiping_players {players}"));
            }
            ui.end_row();
            if c.cl_antiping_players >= 2 {
                sub(ui, "Always predict frozen players");
                let mut on = c.cl_antiping_players == 3;
                if w::toggle(ui, &mut on).changed() {
                    lines.push(format!("cl_antiping_players {}", 2 + on as i32));
                }
                ui.end_row();
            }
            sub(ui, "Weapons");
            toggle(ui, lines, "cl_antiping_weapons", c.cl_antiping_weapons);
            sub(ui, "Grenade paths");
            toggle(ui, lines, "cl_antiping_grenade", c.cl_antiping_grenade);
        }
        ui.label("Prediction details");
        let label = if self.prediction_details {
            "Hide"
        } else {
            "Show"
        };
        if w::button(ui, None, label).clicked() {
            self.prediction_details = !self.prediction_details;
        }
        ui.end_row();
        if !self.prediction_details {
            return;
        }
        sub(ui, "Smooth other players")
            .on_hover_text("Eases out small corrections of other players' predicted moves");
        toggle(ui, lines, "cl_antiping_smooth", c.cl_antiping_smooth);
        sub(ui, "Predicted gunfire")
            .on_hover_text("Your shots and their effects drawn from the prediction");
        toggle(ui, lines, "cl_antiping_gunfire", c.cl_antiping_gunfire);
        sub(ui, "Early inputs")
            .on_hover_text("Uses other players' inputs the server forwards before their tick");
        toggle(ui, lines, "cl_antiping_preinput", c.cl_antiping_preinput);
        let mut drag =
            |ui: &mut egui::Ui, label: &str, hover: &str, name: &str, value: i32, range, suffix| {
                sub(ui, label).on_hover_text(hover);
                let mut v = value;
                let r = ui.add(egui::DragValue::new(&mut v).range(range).suffix(suffix));
                if r.changed() {
                    lines.push(format!("{name} {v}"));
                }
                ui.end_row();
            };
        drag(
            ui,
            "Prediction limit",
            "Predicts others at most this far ahead; 0 uses the amount below",
            "cl_antiping_limit",
            c.cl_antiping_limit,
            0..=500,
            " ms",
        );
        if c.cl_antiping_limit == 0 {
            drag(
                ui,
                "Prediction amount",
                "How much of your ping others are predicted ahead",
                "cl_antiping_percent",
                c.cl_antiping_percent,
                0..=100,
                "%",
            );
        }
        drag(
            ui,
            "Prediction margin",
            "Extra time for ping jumps; adds this much delay (servers that sync weapon input)",
            "cl_prediction_margin",
            c.cl_prediction_margin,
            1..=300,
            " ms",
        );
        sub(ui, "Predict freeze")
            .on_hover_text("Partial: you may still move for the last tick or two");
        let mut freeze = c.cl_predict_freeze.clamp(0, 2) as usize;
        if w::segmented(ui, &mut freeze, &["Off", "On", "Partial"]).clicked() {
            lines.push(format!("cl_predict_freeze {freeze}"));
        }
        ui.end_row();
    }
}
