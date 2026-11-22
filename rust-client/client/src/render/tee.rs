//! Tees: `CRenderTools::RenderTee6` (game/client/render.cpp:494), the hand of
//! `CPlayers::RenderHand6` (players.cpp:85), skin and game-skin textures and the tee
//! colours of `CTeeRenderInfo::ApplyColors` (render.h). 0.7 skins are out of scope.

use ddnet::physics::vmath::{PI, Vec2, angle, vec2};

use super::anim::AnimState;
use super::content::{
    SPRITE_HOOK_CHAIN, SPRITE_HOOK_HEAD, SPRITE_OOP, SPRITE_TEE_BODY, SPRITE_TEE_BODY_OUTLINE,
    SPRITE_TEE_EYE_ANGRY, SPRITE_TEE_EYE_HAPPY, SPRITE_TEE_EYE_NORMAL, SPRITE_TEE_EYE_PAIN,
    SPRITE_TEE_EYE_SURPRISE, SPRITE_TEE_FOOT, SPRITE_TEE_FOOT_OUTLINE, SPRITE_TEE_HAND,
    SPRITE_TEE_HAND_OUTLINE, SPRITES, SPRITESETS, WEAPONS,
};
use super::map::{hsl_to_rgb, unpack_hsl};
use crate::assets::Asset;
use crate::gfx::Gfx;
use crate::gfx::batch::{Batch, ContainerQuad, TextureId};
use crate::image::{Format, Image};
use crate::skin::{Metric, SkinData, sprite_image};
use ddnet::protocol::generated as g;

/// C++ tee render flags (render.h).
pub const TEE_NO_WEAPON: i32 = 2;

/// C++: `CSkin::CSkinTextures`.
#[derive(Clone, Copy, Debug)]
pub struct SkinTextures {
    pub body: TextureId,
    pub body_outline: TextureId,
    pub feet: TextureId,
    pub feet_outline: TextureId,
    pub hands: TextureId,
    pub hands_outline: TextureId,
    pub eyes: [TextureId; 6],
}

impl SkinTextures {
    fn upload(gfx: &mut Gfx, img: &Image) -> Self {
        let mut tex = |sprite: usize| {
            let s = sprite_image(img, None, &SPRITES[sprite]);
            gfx.create_texture(s.width, s.height, &s.data)
        };
        Self {
            body: tex(SPRITE_TEE_BODY),
            body_outline: tex(SPRITE_TEE_BODY_OUTLINE),
            feet: tex(SPRITE_TEE_FOOT),
            feet_outline: tex(SPRITE_TEE_FOOT_OUTLINE),
            hands: tex(SPRITE_TEE_HAND),
            hands_outline: tex(SPRITE_TEE_HAND_OUTLINE),
            eyes: std::array::from_fn(|i| tex(SPRITE_TEE_EYE_NORMAL + i)),
        }
    }
}

/// C++: `CSkin` after `CSkins::LoadSkinFinish`.
#[derive(Clone, Copy, Debug)]
pub struct Skin {
    pub original: SkinTextures,
    pub colorable: SkinTextures,
    pub body: Metric,
    pub feet: Metric,
}

impl Skin {
    pub fn upload(gfx: &mut Gfx, data: &SkinData) -> Self {
        Self {
            original: SkinTextures::upload(gfx, &data.image),
            colorable: SkinTextures::upload(gfx, &data.grayscale),
            body: data.body,
            feet: data.feet,
        }
    }
}

/// C++: the sprite loading of `LoadGameSkin`, `LoadEmoticonsSkin`, ... (assets.cpp):
/// the image must divide into the sprites' grid (resized if not) and be RGBA, else no
/// sprite is loaded (C++ then draws those untextured).
fn sprite_textures<const N: usize>(
    gfx: &mut Gfx,
    asset: Asset,
    sprites: [usize; N],
) -> [Option<TextureId>; N] {
    let set = &SPRITESETS[SPRITES[sprites[0]].set];
    let image = asset.image.and_then(|mut i| {
        (i.check_divisibility(set.gridx as u32, set.gridy as u32, true) && i.format == Format::Rgba)
            .then_some(i)
    });
    sprites.map(|sprite| {
        let s = sprite_image(image.as_ref()?, asset.fallback.as_ref(), &SPRITES[sprite]);
        Some(gfx.create_texture(s.width, s.height, &s.to_rgba()))
    })
}

/// The game-skin and emoticon sprites the players use (C++ `m_GameSkin`,
/// `m_EmoticonsSkin`).
pub struct GameSkin {
    pub weapons: [Option<TextureId>; 6],
    pub hook_chain: Option<TextureId>,
    pub hook_head: Option<TextureId>,
    pub emoticons: [Option<TextureId>; 16],
}

