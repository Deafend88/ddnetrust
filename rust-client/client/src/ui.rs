//! Minimal menus (M6.8): connect by address, basic settings, quit. Drawn with egui over
//! the composited world; the full menu design (C++ `CMenus`) comes later.

use std::net::SocketAddr;
use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};
use winit::window::Window;

use crate::config::{Binds, Settings, escape};
use crate::gfx::{Gfx, Size};
use crate::storage::Storage;

/// egui with its winit input and wgpu painter.
pub struct Ui {
    ctx: egui::Context,
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
}

impl Ui {
    /// `None` without DDNet's `fonts/DejaVuSans.ttf` (the menus need a font).
    pub fn new(
        window: &Window,
        gfx: &Gfx,
        format: wgpu::TextureFormat,
        storage: &Storage,
    ) -> Option<Self> {
        let font = storage.read("fonts/DejaVuSans.ttf")?;
        let ctx = egui::Context::default();
        let mut fonts = FontDefinitions::empty();
        fonts
            .font_data
            .insert("DejaVuSans".into(), Arc::new(FontData::from_owned(font)));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts.families.insert(family, vec!["DejaVuSans".into()]);
        }
        ctx.set_fonts(fonts);
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            None,
            None,
            None,
        );
        let renderer = egui_wgpu::Renderer::new(&gfx.device, format, Default::default());
        Some(Self {
            ctx,
            state,
            renderer,
        })
    }

    /// Feeds a window event to egui; true if egui wants it for itself (typing, clicks).
    pub fn on_window_event(&mut self, window: &Window, event: &winit::event::WindowEvent) -> bool {
        self.state.on_window_event(window, event).consumed
    }

    /// Runs `build` for one frame and draws the result onto `target`.
    pub fn frame(
        &mut self,
        window: &Window,
        gfx: &Gfx,
        target: &wgpu::TextureView,
        size: Size,
        mut build: impl FnMut(&egui::Context),
    ) {
        let input = self.state.take_egui_input(window);
        let mut output = self.ctx.run_pass(input, |ctx| build(ctx));
        self.state
            .handle_platform_output(window, output.platform_output);
        let ppp = output.pixels_per_point;
        let prims = self.ctx.tessellate(output.shapes, ppp);
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
    }
}

/// The bindable controls shown in the settings, as (label, command).
const CONTROLS: [(&str, &str); 5] = [
    ("Move left", "+left"),
    ("Move right", "+right"),
    ("Jump", "+jump"),
    ("Hook", "+hook"),
    ("Fire", "+fire"),
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
        }
    }

    /// The start screen (`connected` false) or the in-game menu, and the settings page.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        connected: bool,
        settings: &mut Settings,
    ) -> Option<Action> {
        let mut action = None;
        egui::Window::new(if self.page == Page::Settings {
            "Settings"
        } else {
            "DDNet-rs"
        })
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| match self.page {
            Page::Main => action = self.main_page(ui, connected),
            Page::Settings => {
                self.settings_page(ui, settings);
                if ui.button("Back").clicked() {
                    self.page = Page::Main;
                    self.reading_key = None;
                    action = Some(Action::SettingsDone);
                }
            }
        });
        action
    }

    fn main_page(&mut self, ui: &mut egui::Ui, connected: bool) -> Option<Action> {
        let mut action = None;
        if connected {
            if ui.button("Back to the game").clicked() {
                action = Some(Action::Resume);
            }
            if ui.button("Disconnect").clicked() {
                action = Some(Action::Disconnect);
            }
        } else {
            ui.horizontal(|ui| {
                ui.label("Server address");
                let edit = ui.text_edit_singleline(&mut self.address);
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button("Connect").clicked() || enter {
                    match self.address.trim().parse::<SocketAddr>() {
                        Ok(a) if a.ip().is_loopback() => action = Some(Action::Connect(a)),
                        Ok(_) => self.error = Some("only local (loopback) servers for now".into()),
                        Err(_) => self.error = Some("address must look like 127.0.0.1:8303".into()),
                    }
                }
            });
            if let Some(e) = &self.error {
                ui.colored_label(egui::Color32::LIGHT_RED, e);
            }
        }
        if ui.button("Settings").clicked() {
            self.page = Page::Settings;
        }
        if ui.button("Quit").clicked() {
            action = Some(Action::Quit);
        }
        action
    }

    /// Every change goes through the C++ console syntax, so it gets the same clamping
    /// and truncation as a config file line.
    fn settings_page(&mut self, ui: &mut egui::Ui, s: &mut Settings) {
        let c = &s.config;
        let mut lines = Vec::new();
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
            ] {
                ui.label(label);
                let mut on = value != 0;
                if ui.checkbox(&mut on, "").changed() {
                    lines.push(format!("{name} {}", on as i32));
                }
                ui.end_row();
            }
            ui.label("Data dir (on restart)");
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
        for line in lines {
            s.execute_line(&line);
        }
    }
}
