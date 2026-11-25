//! The design system's components on egui (PLAN §10.3, 7.1): panels and pills, the
//! icon + value + unit stat, separator dots, buttons, toggle, slider, segmented control,
//! popover. Everything reads its look from `style.rs`.

use egui::{
    Color32, CornerRadius, Frame, InnerResponse, Margin, Response, RichText, Sense, Shadow, Shape,
    Stroke, Ui, vec2,
};

use crate::glass;

use crate::style::{
    Glass, POPUP_SHADOW, Weight, color, font, icon_font, motion, radius, size, space,
};

/// Shows `frame` over glass of the same shape, in paint order: the frame's shadow, the
/// blurred game (none when glass is off), the frame's fill and border.
fn glass_frame<R>(
    ui: &mut Ui,
    glass: Glass,
    liquid: bool,
    mut frame: Frame,
    add: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let shadow = std::mem::replace(&mut frame.shadow, Shadow::NONE);
    let shadow_at = ui.painter().add(Shape::Noop);
    let backdrop_at = ui.painter().add(Shape::Noop);
    let r = frame.show(ui, add);
    let rect = r.response.rect;
    ui.painter()
        .set(shadow_at, shadow.as_shape(rect, frame.corner_radius));
    if glass != Glass::Off {
        // 1 px inside the fill's anti-aliased edge, or the sharper-cut blur shows as a
        // bright rim around the panel
        let radius = frame.corner_radius.nw as f32 - 1.0;
        let liquid = liquid && glass == Glass::Full;
        ui.painter()
            .set(backdrop_at, glass::shape(rect.shrink(1.0), radius, liquid));
    }
    r
}

/// A card: rounded glass panel with a hairline border.
pub fn panel<R>(ui: &mut Ui, glass: Glass, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let frame = Frame::new()
        .fill(glass.panel_fill())
        .stroke(Stroke::new(1.0, color::BORDER))
        .corner_radius(CornerRadius::same(radius::CARD))
        .inner_margin(Margin::same(space::XL as i8));
    glass_frame(ui, glass, false, frame, |ui| ui.vertical(add).inner)
}

fn pill_frame(fill: Color32) -> Frame {
    Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(radius::PILL))
        .inner_margin(Margin::symmetric(space::L as i8, space::S as i8))
}

/// A fully rounded capsule in a solid colour (e.g. the watermark's logo).
pub fn pill<R>(ui: &mut Ui, fill: Color32, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    pill_frame(fill).show(ui, |ui| ui.horizontal(add).inner)
}

/// A glass capsule (the watermark, the island; `liquid`: Liquid Glass at full strength).
pub fn glass_pill<R>(
    ui: &mut Ui,
    glass: Glass,
    liquid: bool,
    add: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let fill = if liquid && glass == Glass::Full {
        Glass::liquid_fill()
    } else {
        glass.panel_fill()
    };
    glass_frame(ui, glass, liquid, pill_frame(fill), |ui| {
        ui.horizontal(add).inner
    })
}

/// A HUD bar: a translucent shape over glass (the watermark's container), `margin`
/// inside, its items centred on a row `row_height` tall (the tallest item). Shape rule
/// (CLAUDE.md): a pill (`radius::PILL`) for one line, a rounded rectangle
/// (`radius::CARD`) for several; the watermark's radius is a setting.
pub fn hud_bar<R>(
    ui: &mut Ui,
    glass: Glass,
    corner_radius: u8,
    margin: Margin,
    row_height: f32,
    add: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    hud_bar_with(
        ui,
        glass,
        corner_radius,
        margin,
        row_height,
        Shadow::NONE,
        add,
    )
}

/// `hud_bar` with a shadow around it (the watermark's dark glow).
pub fn hud_bar_with<R>(
    ui: &mut Ui,
    glass: Glass,
    corner_radius: u8,
    margin: Margin,
    row_height: f32,
    shadow: Shadow,
    add: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let frame = Frame::new()
        .fill(glass.hud_fill())
        .shadow(shadow)
        .corner_radius(CornerRadius::same(corner_radius))
        .inner_margin(margin);
    glass_frame(ui, glass, false, frame, |ui| {
        // egui centres a horizontal row's items on interact_size.y (the row runs right
        // to left in a right-aligned parent, like the killfeed's)
        ui.spacing_mut().interact_size.y = row_height;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            add(ui)
        })
        .inner
    })
}

/// What a hidden widget (every item off, nothing to show) draws in the HUD editor, so it
/// can still be selected and turned back on: its icon and name, greyed.
pub fn placeholder(ui: &mut Ui, glass: Glass, glyph: &str, name: &str) {
    let margin = Margin::symmetric(space::L as i8, space::S as i8);
    hud_bar(ui, glass, radius::PILL, margin, size::ICON, |ui| {
        let weak = ui.visuals().weak_text_color();
        ui.spacing_mut().item_spacing.x = space::S;
        icon(ui, glyph, size::ICON, weak);
        text(ui, name, size::BODY, crate::style::Weight::Regular, weak);
    });
}

