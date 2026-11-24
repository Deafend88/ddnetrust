//! The design system's tokens (PLAN §10.2): colours, radii, spacing, type, glass
//! strength, and the egui fonts and style built from them. One place for the look; the
//! components in `widgets.rs` read only from here.

use std::sync::Arc;

use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId};

/// Colours: dark surfaces, high-contrast text, one accent (blue).
pub mod color {
    use egui::Color32;

    pub const SURFACE: Color32 = Color32::from_rgb(22, 22, 26);
    pub const SURFACE_RAISED: Color32 = Color32::from_rgb(38, 38, 44);
    pub const SURFACE_HOVER: Color32 = Color32::from_rgb(52, 52, 60);
    pub const TEXT: Color32 = Color32::from_rgb(245, 245, 247);
    /// Secondary text (captions, units); widgets take it from the style
    /// (`Visuals::weak_text_color`), which picks it per glass strength.
    pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(172, 172, 178);
    /// Secondary text over full glass, where more of the game shows (5.6:1 on the dm1
    /// review render; 172 grey would be 4.0:1).
    pub const TEXT_SECONDARY_FULL: Color32 = Color32::from_rgb(204, 204, 210);
    pub const SEPARATOR: Color32 = Color32::from_rgba_premultiplied(14, 14, 14, 14);
    pub const BORDER: Color32 = Color32::from_rgba_premultiplied(8, 8, 8, 8);
    pub const ACCENT: Color32 = Color32::from_rgb(10, 132, 255);
    pub const SUCCESS: Color32 = Color32::from_rgb(48, 209, 88);
    pub const DANGER: Color32 = Color32::from_rgb(255, 69, 58);
    /// The logo pill of the watermark (white, SPEC §1).
    pub const LOGO: Color32 = Color32::from_rgb(250, 250, 252);
    /// Icon and name on the logo pill: grey, as in the references; 4.5:1 on `LOGO`.
    pub const LOGO_TEXT: Color32 = Color32::from_rgb(114, 114, 122);
}

/// Corner radii in points; pills use `PILL` (fully rounded).
pub mod radius {
    pub const SMALL: u8 = 12;
    pub const CARD: u8 = 24;
    pub const PILL: u8 = 255;
}

/// Spacing scale in points.
pub mod space {
    pub const XS: f32 = 4.0;
    pub const S: f32 = 8.0;
    pub const M: f32 = 12.0;
    pub const L: f32 = 16.0;
    pub const XL: f32 = 24.0;
    pub const XXL: f32 = 32.0;
}

/// Type scale in points.
pub mod size {
    pub const CAPTION: f32 = 12.0;
    pub const BODY: f32 = 15.0;
    pub const LABEL: f32 = 16.0;
    pub const TITLE: f32 = 19.0;
    pub const LARGE: f32 = 26.0;
    pub const ICON_SMALL: f32 = 14.0;
    pub const ICON: f32 = 17.0;
    pub const ICON_LARGE: f32 = 21.0;
}

/// Animation timing in seconds.
pub mod motion {
    /// Toggles, hovers, small state changes.
    pub const FAST: f32 = 0.12;
    /// Panels appearing, capsules sliding in.
    pub const NORMAL: f32 = 0.25;
    /// Spring settle time of the island and other size changes.
    pub const SPRING: f32 = 0.45;
}

/// The soft shadow under popovers and menus.
pub const POPUP_SHADOW: egui::Shadow = egui::Shadow {
    offset: [0, 8],
    blur: 28,
    spread: 0,
    color: Color32::from_black_alpha(110),
};

/// The soft white glow around the watermark's logo (the reference shows one).
pub const LOGO_GLOW: egui::Shadow = egui::Shadow {
    offset: [0, 0],
    blur: 20,
    spread: 1,
    color: Color32::from_rgba_premultiplied(150, 150, 150, 150),
};

/// Font weights (egui font families).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weight {
    Regular,
    Medium,
    SemiBold,
}

pub fn font(size: f32, weight: Weight) -> FontId {
    FontId::new(
        size,
        match weight {
            Weight::Regular => FontFamily::Proportional,
            Weight::Medium => FontFamily::Name("medium".into()),
            Weight::SemiBold => FontFamily::Name("semibold".into()),
        },
    )
}

/// Phosphor icons (`icons.rs`) are drawn with this font.
pub fn icon_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("icons".into()))
}

/// C++-free setting `rs_glass`: how much of the game shows through panels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glass {
    Off,
    Subtle,
    Full,
}

impl Glass {
    pub fn from_setting(v: i32) -> Self {
        match v {
            0 => Self::Off,
            2 => Self::Full,
            _ => Self::Subtle,
        }
    }

    /// A panel's fill: opaque, ~85% (subtle, PLAN §10.6) or ~70%. The blur behind it
    /// comes with 7.2.
    pub fn panel_fill(self) -> Color32 {
        let alpha = match self {
            Self::Off => 255,
            Self::Subtle => 217,
            Self::Full => 178,
        };
        color::SURFACE.gamma_multiply_u8(alpha)
    }

