//! Snapshot entities as the C++ client reads them: `ExtractProjectileInfo`
//! (game/client/projectile_data.cpp), `ExtractPickupInfo` (pickup_data.cpp),
//! `ExtractLaserInfo` (laser_data.cpp) and `CalcPos` (gamecore.h). `data` holds at least
//! the object's ints (`pad`).

use crate::physics::collision::Collision;
use crate::physics::vmath::{Vec2, normalize, vec2};
use crate::protocol::generated::{
    self as g, ObjDDNetLaser, ObjDDNetPickup, ObjDDNetProjectile, ObjDDRaceProjectile, ObjEntityEx,
    ObjLaser, ObjPickup, ObjProjectile,
};

/// C++ `MAX_CLIENTS`.
const MAX_CLIENTS: i32 = 128;

/// A snapshot item's ints, zero-padded to `n` (shorter items come from older servers).
pub fn pad(data: &[i32], n: usize) -> Vec<i32> {
    let mut v = data.to_vec();
    v.resize(v.len().max(n), 0);
    v
}

/// C++: `CalcPos`.
pub fn calc_pos(pos: Vec2, vel: Vec2, curvature: f32, speed: f32, time: f32) -> Vec2 {
    let time = time * speed;
    vec2(
        pos.x + vel.x * time,
        pos.y + vel.y * time + curvature / 10000.0 * (time * time),
    )
}

/// C++: the tune zone at `pos` when the world uses tune zones, else 0.
fn tune_zone(tunes: Option<&Collision>, pos: Vec2) -> i32 {
    tunes.map_or(0, |c| c.is_tune(c.get_map_index(pos)))
}

/// C++: `CProjectileData`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ProjectileData {
    pub start_pos: Vec2,
    pub start_vel: Vec2,
    pub type_: i32,
    pub start_tick: i32,
    pub extra_info: bool,
    pub owner: i32,
    pub explosive: bool,
    pub bouncing: i32,
    pub freeze: bool,
    pub switch_number: i32,
    pub tune_zone: i32,
}

/// C++: `ExtractProjectileInfo`. `tunes`: the collision when the world uses tune zones.
pub fn extract_projectile(
    type_id: i32,
    data: &[i32],
    tunes: Option<&Collision>,
    ex: Option<&ObjEntityEx>,
) -> ProjectileData {
    let switch_number = ex.map_or(0, |e| e.switch_number);
    if type_id == g::NETOBJTYPE_DDNETPROJECTILE {
        let p = ObjDDNetProjectile::from_ints(data);
        let v = vec2(p.vel_x as f32, p.vel_y as f32);
        let mut start_vel = if p.owner < 0 {
            vec2(v.x / 1e6, v.y / 1e6)
        } else {
            v
        };
        if p.flags & g::PROJECTILEFLAG_NORMALIZE_VEL != 0 {
            start_vel = normalize(start_vel);
        }
        let flag = |f: i32| p.flags & f != 0;
        return ProjectileData {
            start_pos: vec2(p.x as f32 / 100.0, p.y as f32 / 100.0),
            start_vel,
            type_: p.type_,
            start_tick: p.start_tick,
            extra_info: true,
            owner: p.owner,
            explosive: flag(g::PROJECTILEFLAG_EXPLOSIVE),
            bouncing: flag(g::PROJECTILEFLAG_BOUNCE_HORIZONTAL) as i32
                | (flag(g::PROJECTILEFLAG_BOUNCE_VERTICAL) as i32) << 1,
            freeze: flag(g::PROJECTILEFLAG_FREEZE),
            switch_number: p.switch_number,
            tune_zone: p.tune_zone,
        };
    }
    let p = ObjProjectile::from_ints(data);
    // C++: UseProjectileExtraInfo
    let extra = p.vel_y >= 0 && p.vel_y & g::LEGACYPROJECTILEFLAG_IS_DDNET != 0;
    if type_id == g::NETOBJTYPE_DDRACEPROJECTILE || extra {
        // C++: ExtractProjectileInfoDDRace
        let p = ObjDDRaceProjectile::from_ints(data);
        let start_pos = vec2(p.x as f32 / 100.0, p.y as f32 / 100.0);
        let angle = p.angle as f32 / 1000000.0;
        let mut owner = p.data & 255;
        if p.data & g::LEGACYPROJECTILEFLAG_NO_OWNER != 0 || !(0..MAX_CLIENTS).contains(&owner) {
            owner = -1;
        }
        return ProjectileData {
            start_pos,
            start_vel: vec2((-angle).sin(), (-angle).cos()),
            type_: p.type_,
            start_tick: p.start_tick,
            extra_info: true,
            owner,
            bouncing: (p.data >> 10) & 3,
            explosive: p.data & g::LEGACYPROJECTILEFLAG_EXPLOSIVE != 0,
            freeze: p.data & g::LEGACYPROJECTILEFLAG_FREEZE != 0,
            tune_zone: tune_zone(tunes, start_pos),
            switch_number,
        };
    }
    let start_pos = vec2(p.x as f32, p.y as f32);
    ProjectileData {
        start_pos,
        start_vel: vec2(p.vel_x as f32 / 100.0, p.vel_y as f32 / 100.0),
        type_: p.type_,
        start_tick: p.start_tick,
        extra_info: false,
        owner: -1,
        tune_zone: tune_zone(tunes, start_pos),
        switch_number,
        ..Default::default()
    }
}

