//! The crosshair: C++ `CHud::RenderCursor` (hud.cpp:609) for the local tee, with the
//! target position of `CControls::OnRender` (controls.cpp:424). The spectated player's
//! cursor (`CCursorInfo`, `cl_spec_cursor`) comes with spectating (PLAN §13.7).

use super::content::{
    SPRITE_WEAPON_GRENADE_CURSOR, SPRITE_WEAPON_GUN_CURSOR, SPRITE_WEAPON_HAMMER_CURSOR,
    SPRITE_WEAPON_LASER_CURSOR, SPRITE_WEAPON_NINJA_CURSOR, SPRITE_WEAPON_SHOTGUN_CURSOR, SPRITES,
};
use super::tee::Sprites;
use crate::gfx::batch::{Batch, ContainerQuad};
use ddnet::physics::vmath::Vec2;

/// C++ `m_GameSkin.m_aSpriteWeaponCursors`, by weapon.
const CURSORS: [usize; 6] = [
    SPRITE_WEAPON_HAMMER_CURSOR,
    SPRITE_WEAPON_GUN_CURSOR,
    SPRITE_WEAPON_SHOTGUN_CURSOR,
    SPRITE_WEAPON_GRENADE_CURSOR,
    SPRITE_WEAPON_LASER_CURSOR,
    SPRITE_WEAPON_NINJA_CURSOR,
];

/// C++ `CControls::OnRender`'s target position while playing: the tee plus the mouse,
/// corrected for the smooth dyncam so the cursor stays still in the world when zoomed.
pub fn target_pos(
    local: Vec2,
    mouse: Vec2,
    dyncam_target: Vec2,
    dyncam_current: Vec2,
    zoom: f32,
) -> Vec2 {
    let delta = dyncam_target - dyncam_current;
    local + mouse - delta + delta * (1.0 / zoom)
}

/// C++ `CHud::RenderCursor` for the local tee: the active weapon's cursor sprite at
/// `target`, in a screen mapped around `center` at zoom 1 (`m_aCursorOffset`: 64 units
/// scaled by the sprite's aspect).
pub fn render(
    batch: &mut Batch,
    sprites: &Sprites,
    weapon: i32,
    target: Vec2,
    center: Vec2,
    aspect: f32,
) {
    let weapon = weapon.max(0) as usize % CURSORS.len();
    let sprite = CURSORS[weapon];
    let r = super::map_screen_to_world(
        center.x, center.y, 100.0, 100.0, 100.0, 0.0, 0.0, aspect, 1.0,
    );
    batch.map_screen(r[0], r[1], r[2], r[3]);
    let s = &SPRITES[sprite];
    let f = ((s.w * s.w + s.h * s.h) as f32).sqrt();
    let (w, h) = (64.0 * s.w as f32 / f, 64.0 * s.h as f32 / f);
    let quad = ContainerQuad::new(-w / 2.0, -h / 2.0, w, h, [0.0, 0.0, 1.0, 1.0]);
    batch.set_color([1.0, 1.0, 1.0, 1.0]);
    batch.texture_set(sprites.get(sprite));
    batch.container_quad(&quad, target.x, target.y, 1.0, 1.0);
}
