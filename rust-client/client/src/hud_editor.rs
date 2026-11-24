//! The HUD editor (PLAN §10.3, 7.4): widgets are dragged with snapping (screen edges and
//! centre, other widgets' edges, else an 8 pt grid; Shift moves freely), a click opens
//! the widget's settings, and a panel lists every widget with an on/off toggle. A widget
//! takes the anchor nearest to where it is dropped, so it keeps its place when the
//! window size changes. The caller saves settings.cfg when the editor closes.

use egui::{
    Align2, CornerRadius, Id, LayerId, Order, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, vec2,
};

use crate::hud::{Anchor, HudConfig, HudData, Placement, Widget, area_id};
use crate::icons;
use crate::style::{Glass, Weight, color, radius, size, space};
use crate::widgets as w;

/// How close (points) an edge or centre must come to a line to snap to it.
pub const SNAP: f32 = space::S;
/// The grid offsets snap to when no line is near.
pub const GRID: f32 = space::S;

/// An anchor's point as a fraction of a rectangle (0, 0.5 or 1 per axis).
fn fraction(a: Anchor) -> Vec2 {
    let align = a.align();
    vec2(align.x().to_factor(), align.y().to_factor())
}

/// The anchor of the screen third (per axis) the rectangle's centre is in.
pub fn nearest_anchor(rect: Rect, screen: Rect) -> Anchor {
    let c = (rect.center() - screen.min) / screen.size();
    let third = |f: f32| {
        if f < 1.0 / 3.0 {
            0
        } else if f > 2.0 / 3.0 {
            2
        } else {
            1
        }
    };
    Anchor::ALL[third(c.y) * 3 + third(c.x)]
}

/// The placement that puts a widget exactly at `rect`: the nearest anchor, and the offset
/// between the widget's and the screen's anchor points.
pub fn placement_of(rect: Rect, screen: Rect, enabled: bool) -> Placement {
    let anchor = nearest_anchor(rect, screen);
    let f = fraction(anchor);
    let offset = (rect.min + rect.size() * f) - (screen.min + screen.size() * f);
    Placement {
        anchor,
        x: offset.x.round() as i32,
        y: offset.y.round() as i32,
        enabled,
    }
}

/// Moves `min` so the nearest of the span's start, centre or end lies on one of `lines`
/// (within `SNAP`); returns the new start and the line used.
fn snap_axis(min: f32, len: f32, lines: &[f32]) -> (f32, Option<f32>) {
    let mut best: Option<(f32, f32)> = None;
    for &line in lines {
        for p in [min, min + len / 2.0, min + len] {
            let d = line - p;
            if d.abs() <= SNAP && best.is_none_or(|(b, _)| d.abs() < b.abs()) {
                best = Some((d, line));
            }
        }
    }
    best.map_or((min, None), |(d, line)| (min + d, Some(line)))
}

/// Where a widget dragged to `rect` lands: kept on screen; unless `free`, snapped to the
/// screen's edges and centre or `others`' edges, else its offset to the `GRID`. Returns
/// the placement and the guide line used per axis (x, y).
pub fn drop(
    rect: Rect,
    screen: Rect,
    others: &[Rect],
    free: bool,
) -> (Placement, [Option<f32>; 2]) {
    let max = (screen.max - rect.size()).max(screen.min);
    let mut rect = Rect::from_min_size(rect.min.clamp(screen.min, max), rect.size());
    let mut guides = [None; 2];
    if !free {
        let xs: Vec<f32> = [screen.left(), screen.center().x, screen.right()]
            .into_iter()
            .chain(others.iter().flat_map(|o| [o.left(), o.right()]))
            .collect();
        let ys: Vec<f32> = [screen.top(), screen.center().y, screen.bottom()]
            .into_iter()
            .chain(others.iter().flat_map(|o| [o.top(), o.bottom()]))
            .collect();
        let (x, gx) = snap_axis(rect.min.x, rect.width(), &xs);
        let (y, gy) = snap_axis(rect.min.y, rect.height(), &ys);
        rect = Rect::from_min_size(Pos2::new(x, y), rect.size());
        guides = [gx, gy];
    }
    let mut p = placement_of(rect, screen, true);
    if !free {
        let grid = |v: i32| ((v as f32 / GRID).round() * GRID) as i32;
        if guides[0].is_none() {
            p.x = grid(p.x);
        }
        if guides[1].is_none() {
            p.y = grid(p.y);
        }
    }
    (p, guides)
}

/// The editor's state while it is open.
#[derive(Default)]
pub struct Editor {
    pub open: bool,
    /// The widget whose settings popover is open.
    selected: Option<&'static str>,
    /// The widget being dragged, its rectangle when the drag began and the pointer
    /// motion since.
    drag: Option<(&'static str, Rect, Vec2)>,
    guides: [Option<f32>; 2],
}

impl Editor {
    pub fn close(&mut self) {
        *self = Self::default();
    }

