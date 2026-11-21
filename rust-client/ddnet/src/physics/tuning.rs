//! Tuning parameters. C++: `src/game/tuning.h` with `CTuneParam`/`CTuningParams`
//! (gamecore.h). Values are stored as `int` hundredths, so reading one back can differ
//! from the float it was set from; that rounding is part of the physics.

use super::vmath::trunc_i32;

macro_rules! params {
    ($($name:ident = $default:expr,)*) => {
        /// One tuning parameter, in `tuning.h` order (the network order of `Sv_TuneParams`).
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Param {
            $($name,)*
        }
        const DEFAULTS: &[f32] = &[$($default,)*];
    };
}

params! {
    GroundControlSpeed = 10.0,
    GroundControlAccel = 100.0 / 50.0,
    GroundFriction = 0.5,
    GroundJumpImpulse = 13.2,
    AirJumpImpulse = 12.0,
    AirControlSpeed = 250.0 / 50.0,
    AirControlAccel = 1.5,
    AirFriction = 0.95,
    HookLength = 380.0,
    HookFireSpeed = 80.0,
    HookDragAccel = 3.0,
    HookDragSpeed = 15.0,
    Gravity = 0.5,
    VelrampStart = 550.0,
    VelrampRange = 2000.0,
    VelrampCurvature = 1.4,
    GunCurvature = 1.25,
    GunSpeed = 2200.0,
    GunLifetime = 2.0,
    ShotgunCurvature = 1.25,
    ShotgunSpeed = 2750.0,
    ShotgunSpeeddiff = 0.8,
    ShotgunLifetime = 0.20,
    GrenadeCurvature = 7.0,
    GrenadeSpeed = 1000.0,
    GrenadeLifetime = 2.0,
    LaserReach = 800.0,
    LaserBounceDelay = 150.0,
    LaserBounceNum = 1000.0,
    LaserBounceCost = 0.0,
    LaserDamage = 5.0,
    PlayerCollision = 1.0,
    PlayerHooking = 1.0,
    JetpackStrength = 400.0,
    ShotgunStrength = 10.0,
    ExplosionStrength = 6.0,
    HammerStrength = 1.0,
    HookDuration = 1.25,
    HammerFireDelay = 125.0,
    GunFireDelay = 125.0,
    ShotgunFireDelay = 500.0,
    GrenadeFireDelay = 500.0,
    LaserFireDelay = 800.0,
    NinjaFireDelay = 800.0,
    HammerHitFireDelay = 320.0,
    GroundElasticityX = 0.0,
    GroundElasticityY = 0.0,
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
