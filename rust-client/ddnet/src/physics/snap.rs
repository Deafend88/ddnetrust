//! Snapshot entities into the prediction world: C++ `CGameWorld::NetObjAdd`
//! (gameworld.cpp:437), `FindMatch` (:721) and the entities' `Match`/`Read`.

use super::core::PHYSICAL_SIZE;
use super::door::Door;
use super::laser::Laser;
use super::pickup::{Dragger, Pickup, Plasma};
use super::projectile::Projectile;
use super::tuning::Param;
use super::vmath::{distance, length, normalize};
use super::world::GameWorld;
use crate::items::{LaserData, extract_laser, extract_pickup, extract_projectile};
use crate::protocol::generated::{self as g, ObjEntityEx};

/// An entity `FindMatch` found: its list and index in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityRef {
    Projectile(usize),
    Laser(usize),
    Door(usize),
    Dragger(usize),
    Plasma(usize),
    Pickup(usize),
}

impl Projectile {
    /// C++: `CProjectile::Match` (projectile.cpp:231), `self` being the snapshot one.
    pub fn matches(&self, other: &Projectile) -> bool {
        other.type_ == self.type_
            && other.start_tick == self.start_tick
            && distance(other.pos, self.pos) <= 2.0
            && distance(other.direction, self.direction) <= 2.0
    }
}

impl Laser {
    /// C++: `CLaser(CGameWorld *, int Id, const CLaserData *)` (laser.cpp:187). Only used
    /// to match snapshot lasers; never inserted (its bounce count stays unset in C++).
    pub fn from_data(w: &GameWorld, id: i32, d: &LaserData) -> Self {
        let pos = d.to;
        let tune_zone = if w.config.use_tune_zones {
            w.col.is_tune(w.col.get_map_index(pos))
        } else {
            0
        };
        let mut energy = w.tunings[tune_zone as usize].get(Param::LaserReach);
        if w.config.is_fng && energy < 10.0 {
            energy = 800.0;
        }
        let mut dir = pos - d.from;
        if length(pos - d.from) > 0.001 {
            dir = normalize(dir);
        } else {
            energy = 0.0;
        }
        Laser {
            id,
            pos,
            from: d.from,
            dir,
            prev_pos: d.from,
            energy,
            bounces: 0,
            eval_tick: d.start_tick,
            owner: -2,
            type_: if d.type_ == g::LASERTYPE_SHOTGUN {
                g::WEAPON_SHOTGUN
            } else {
                g::WEAPON_LASER
            },
            zero_energy_bounce_in_last_tick: false,
            tune_zone,
            marked_for_destroy: false,
        }
    }

    /// C++: `CLaser::Match` (laser.cpp:211), `self` being the snapshot one.
    pub fn matches(&self, other: &Laser) -> bool {
        if other.eval_tick != self.eval_tick || distance(other.from, self.from) > 2.0 {
            return false;
        }
        let this_diff = self.pos - self.from;
        let other_diff = other.pos - other.from;
        distance(normalize(other_diff) * length(this_diff), this_diff) <= 2.0
    }
}

impl Pickup {
    /// C++: `CPickup::Match` (pickup.cpp:190).
    pub fn matches(&self, other: &Pickup) -> bool {
        other.type_ == self.type_
            && other.subtype == self.subtype
            && distance(other.pos, self.pos) <= 2.0
    }
}

impl Dragger {
    /// C++: `CDragger::Match` (dragger.cpp:157).
    pub fn matches(&self, other: &Dragger) -> bool {
        other.strength == self.strength
            && other.number == self.number
            && other.ignore_walls == self.ignore_walls
    }

    /// C++: `CDragger::Read` (dragger.cpp:151).
    fn read(&mut self, d: &LaserData) {
        self.pos = d.from;
        self.target_id = d.owner;
    }
}

impl Plasma {
    /// C++: `CPlasma::Match` (plasma.cpp:34).
    pub fn matches(&self, other: &Plasma) -> bool {
        other.eval_tick == self.eval_tick
            && other.number == self.number
            && other.explosive == self.explosive
            && other.freeze == self.freeze
            && other.for_client_id == self.for_client_id
    }

    /// C++: `CPlasma::Read` (plasma.cpp:40).
    fn read(&mut self, d: &LaserData) {
        self.pos = d.from;
        self.eval_tick = d.start_tick;
        self.for_client_id = d.owner;
        if (0..g::NUM_LASERGUNTYPES).contains(&d.subtype) {
            self.explosive = d.subtype & 1 != 0;
            self.freeze = d.subtype & 2 != 0;
        }
    }
}