impl GameSkin {
    pub fn upload(gfx: &mut Gfx, game: Asset, emoticons: Asset) -> Self {
        let [w0, w1, w2, w3, w4, w5, hook_chain, hook_head] = sprite_textures(
            gfx,
            game,
            [
                WEAPONS[0].sprite_body,
                WEAPONS[1].sprite_body,
                WEAPONS[2].sprite_body,
                WEAPONS[3].sprite_body,
                WEAPONS[4].sprite_body,
                WEAPONS[5].sprite_body,
                SPRITE_HOOK_CHAIN,
                SPRITE_HOOK_HEAD,
            ],
        );
        Self {
            weapons: [w0, w1, w2, w3, w4, w5],
            hook_chain,
            hook_head,
            emoticons: sprite_textures(gfx, emoticons, std::array::from_fn(|i| SPRITE_OOP + i)),
        }
    }
}

/// C++: `CTeeRenderInfo` (0.6 part).
#[derive(Clone, Copy, Debug)]
pub struct TeeRenderInfo {
    pub skin: Skin,
    pub custom_colored: bool,
    pub color_body: [f32; 4],
    pub color_feet: [f32; 4],
    pub size: f32,
    pub got_air_jump: bool,
    pub flags: i32,
    pub feet_flipped: bool,
}

impl TeeRenderInfo {
    pub fn new(skin: Skin) -> Self {
        Self {
            skin,
            custom_colored: false,
            color_body: [1.0; 4],
            color_feet: [1.0; 4],
            size: 1.0,
            got_air_jump: true,
            flags: 0,
            feet_flipped: false,
        }
    }

    /// C++: `CTeeRenderInfo::ApplyColors`.
    pub fn apply_colors(&mut self, custom: bool, body: i32, feet: i32) {
        self.custom_colored = custom;
        (self.color_body, self.color_feet) = if custom {
            (skin_color(body), skin_color(feet))
        } else {
            ([1.0; 4], [1.0; 4])
        };
    }

    fn textures(&self) -> &SkinTextures {
        if self.custom_colored {
            &self.skin.colorable
        } else {
            &self.skin.original
        }
    }
}

/// C++: `ColorHSLA::DARKEST_LGT`.
const DARKEST_LGT: f32 = 0.5;

/// C++: `color_cast<ColorRGBA>(ColorHSLA(Packed).UnclampLighting(DARKEST_LGT))`.
pub fn skin_color(packed: i32) -> [f32; 4] {
    let [h, s, l] = unpack_hsl(packed as u32);
    let [r, g, b] = hsl_to_rgb([h, s, DARKEST_LGT + l * (1.0 - DARKEST_LGT)]);
    [r, g, b, 1.0]
}

/// The quad containers of `CRenderTools::Init` (render.cpp:94) and `CPlayers::OnInit`
/// (players.cpp:1088), up to the hook (emoticons are not drawn yet).
pub struct Quads {
    /// Body (2), eyes (5), feet (2), mirrored feet (2).
    tee: [ContainerQuad; 11],
    /// Per weapon: normal and vertically flipped; then hand (2), hook head, hook
    /// chain, 16 emoticons.
    pub weapon: [ContainerQuad; 32],
}

const FULL: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