    /// A HUD bar's fill: opaque when glass is off; else a translucent black that blends
    /// into dark scenes and keeps white text readable on light ones (the watermark
    /// reference: 45% black: about 65 grey over the light reference's 118).
    pub fn hud_fill(self) -> Color32 {
        match self {
            Self::Off => color::SURFACE,
            Self::Subtle => Color32::from_black_alpha(115),
            Self::Full => Color32::from_black_alpha(90),
        }
    }

    /// Secondary text on this strength's panels.
    pub fn text_secondary(self) -> Color32 {
        match self {
            Self::Full => color::TEXT_SECONDARY_FULL,
            _ => color::TEXT_SECONDARY,
        }
    }

    /// Liquid Glass's fill: light, the shader's tint does most of the darkening.
    pub fn liquid_fill() -> Color32 {
        color::SURFACE.gamma_multiply_u8(77)
    }
}

/// Inter (regular, medium, semibold; SIL OFL) and Phosphor (MIT), embedded from
/// `rust-client/assets/fonts` with their licences.
pub fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::empty();
    for (name, data) in [
        (
            "Inter-Regular",
            &include_bytes!("../../assets/fonts/Inter-Regular.ttf")[..],
        ),
        (
            "Inter-Medium",
            &include_bytes!("../../assets/fonts/Inter-Medium.ttf")[..],
        ),
        (
            "Inter-SemiBold",
            &include_bytes!("../../assets/fonts/Inter-SemiBold.ttf")[..],
        ),
        (
            "Phosphor",
            &include_bytes!("../../assets/fonts/Phosphor-Fill.ttf")[..],
        ),
    ] {
        fonts
            .font_data
            .insert(name.into(), Arc::new(FontData::from_static(data)));
    }
    for (family, font) in [
        (FontFamily::Proportional, "Inter-Regular"),
        (FontFamily::Monospace, "Inter-Regular"),
        (FontFamily::Name("medium".into()), "Inter-Medium"),
        (FontFamily::Name("semibold".into()), "Inter-SemiBold"),
        (FontFamily::Name("icons".into()), "Phosphor"),
    ] {
        // no Phosphor fallback: Inter maps the same private-use codepoints itself
        fonts.families.insert(family, vec![font.into()]);
    }
    fonts
}

/// Installs the fonts and the egui style made from the tokens.
pub fn install(ctx: &egui::Context, glass: Glass) {
    ctx.set_fonts(fonts());
    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (egui::TextStyle::Small, font(size::CAPTION, Weight::Regular)),
            (egui::TextStyle::Body, font(size::BODY, Weight::Regular)),
            (egui::TextStyle::Button, font(size::BODY, Weight::Medium)),
            (
                egui::TextStyle::Heading,
                font(size::TITLE, Weight::SemiBold),
            ),
            (
                egui::TextStyle::Monospace,
                font(size::BODY, Weight::Regular),
            ),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(space::M, space::M);
        s.spacing.button_padding = egui::vec2(space::L, space::S);
        s.spacing.window_margin = egui::Margin::same(space::XL as i8);
        s.spacing.interact_size.y = space::XXL;
        let v = &mut s.visuals;
        *v = egui::Visuals::dark();
        v.override_text_color = Some(color::TEXT);
        v.weak_text_color = Some(glass.text_secondary());
        v.panel_fill = glass.panel_fill();
        v.window_fill = glass.panel_fill();
        v.window_stroke = egui::Stroke::new(1.0, color::BORDER);
        v.window_corner_radius = egui::CornerRadius::same(radius::CARD);
        v.window_shadow = egui::Shadow::NONE;
        v.popup_shadow = POPUP_SHADOW;
        v.menu_corner_radius = egui::CornerRadius::same(radius::SMALL);
        v.selection.bg_fill = color::ACCENT;
        v.handle_shape = egui::style::HandleShape::Circle;
        v.hyperlink_color = color::ACCENT;
        v.extreme_bg_color = color::SURFACE;
        for w in [
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
            &mut v.widgets.noninteractive,
        ] {
            w.corner_radius = egui::CornerRadius::same(radius::PILL);
            w.bg_stroke = egui::Stroke::NONE;
            w.fg_stroke.color = color::TEXT;
        }
        v.widgets.noninteractive.bg_fill = color::SURFACE;
        v.widgets.inactive.bg_fill = color::SURFACE_RAISED;
        v.widgets.inactive.weak_bg_fill = color::SURFACE_RAISED;
        v.widgets.hovered.bg_fill = color::SURFACE_HOVER;
        v.widgets.hovered.weak_bg_fill = color::SURFACE_HOVER;
        // egui styles a focused widget as active: focus is a ring, not the accent, so a
        // focused button never looks like the view's one primary action
        v.widgets.active.bg_fill = color::SURFACE_HOVER;
        v.widgets.active.weak_bg_fill = color::SURFACE_HOVER;
        v.widgets.active.bg_stroke = egui::Stroke::new(1.0, color::TEXT);
        v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, color::SEPARATOR);
    });
}