/// C++: `CPickupData`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PickupData {
    pub pos: Vec2,
    pub type_: i32,
    pub subtype: i32,
    pub switch_number: i32,
    pub flags: i32,
}

/// C++: `ExtractPickupInfo`.
pub fn extract_pickup(type_id: i32, data: &[i32], ex: Option<&ObjEntityEx>) -> PickupData {
    if type_id == g::NETOBJTYPE_DDNETPICKUP {
        let p = ObjDDNetPickup::from_ints(data);
        return PickupData {
            pos: vec2(p.x as f32, p.y as f32),
            type_: p.type_,
            subtype: p.subtype,
            switch_number: p.switch_number,
            flags: p.flags,
        };
    }
    let p = ObjPickup::from_ints(data);
    PickupData {
        pos: vec2(p.x as f32, p.y as f32),
        type_: p.type_,
        subtype: p.subtype,
        switch_number: ex.map_or(0, |e| e.switch_number),
        flags: 0,
    }
}

/// C++: `CLaserData`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LaserData {
    pub from: Vec2,
    pub to: Vec2,
    pub start_tick: i32,
    pub extra_info: bool,
    pub owner: i32,
    pub type_: i32,
    pub switch_number: i32,
    pub subtype: i32,
    pub tune_zone: i32,
    pub predict: bool,
}

/// C++: `ExtractLaserInfo`.
pub fn extract_laser(
    type_id: i32,
    data: &[i32],
    tunes: Option<&Collision>,
    ex: Option<&ObjEntityEx>,
) -> LaserData {
    let ddnet = type_id == g::NETOBJTYPE_DDNETLASER;
    let mut r = if ddnet {
        let l = ObjDDNetLaser::from_ints(data);
        let from = vec2(l.from_x as f32, l.from_y as f32);
        LaserData {
            from,
            to: vec2(l.to_x as f32, l.to_y as f32),
            start_tick: l.start_tick,
            extra_info: true,
            owner: l.owner,
            type_: l.type_,
            switch_number: l.switch_number,
            subtype: l.subtype,
            tune_zone: tune_zone(tunes, from),
            predict: l.flags & g::LASERFLAG_NO_PREDICT == 0,
        }
    } else {
        let l = ObjLaser::from_ints(data);
        let from = vec2(l.from_x as f32, l.from_y as f32);
        LaserData {
            from,
            to: vec2(l.x as f32, l.y as f32),
            start_tick: l.start_tick,
            extra_info: false,
            owner: -1,
            type_: -1,
            switch_number: 0,
            subtype: -1,
            tune_zone: tune_zone(tunes, from),
            predict: true,
        }
    };
    if let Some(ex) = ex
        && !(ddnet && r.switch_number >= 0)
    {
        r.switch_number = ex.switch_number;
        r.type_ = match ex.entity_class {
            g::ENTITYCLASS_LIGHT => g::LASERTYPE_FREEZE,
            g::ENTITYCLASS_GUN_NORMAL..=g::ENTITYCLASS_GUN_UNFREEZE => g::LASERTYPE_GUN,
            g::ENTITYCLASS_DRAGGER_WEAK..=g::ENTITYCLASS_DRAGGER_STRONG => g::LASERTYPE_DRAGGER,
            g::ENTITYCLASS_DOOR => g::LASERTYPE_DOOR,
            _ => r.type_,
        };
    }
    r
}

/// The snapshot entities C++ `CGameClient::SnapCollectEntities` (gameclient.cpp:4344)
/// collects: pickups, lasers and projectiles sorted by id, each with the `EntityEx` of
/// the same id. Returns (item index, EntityEx) pairs.
pub fn snap_entities(
    items: &[crate::protocol::snapshot::Item],
) -> Vec<(usize, Option<ObjEntityEx>)> {
    const TYPES: [i32; 7] = [
        g::NETOBJTYPE_PICKUP,
        g::NETOBJTYPE_DDNETPICKUP,
        g::NETOBJTYPE_LASER,
        g::NETOBJTYPE_DDNETLASER,
        g::NETOBJTYPE_PROJECTILE,
        g::NETOBJTYPE_DDRACEPROJECTILE,
        g::NETOBJTYPE_DDNETPROJECTILE,
    ];
    let mut ents: Vec<usize> = (0..items.len())
        .filter(|&i| TYPES.contains(&items[i].type_id))
        .collect();
    // ponytail: stable where C++ std::sort is not; servers give every entity its own id
    ents.sort_by_key(|&i| items[i].id);
    ents.into_iter()
        .map(|i| {
            let ex = items
                .iter()
                .find(|it| it.type_id == g::NETOBJTYPE_ENTITYEX && it.id == items[i].id)
                .map(|it| ObjEntityEx::from_ints(&pad(&it.data, ObjEntityEx::NUM_INTS)));
            (i, ex)
        })
        .collect()
}
