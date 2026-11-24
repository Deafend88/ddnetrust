//! The design system's components on egui (PLAN §10.3, 7.1): panels and pills, the
//! icon + value + unit stat, separator dots, buttons, toggle, slider, segmented control,
//! popover. Everything reads its look from `style.rs`.

use egui::{
    Color32, CornerRadius, Frame, InnerResponse, Margin, Response, RichText, Sense, Stroke, Ui,
    vec2,
};

use crate::style::{
    Glass, POPUP_SHADOW, Weight, color, font, icon_font, motion, radius, size, space,
};

/// A card: rounded panel with the glass fill and a hairline border.
pub fn panel<R>(ui: &mut Ui, glass: Glass, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    Frame::new()
        .fill(glass.panel_fill())
        .stroke(Stroke::new(1.0, color::BORDER))
        .corner_radius(CornerRadius::same(radius::CARD))
        .inner_margin(Margin::same(space::XL as i8))
        .show(ui, |ui| ui.vertical(add).inner)
}

/// A fully rounded capsule.
pub fn pill<R>(ui: &mut Ui, fill: Color32, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(radius::PILL))
        .inner_margin(Margin::symmetric(space::L as i8, space::S as i8))
        .show(ui, |ui| ui.horizontal(add).inner)
}

/// An icon glyph (`icons.rs`).
pub fn icon(ui: &mut Ui, glyph: &str, size: f32, color: Color32) -> Response {
    ui.label(RichText::new(glyph).font(icon_font(size)).color(color))
}

/// Text in a weight and size from the type scale.
pub fn text(ui: &mut Ui, s: &str, size: f32, weight: Weight, color: Color32) -> Response {
    ui.label(RichText::new(s).font(font(size, weight)).color(color))
}

/// A stat as in the watermark (SPEC §1): small white icon, white value (medium), grey
/// unit right after it.
pub fn stat(ui: &mut Ui, glyph: &str, value: &str, unit: Option<&str>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        icon(ui, glyph, size::ICON, color::TEXT);
        ui.add_space(space::XS + 2.0);
        text(ui, value, size::BODY, Weight::Medium, color::TEXT);
        if let Some(unit) = unit {
            ui.add_space(1.0);
            text(ui, unit, size::BODY, Weight::Regular, color::TEXT_SECONDARY);
        }
    });
}

/// The faint dot between stats.
pub fn dot(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(space::L, size::BODY), Sense::hover());
    ui.painter().circle_filled(
        rect.center(),
        1.5,
        color::TEXT_SECONDARY.gamma_multiply(0.5),
    );
}

/// A button with an optional icon before the label. The icon is its own atom in the
/// icon font: Inter maps Phosphor's private-use codepoints to glyphs of its own.
fn labelled(
    glyph: Option<&str>,
    label: &str,
    weight: Weight,
    color: Color32,
) -> egui::Button<'static> {
    let text = RichText::new(label)
        .font(font(size::BODY, weight))
        .color(color);
    match glyph {
        Some(g) => egui::Button::new((
            RichText::new(g).font(icon_font(size::ICON)).color(color),
            text,
        )),
        None => egui::Button::new(text),
    }
}

/// A plain button.
pub fn button(ui: &mut Ui, glyph: Option<&str>, label: &str) -> Response {
    ui.add(labelled(glyph, label, Weight::Medium, color::TEXT))
}

/// The one accent-coloured button of a view (e.g. Connect).
pub fn accent_button(ui: &mut Ui, glyph: Option<&str>, label: &str) -> Response {
    ui.add(labelled(glyph, label, Weight::SemiBold, Color32::WHITE).fill(color::ACCENT))
}

/// An on/off switch with an animated knob.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let size = vec2(44.0, 26.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = ui
        .ctx()
        .animate_bool_with_time(response.id, *on, motion::FAST);
    let track = color::SURFACE_HOVER.lerp_to_gamma(color::ACCENT, t);
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(radius::PILL), track);
    let r = rect.height() / 2.0 - 2.0;
    let x = egui::lerp(rect.left() + r + 2.0..=rect.right() - r - 2.0, t);
    p.circle_filled(egui::pos2(x, rect.center().y), r, Color32::WHITE);
    response
}

/// A slider in the accent colour.
pub fn slider<N: egui::emath::Numeric>(
    ui: &mut Ui,
    value: &mut N,
    range: std::ops::RangeInclusive<N>,
) -> Response {
    ui.scope(|ui| {
        ui.visuals_mut().selection.bg_fill = color::ACCENT;
        ui.add(egui::Slider::new(value, range).trailing_fill(true))
    })
    .inner
}

/// A segmented control: one of `labels` selected.
pub fn segmented(ui: &mut Ui, selected: &mut usize, labels: &[&str]) -> Response {
    Frame::new()
        .fill(color::SURFACE_RAISED)
        .corner_radius(CornerRadius::same(radius::PILL))
        .inner_margin(Margin::same(3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let mut changed: Option<Response> = None;
                for (i, l) in labels.iter().enumerate() {
                    let on = *selected == i;
                    let b = egui::Button::new(
                        RichText::new(*l)
                            .font(font(size::BODY, Weight::Medium))
                            .color(if on {
                                Color32::WHITE
                            } else {
                                color::TEXT_SECONDARY
                            }),
                    )
                    .fill(if on {
                        color::ACCENT
                    } else {
                        Color32::TRANSPARENT
                    });
                    let r = ui.add(b);
                    if r.clicked() {
                        *selected = i;
                        changed = Some(r.clone());
                    }
                    changed = Some(changed.map_or(r.clone(), |c| c.union(r)));
                }
                changed.expect("at least one label")
            })
            .inner
        })
        .inner
}

/// A popover under `anchor`, open while `open` is set (e.g. a widget's settings).
pub fn popover<R>(
    anchor: &Response,
    open: bool,
    glass: Glass,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    egui::Popup::from_response(anchor)
        .open(open)
        .gap(space::S)
        .frame(
            Frame::new()
                .fill(glass.panel_fill())
                .stroke(Stroke::new(1.0, color::BORDER))
                .corner_radius(CornerRadius::same(radius::CARD - 4))
                .inner_margin(Margin::same(space::L as i8))
                .shadow(POPUP_SHADOW),
        )
        .show(add)
        .map(|r| r.inner)
}