    /// Draws the enabled widgets (movable, outlined) and the editor panel; true when
    /// the player clicks Done.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        widgets: &mut [Box<dyn Widget>],
        data: &HudData,
        config: &mut HudConfig,
    ) -> bool {
        let screen = ctx.content_rect();
        let free = ctx.input(|i| i.modifiers.shift);
        let placements: Vec<Placement> = (widgets.iter())
            .map(|w| config.placement(w.id(), w.default_placement()))
            .collect();
        // last frame's rectangles of the shown widgets, for snapping to each other
        let rects: Vec<(&str, Rect)> = (widgets.iter().zip(&placements))
            .filter(|(_, p)| p.enabled)
            .filter_map(|(w, _)| Some((w.id(), ctx.memory(|m| m.area_rect(area_id(w.id())))?)))
            .collect();
        for (w, p) in widgets.iter_mut().zip(placements) {
            if !p.enabled {
                continue;
            }
            let id = w.id();
            let response = egui::Area::new(area_id(id))
                .anchor(p.anchor.align(), [p.x as f32, p.y as f32])
                // the panel's order: the widget last touched comes on top of it
                .order(Order::Middle)
                .movable(false)
                .sense(Sense::click_and_drag())
                .show(ctx, |ui| {
                    // selectable text would take the drag from the widget
                    ui.style_mut().interaction.selectable_labels = false;
                    w.show(ui, data, config)
                })
                .response;
            if response.drag_started() {
                self.drag = Some((id, response.rect, Vec2::ZERO));
                self.selected = None;
            }
            if let Some((dragged, start, moved)) = &mut self.drag
                && *dragged == id
            {
                *moved += response.drag_delta();
                let others: Vec<Rect> = (rects.iter())
                    .filter(|(o, _)| *o != id)
                    .map(|(_, r)| *r)
                    .collect();
                let (placement, guides) = drop(start.translate(*moved), screen, &others, free);
                config.placements.insert(id.into(), placement);
                self.guides = guides;
                if response.drag_stopped() {
                    self.drag = None;
                    self.guides = [None; 2];
                }
            }
            if response.clicked() {
                self.selected = (self.selected != Some(id)).then_some(id);
            }
            let active = self.selected == Some(id) || response.dragged();
            let outline = if active {
                Stroke::new(2.0, color::ACCENT)
            } else {
                Stroke::new(1.0, color::TEXT_SECONDARY)
            };
            ctx.layer_painter(response.layer_id).rect_stroke(
                response.rect.expand(space::XS),
                CornerRadius::same(radius::PILL),
                outline,
                StrokeKind::Outside,
            );
            w::popover(&response, self.selected == Some(id), data.glass, |ui| {
                w::text(ui, w.name(), size::LABEL, Weight::SemiBold, color::TEXT);
                w.settings(ui, config);
                if w::button(ui, Some(icons::RESET), "Reset").clicked() {
                    config.reset(id);
                }
            });
        }
        // a click on the game closes the settings popover
        if ctx.input(|i| i.pointer.primary_clicked()) && !ctx.is_pointer_over_egui() {
            self.selected = None;
        }
        let guides = ctx.layer_painter(LayerId::new(Order::Foreground, Id::unique("hud_guides")));
        let guide = Stroke::new(1.0, color::ACCENT);
        if let Some(x) = self.guides[0] {
            guides.vline(x, screen.y_range(), guide);
        }
        if let Some(y) = self.guides[1] {
            guides.hline(screen.x_range(), y, guide);
        }
        self.panel(ctx, widgets, config, data.glass)
    }

    /// The list of widgets with on/off toggles, Reset all and Done.
    fn panel(
        &mut self,
        ctx: &egui::Context,
        widgets: &[Box<dyn Widget>],
        config: &mut HudConfig,
        glass: Glass,
    ) -> bool {
        let mut done = false;
        egui::Area::new(Id::unique("hud_editor"))
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                w::panel(ui, glass, |ui| {
                    w::text(ui, "Edit HUD", size::TITLE, Weight::SemiBold, color::TEXT);
                    let weak = ui.visuals().weak_text_color();
                    w::text(
                        ui,
                        "Drag to move, hold Shift to move freely. Click a widget for its settings.",
                        size::CAPTION,
                        Weight::Regular,
                        weak,
                    );
                    egui::Grid::new("hud_widgets")
                        .num_columns(2)
                        .show(ui, |ui| {
                            for w in widgets {
                                let mut p = config.placement(w.id(), w.default_placement());
                                ui.horizontal(|ui| {
                                    w::icon(ui, w.icon(), size::ICON, color::TEXT);
                                    ui.label(w.name());
                                });
                                if w::toggle(ui, &mut p.enabled).changed() {
                                    config.placements.insert(w.id().into(), p);
                                }
                                ui.end_row();
                            }
                        });
                    ui.horizontal(|ui| {
                        if w::button(ui, Some(icons::RESET), "Reset all").clicked() {
                            // settings of widgets this version does not know stay
                            for w in widgets {
                                config.reset(w.id());
                            }
                            self.selected = None;
                        }
                        done = w::accent_button(ui, Some(icons::CHECK), "Done").clicked();
                    });
                });
            });
        done
    }
}