impl Default for Quads {
    fn default() -> Self {
        let body = ContainerQuad::centered(64.0, 64.0, FULL);
        let eye = ContainerQuad::centered(64.0 * 0.4, 64.0 * 0.4, FULL);
        let foot = ContainerQuad::new(-32.0, -16.0, 64.0, 32.0, FULL);
        let mirrored = ContainerQuad::free(
            -32.0,
            -16.0,
            64.0,
            32.0,
            [[1.0, 0.0], [0.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
        );
        let tee = [
            body, body, eye, eye, eye, eye, eye, foot, foot, mirrored, mirrored,
        ];
        let hand = ContainerQuad::centered(20.0, 20.0, FULL);
        let hook = ContainerQuad::new(-12.0, -8.0, 24.0, 16.0, FULL);
        let weapon = std::array::from_fn(|i| match i {
            0..12 => {
                let spec = &WEAPONS[i / 2];
                let s = &SPRITES[spec.sprite_body];
                // C++: GetSpriteScaleImpl
                let f = ((s.w * s.w + s.h * s.h) as f32).sqrt();
                let (sx, sy) = (s.w as f32 / f, s.h as f32 / f);
                let subset = if i % 2 == 0 {
                    FULL
                } else {
                    [0.0, 1.0, 1.0, 0.0]
                };
                ContainerQuad::centered(
                    spec.visual_size as f32 * sx,
                    spec.visual_size as f32 * sy,
                    subset,
                )
            }
            12 | 13 => hand,
            14 | 15 => hook,
            _ => ContainerQuad::centered(64.0, 64.0, FULL),
        });
        Self { tee, weapon }
    }
}

/// Quad indices into `Quads::weapon` (C++ `NUM_WEAPONS * 2 + n`).
pub const QUAD_HAND: usize = 12;
pub const QUAD_HOOK_HEAD: usize = 14;
pub const QUAD_HOOK_CHAIN: usize = 15;
pub const QUAD_EMOTICONS: usize = 16;

/// C++: `CRenderTools::RenderTee` -> `RenderTee6` (render.cpp:494), then colour white
/// and rotation 0.
#[allow(clippy::too_many_arguments)] // the C++ signature
pub fn render_tee(
    batch: &mut Batch,
    quads: &Quads,
    anim: &AnimState,
    info: &TeeRenderInfo,
    emote: i32,
    dir: Vec2,
    pos: Vec2,
    alpha: f32,
) {
    let tex = info.textures();
    // C++: GetRenderTeeAnimScaleAndBaseSize
    let (anim_scale, base_size) = (info.size * 1.0 / 64.0, info.size);
    for pass in 0..2 {
        let outline = pass == 0;
        for filling in 0..2 {
            if filling == 1 {
                batch.set_rotation(anim.body.angle * PI * 2.0);
                let c = info.color_body;
                batch.set_color([c[0], c[1], c[2], alpha]);
                let body_pos = pos + vec2(anim.body.x, anim.body.y) * anim_scale;
                // C++: GetRenderTeeBodyScale with cl_fat_skins 0
                let body_scale = base_size / 64.0;
                batch.texture_set(Some(if outline { tex.body_outline } else { tex.body }));
                batch.container_quad(
                    &quads.tee[pass],
                    body_pos.x,
                    body_pos.y,
                    body_scale,
                    body_scale,
                );
                if pass == 1 {
                    let (eye_quad, eye) = match emote {
                        g::EMOTE_PAIN => (0, SPRITE_TEE_EYE_PAIN),
                        g::EMOTE_HAPPY => (1, SPRITE_TEE_EYE_HAPPY),
                        g::EMOTE_SURPRISE => (2, SPRITE_TEE_EYE_SURPRISE),
                        g::EMOTE_ANGRY => (3, SPRITE_TEE_EYE_ANGRY),
                        _ => (4, SPRITE_TEE_EYE_NORMAL),
                    };
                    let eye_scale = base_size * 0.40;
                    let h = if emote == g::EMOTE_BLINK {
                        base_size * 0.15
                    } else {
                        eye_scale
                    };
                    let separation = (0.075 - 0.010 * dir.x.abs()) * base_size;
                    let offset = vec2(dir.x * 0.125, -0.05 + dir.y * 0.10) * base_size;
                    batch.texture_set(Some(tex.eyes[eye - SPRITE_TEE_EYE_NORMAL]));
                    let q = &quads.tee[2 + eye_quad];
                    let (sx, sy) = (eye_scale / (64.0 * 0.4), h / (64.0 * 0.4));
                    let y = body_pos.y + offset.y;
                    batch.container_quad(q, body_pos.x - separation + offset.x, y, sx, sy);
                    let sx = -eye_scale / (64.0 * 0.4);
                    batch.container_quad(q, body_pos.x + separation + offset.x, y, sx, sy);
                }
            }

            // draw feet
            let foot = if filling == 1 {
                &anim.front_foot
            } else {
                &anim.back_foot
            };
            let (w, h) = (base_size, base_size / 2.0);
            let mut quad = 7;
            if dir.x < 0.0 && info.feet_flipped {
                quad += 2;
            }
            batch.set_rotation(foot.angle * PI * 2.0);
            // C++ cl_airjumpindicator 1
            let indicate = !info.got_air_jump;
            let mut color_scale = 1.0;
            if !outline {
                quad += 1;
                if indicate {
                    color_scale = 0.5;
                }
            }
            let c = info.color_feet;
            batch.set_color([
                c[0] * color_scale,
                c[1] * color_scale,
                c[2] * color_scale,
                alpha,
            ]);
            batch.texture_set(Some(if outline { tex.feet_outline } else { tex.feet }));
            batch.container_quad(
                &quads.tee[quad],
                pos.x + foot.x * anim_scale,
                pos.y + foot.y * anim_scale,
                w / 64.0,
                h / 32.0,
            );
        }
    }
    batch.set_color([1.0; 4]);
    batch.set_rotation(0.0);
}

/// C++: `CPlayers::RenderHand` with `RenderHand6` (players.cpp:53).
#[allow(clippy::too_many_arguments)] // the C++ signature
pub fn render_hand(
    batch: &mut Batch,
    quads: &Quads,
    info: &TeeRenderInfo,
    center: Vec2,
    dir: Vec2,
    angle_offset: f32,
    post_rot_offset: Vec2,
    alpha: f32,
) {
    // C++: CalculateHandPosition
    let mut dir_y = vec2(-dir.y, dir.x);
    if dir.x < 0.0 {
        dir_y = vec2(-dir_y.x, -dir_y.y);
    }
    let hand_pos = center + dir + dir * post_rot_offset.x + dir_y * post_rot_offset.y;
    // C++: CalculateHandAngle
    let a = angle(dir);
    let hand_angle = if dir.x < 0.0 {
        a - angle_offset
    } else {
        a + angle_offset
    };
    let tex = info.textures();
    let c = info.color_body;
    batch.set_color([c[0], c[1], c[2], alpha]);
    batch.set_rotation(hand_angle);
    batch.texture_set(Some(tex.hands_outline));
    batch.container_quad(&quads.weapon[QUAD_HAND], hand_pos.x, hand_pos.y, 1.0, 1.0);
    batch.texture_set(Some(tex.hands));
    batch.container_quad(
        &quads.weapon[QUAD_HAND + 1],
        hand_pos.x,
        hand_pos.y,
        1.0,
        1.0,
    );
}
