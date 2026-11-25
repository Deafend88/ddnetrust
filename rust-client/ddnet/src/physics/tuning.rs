//! Tuning parameters. C++: `src/game/tuning.h` with `CTuneParam`/`CTuningParams`
//! (gamecore.h). Values are stored as `int` hundredths, so reading one back can differ
//! from the float it was set from; that rounding is part of the physics.

use super::vmath::trunc_i32;

macro_rules! params {
    ($($name:ident $script:literal = $default:expr,)*) => {
        /// One tuning parameter, in `tuning.h` order (the network order of `Sv_TuneParams`).
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Param {
            $($name,)*
        }
        const DEFAULTS: &[f32] = &[$($default,)*];
        /// C++: the `ScriptName`s of tuning.h (`CTuningParams::Name`).
        const NAMES: &[&str] = &[$($script,)*];
    };
}

params! {
    GroundControlSpeed "ground_control_speed" = 10.0,
    GroundControlAccel "ground_control_accel" = 100.0 / 50.0,
    GroundFriction "ground_friction" = 0.5,
    GroundJumpImpulse "ground_jump_impulse" = 13.2,
    AirJumpImpulse "air_jump_impulse" = 12.0,
    AirControlSpeed "air_control_speed" = 250.0 / 50.0,
    AirControlAccel "air_control_accel" = 1.5,
    AirFriction "air_friction" = 0.95,
    HookLength "hook_length" = 380.0,
    HookFireSpeed "hook_fire_speed" = 80.0,
    HookDragAccel "hook_drag_accel" = 3.0,
    HookDragSpeed "hook_drag_speed" = 15.0,
    Gravity "gravity" = 0.5,
    VelrampStart "velramp_start" = 550.0,
    VelrampRange "velramp_range" = 2000.0,
    VelrampCurvature "velramp_curvature" = 1.4,
    GunCurvature "gun_curvature" = 1.25,
    GunSpeed "gun_speed" = 2200.0,
    GunLifetime "gun_lifetime" = 2.0,
    ShotgunCurvature "shotgun_curvature" = 1.25,
    ShotgunSpeed "shotgun_speed" = 2750.0,
    ShotgunSpeeddiff "shotgun_speeddiff" = 0.8,
    ShotgunLifetime "shotgun_lifetime" = 0.20,
    GrenadeCurvature "grenade_curvature" = 7.0,
    GrenadeSpeed "grenade_speed" = 1000.0,
    GrenadeLifetime "grenade_lifetime" = 2.0,
    LaserReach "laser_reach" = 800.0,
    LaserBounceDelay "laser_bounce_delay" = 150.0,
    LaserBounceNum "laser_bounce_num" = 1000.0,
    LaserBounceCost "laser_bounce_cost" = 0.0,
    LaserDamage "laser_damage" = 5.0,
    PlayerCollision "player_collision" = 1.0,
    PlayerHooking "player_hooking" = 1.0,
    JetpackStrength "jetpack_strength" = 400.0,
    ShotgunStrength "shotgun_strength" = 10.0,
    ExplosionStrength "explosion_strength" = 6.0,
    HammerStrength "hammer_strength" = 1.0,
    HookDuration "hook_duration" = 1.25,
    HammerFireDelay "hammer_fire_delay" = 125.0,
    GunFireDelay "gun_fire_delay" = 125.0,
    ShotgunFireDelay "shotgun_fire_delay" = 500.0,
    GrenadeFireDelay "grenade_fire_delay" = 500.0,
    LaserFireDelay "laser_fire_delay" = 800.0,
    NinjaFireDelay "ninja_fire_delay" = 800.0,
    HammerHitFireDelay "hammer_hit_fire_delay" = 320.0,
    GroundElasticityX "ground_elasticity_x" = 0.0,
    GroundElasticityY "ground_elasticity_y" = 0.0,
}

pub const NUM_PARAMS: usize = DEFAULTS.len();

/// C++: `CTuningParams`; the ints are what goes over the network.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tuning(pub [i32; NUM_PARAMS]);

impl Default for Tuning {
    fn default() -> Self {
        let mut t = Self([0; NUM_PARAMS]);
        for (v, &d) in t.0.iter_mut().zip(DEFAULTS) {
            *v = to_fixed(d);
        }
        t
    }
}

/// C++: `CTuneParam::operator=(float)`: `(int)(v * 100.0f)` if in range, else `INT_MIN`.
fn to_fixed(v: f32) -> i32 {
    trunc_i32(v * 100.0)
}

impl Tuning {
    /// C++: `CTuneParam::operator float`.
    pub fn get(&self, p: Param) -> f32 {
        self.0[p as usize] as f32 / 100.0
    }

    pub fn set(&mut self, p: Param, v: f32) {
        self.0[p as usize] = to_fixed(v);
    }

    /// C++: `CTuningParams::Set(const char *pName, float Value)`: the name is compared
    /// case-insensitively; `false` if there is no such parameter.
    pub fn set_by_name(&mut self, name: &str, v: f32) -> bool {
        match NAMES.iter().position(|n| n.eq_ignore_ascii_case(name)) {
            Some(i) => {
                self.0[i] = to_fixed(v);
                true
            }
            None => false,
        }
    }

    /// C++: the tune zone reset of `CGameClient::LoadMapSettings` (gameclient.cpp:4213).
    pub fn map_default() -> Self {
        let mut t = Self::default();
        for (name, v) in [
            ("gun_curvature", 0.0),
            ("gun_speed", 1400.0),
            ("shotgun_curvature", 0.0),
            ("shotgun_speed", 500.0),
            ("shotgun_speeddiff", 0.0),
        ] {
            t.set_by_name(name, v);
        }
        t
    }

    /// C++: the `NETMSGTYPE_SV_TUNEPARAMS` branch of `CGameClient::OnMessage`
    /// (gameclient.cpp:1053): defaults with jetpack strength 0, then the ints that were
    /// read before the first unpack error.
    pub fn from_net(ints: &[i32]) -> Self {
        let mut t = Self::default();
        t.0[Param::JetpackStrength as usize] = to_fixed(0.0);
        for (v, &i) in t.0.iter_mut().zip(ints) {
            *v = i;
        }
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_through_hundredths() {
        let t = Tuning::default();
        assert_eq!(NUM_PARAMS, 47);
        assert_eq!(t.0[Param::AirFriction as usize], 95);
        assert_eq!(t.0[Param::VelrampCurvature as usize], 140);
        assert_eq!(t.get(Param::HookDuration), 1.25);
        assert_eq!(t.get(Param::GroundControlAccel), 2.0);
    }
}
