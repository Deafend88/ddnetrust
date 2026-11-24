//! Map and game rendering (M6.3+). See `docs/PLAN.md` section 9.

pub mod anim;
pub mod content;
pub mod envelope;
pub mod items;
pub mod map;
pub mod players;
pub mod tee;

pub use ddnet::net::client::calc_screen_params;

/// C++: `IGraphics::MapScreenToWorld`: the screen rectangle (left, top, right, bottom)
/// of a map group with this parallax and offset around the camera center.
#[allow(clippy::too_many_arguments)] // the C++ signature
pub fn map_screen_to_world(
    center_x: f32,
    center_y: f32,
    parallax_x: f32,
    parallax_y: f32,
    parallax_zoom: f32,
    offset_x: f32,
    offset_y: f32,
    aspect: f32,
    zoom: f32,
) -> [f32; 4] {
    let (mut w, mut h) = calc_screen_params(aspect, zoom);
    let scale = (parallax_zoom * (zoom - 1.0) + 100.0) / 100.0 / zoom;
    w *= scale;
    h *= scale;
    let (cx, cy) = (
        center_x * (parallax_x / 100.0),
        center_y * (parallax_y / 100.0),
    );
    // C++: CScreenRect(Left, Top, Width, Height)
    let (left, top) = (offset_x + cx - w / 2.0, offset_y + cy - h / 2.0);
    [left, top, left + w, top + h]
}