fn is_projectile(type_id: i32) -> bool {
    [
        g::NETOBJTYPE_PROJECTILE,
        g::NETOBJTYPE_DDRACEPROJECTILE,
        g::NETOBJTYPE_DDNETPROJECTILE,
    ]
    .contains(&type_id)
}

impl GameWorld {
    /// C++: `CGameWorld::IsLocalTeam`. ponytail: without a local player (-1) C++ indexes
    /// its teams out of bounds; everything counts as local then.
    fn is_local_team(&self, owner: i32) -> bool {
        owner < 0 || self.local_id < 0 || self.teams.can_collide(self.local_id, owner)
    }

    fn tunes(&self) -> Option<&super::collision::Collision> {
        self.config.use_tune_zones.then_some(&*self.col)
    }

    /// C++: `CGameWorld::NetObjAdd` (gameworld.cpp:437) for a projectile, pickup or laser
    /// object. `data` holds at least the object's ints.
    pub fn net_obj_add(&mut self, id: i32, type_id: i32, data: &[i32], ex: Option<&ObjEntityEx>) {
        let cfg = self.config;
        if is_projectile(type_id) && cfg.predict_weapons {
            let d = extract_projectile(type_id, data, self.tunes(), ex);
            if !self.is_local_team(d.owner) {
                return;
            }
            let mut net = Projectile::from_data(self, id, &d);
            // workaround to skip grenades on ball mod
            if net.type_ != g::WEAPON_SHOTGUN && (length(net.direction) - 1.0).abs() > 0.02 {
                return;
            }
            let tick = self.game_tick;
            if let Some(p) = self.projectiles.iter_mut().find(|p| p.id == id)
                && net.matches(p)
            {
                p.marked_for_destroy = false; // Keep
                if p.type_ == g::WEAPON_SHOTGUN && cfg.is_ddrace {
                    p.life_span = 20 * super::core::SERVER_TICK_SPEED - (tick - p.start_tick);
                }
                return;
            }
            if !d.extra_info {
                // a locally fired projectile the server now sends
                if let Some(p) = self
                    .projectiles
                    .iter_mut()
                    .find(|p| p.id == -1 && net.matches(p))
                {
                    p.id = id;
                    p.marked_for_destroy = false;
                    return;
                }
                // otherwise the owner is the only player near its start
                if net.start_tick >= tick - 4 {
                    let net_pos = net.pos - normalize(net.direction) * PHYSICAL_SIZE * 0.75;
                    let prev = (tick - net.start_tick) > 1;
                    let (mut first, mut second) = (200.0f32, 200.0f32);
                    let mut closest = None;
                    for &i in &self.order {
                        let ch = self.chars[i].as_ref().unwrap();
                        let dist =
                            distance(if prev { ch.prev_prev_pos } else { ch.prev_pos }, net_pos);
                        if dist < first {
                            closest = Some(i);
                            first = dist;
                        } else if dist < second {
                            second = dist;
                        }
                    }
                    if let Some(c) = closest
                        && first.max(2.0) * 1.2 < second
                    {
                        net.owner = c as i32;
                    }
                }
            }
            self.projectiles.insert(0, net);
        } else if (type_id == g::NETOBJTYPE_PICKUP || type_id == g::NETOBJTYPE_DDNETPICKUP)
            && cfg.predict_weapons
        {
            let d = extract_pickup(type_id, data, ex);
            if d.flags & g::PICKUPFLAG_NO_PREDICT != 0 {
                return;
            }
            let net = Pickup::from_data(id, &d);
            if let Some(p) = self.pickups.iter_mut().find(|p| p.id == id)
                && net.matches(p)
            {
                p.pos = net.pos;
                p.marked_for_destroy = false;
                return;
            }
            self.pickups.push(net);
        } else if type_id == g::NETOBJTYPE_LASER || type_id == g::NETOBJTYPE_DDNETLASER {
            let d = extract_laser(type_id, data, self.tunes(), ex);
            if !self.is_local_team(d.owner) || !d.predict {
                return;
            }
            // doors are static world geometry: they follow the tile config
            let wanted = if d.type_ == g::LASERTYPE_DOOR {
                cfg.predict_tiles
            } else {
                cfg.predict_weapons
            };
            if !wanted {
                return;
            }
            match d.type_ {
                t if t == g::LASERTYPE_RIFLE || t == g::LASERTYPE_SHOTGUN || t < 0 => {
                    let net = Laser::from_data(self, id, &d);
                    let matching = match self.lasers.iter().position(|l| l.id == id) {
                        Some(i) if net.matches(&self.lasers[i]) => Some(i),
                        _ => self
                            .lasers
                            .iter()
                            .position(|l| l.id == -1 && net.matches(l)),
                    };
                    if let Some(i) = matching {
                        let l = &mut self.lasers[i];
                        l.id = id;
                        l.marked_for_destroy = false;
                        if distance(net.from, net.pos) < distance(l.from, l.pos) - 2.0 {
                            // the laser stopped earlier than predicted
                            l.energy = 0.0;
                            l.pos = net.pos;
                        }
                    }
                }
                g::LASERTYPE_DRAGGER => {
                    let net = Dragger::from_data(id, &d);
                    if net.strength > 0.0 {
                        if let Some(dr) = self.draggers.iter_mut().find(|dr| dr.id == id)
                            && net.matches(dr)
                        {
                            dr.marked_for_destroy = false;
                            dr.read(&d);
                            return;
                        }
                        self.draggers.insert(0, net);
                    }
                }
                g::LASERTYPE_DOOR => {
                    let net = Door::new(id, d.from, d.to, d.switch_number);
                    if let Some(door) = self.doors.iter_mut().find(|door| door.id == id)
                        && net.matches(door)
                    {
                        door.marked_for_destroy = false; // C++: Keep (Read sets the same positions)
                        return;
                    }
                    self.doors.insert(0, net);
                }
                g::LASERTYPE_PLASMA => {
                    let net = Plasma::from_data(self, id, &d);
                    if let Some(p) = self.plasmas.iter_mut().find(|p| p.id == id)
                        && net.matches(p)
                    {
                        p.marked_for_destroy = false;
                        p.read(&d);
                        return;
                    }
                    self.plasmas.insert(0, net);
                }
                _ => {}
            }
        }
    }