/// The DDNet-rs mark: a tee looking right (round body, two upright eyes, two feet) in
/// `color`, the eyes in `eyes`.
pub fn tee_mark(ui: &mut Ui, size: f32, color: Color32, eyes: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let p = ui.painter();
    let capsule = |center: egui::Pos2, s: egui::Vec2, c: Color32| {
        p.rect_filled(
            egui::Rect::from_center_size(center, s),
            CornerRadius::same(radius::PILL),
            c,
        );
    };
    let (c, u) = (rect.center() - vec2(0.0, size * 0.06), size);
    p.circle_filled(c, u * 0.42, color);
    for dx in [-0.27, 0.12] {
        capsule(
            rect.center() + vec2(u * dx, u * 0.38),
            vec2(u * 0.42, u * 0.2),
            color,
        );
    }
    for dx in [0.1, 0.26] {
        capsule(c + vec2(u * dx, -u * 0.08), vec2(u * 0.1, u * 0.2), eyes);
    }
    response
}

/// An icon glyph (`icons.rs`).
pub fn icon(ui: &mut Ui, glyph: &str, size: f32, color: Color32) -> Response {
    ui.label(RichText::new(glyph).font(icon_font(size)).color(color))
}

/// Text in a weight and size from the type scale.
pub fn text(ui: &mut Ui, s: &str, size: f32, weight: Weight, color: Color32) -> Response {
    ui.label(RichText::new(s).font(font(size, weight)).color(color))
}

/// A number right-aligned in a field at least as wide as `template` (e.g. "0000" for
/// four digits; "" for none). With Inter's tabular digits (`scripts/gen_tnum.py`) a
/// changing value never moves what comes after it.
pub fn number(
    ui: &mut Ui,
    value: &str,
    template: &str,
    size: f32,
    weight: Weight,
    color: Color32,
) -> Response {
    let font = font(size, weight);
    let min = ui.fonts_mut(|f| {
        f.layout_no_wrap(template.into(), font.clone(), color)
            .size()
            .x
    });
    let g = ui.fonts_mut(|f| f.layout_no_wrap(value.into(), font, color));
    let (rect, response) =
        ui.allocate_exact_size(vec2(g.size().x.max(min), g.size().y), Sense::hover());
    let at = egui::pos2(rect.right() - g.size().x, rect.top());
    ui.painter().galley(at, g, color);
    response
}

/// A stat as in the watermark (SPEC §1): small white icon with `glow` behind it
/// (`Shadow::NONE`: none), white value (medium) in a field as wide as `template` at
/// least, grey unit right after it.
pub fn stat(
    ui: &mut Ui,
    glyph: &str,
    value: &str,
    template: &str,
    unit: Option<&str>,
    glow: Shadow,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let glow_at = ui.painter().add(Shape::Noop);
        let r = icon(ui, glyph, size::ICON, color::TEXT);
        // a round blur the icon's size, behind it
        let behind = egui::Rect::from_center_size(r.rect.center(), vec2(size::ICON, size::ICON));
        ui.painter().set(
            glow_at,
            glow.as_shape(behind, CornerRadius::same(radius::PILL)),
        );
        ui.add_space(space::S);
        number(ui, value, template, size::BODY, Weight::Medium, color::TEXT);
        if let Some(unit) = unit {
            let weak = ui.visuals().weak_text_color();
            text(ui, unit, size::BODY, Weight::Regular, weak);
        }
    });
}

/// The barely visible dot between stats, in the middle of the gap between them.
pub fn dot(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(2.0 * space::XL, size::BODY), Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), 1.5, color::SEPARATOR);
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

/// A frameless icon button; `label` is its tooltip.
pub fn icon_button(ui: &mut Ui, glyph: &str, label: &str) -> Response {
    let icon = RichText::new(glyph).font(icon_font(size::ICON));
    ui.add(egui::Button::new(icon).frame(false))
        .on_hover_text(label)
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
    let size = vec2(space::XXL + space::M, space::XL);
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
        .inner_margin(Margin::same(space::XS as i8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = space::XS;
                let mut changed: Option<Response> = None;
                let weak = ui.visuals().weak_text_color();
                for (i, l) in labels.iter().enumerate() {
                    let on = *selected == i;
                    let b = egui::Button::new(
                        RichText::new(*l)
                            .font(font(size::BODY, Weight::Medium))
                            .color(if on { Color32::WHITE } else { weak }),
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

/// The gap between a popover and what it belongs to.
pub const POPOVER_GAP: f32 = space::S;

/// A popover next to `anchor`, open while `open` is set (e.g. a widget's settings); on
/// the side `align` names, else where egui finds room. Returns the content's result and
/// the popover's rectangle.
pub fn popover<R>(
    anchor: &Response,
    open: bool,
    glass: Glass,
    align: Option<egui::RectAlign>,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<(R, egui::Rect)> {
    let frame = Frame::new()
        .fill(glass.panel_fill())
        .stroke(Stroke::new(1.0, color::BORDER))
        .corner_radius(CornerRadius::same(radius::CARD))
        .inner_margin(Margin::same(space::L as i8))
        .shadow(POPUP_SHADOW);
    let mut popup = egui::Popup::from_response(anchor);
    if let Some(align) = align {
        popup = popup.align(align).align_alternatives(&[]);
    }
    popup
        .open(open)
        .gap(POPOVER_GAP)
        .frame(Frame::NONE)
        .show(|ui| glass_frame(ui, glass, false, frame, add).inner)
        .map(|r| (r.inner, r.response.rect))
}