    /// C++: `CGameWorld::FindMatch` (gameworld.cpp:721) for snapshot entities (the
    /// character case is not used by the client's renderer and not ported).
    pub fn find_match(&self, id: i32, type_id: i32, data: &[i32]) -> Option<EntityRef> {
        if is_projectile(type_id) {
            let d = extract_projectile(type_id, data, self.tunes(), None);
            let i = self.projectiles.iter().position(|p| p.id == id)?;
            return Projectile::from_data(self, id, &d)
                .matches(&self.projectiles[i])
                .then_some(EntityRef::Projectile(i));
        }
        if type_id == g::NETOBJTYPE_PICKUP || type_id == g::NETOBJTYPE_DDNETPICKUP {
            let d = extract_pickup(type_id, data, None);
            let i = self.pickups.iter().position(|p| p.id == id)?;
            return Pickup::from_data(id, &d)
                .matches(&self.pickups[i])
                .then_some(EntityRef::Pickup(i));
        }
        if type_id != g::NETOBJTYPE_LASER && type_id != g::NETOBJTYPE_DDNETLASER {
            return None;
        }
        let d = extract_laser(type_id, data, self.tunes(), None);
        match d.type_ {
            g::LASERTYPE_RIFLE | g::LASERTYPE_SHOTGUN => {
                let i = self.lasers.iter().position(|l| l.id == id)?;
                Laser::from_data(self, id, &d)
                    .matches(&self.lasers[i])
                    .then_some(EntityRef::Laser(i))
            }
            g::LASERTYPE_DRAGGER => {
                let i = self.draggers.iter().position(|dr| dr.id == id)?;
                Dragger::from_data(id, &d)
                    .matches(&self.draggers[i])
                    .then_some(EntityRef::Dragger(i))
            }
            g::LASERTYPE_DOOR => {
                let i = self.doors.iter().position(|door| door.id == id)?;
                Door::new(id, d.from, d.to, d.switch_number)
                    .matches(&self.doors[i])
                    .then_some(EntityRef::Door(i))
            }
            g::LASERTYPE_PLASMA => {
                let i = self.plasmas.iter().position(|p| p.id == id)?;
                Plasma::from_data(self, id, &d)
                    .matches(&self.plasmas[i])
                    .then_some(EntityRef::Plasma(i))
            }
            _ => None,
        }
    }
}
