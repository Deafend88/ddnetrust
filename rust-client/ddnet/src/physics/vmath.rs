//! Float vector math with the exact operation order of the C++ helpers.
//! C++: `src/base/vmath.h`, `src/base/math.h`.

use std::ops::{Add, AddAssign, Mul, MulAssign, Neg, Sub, SubAssign};

/// C++: `pi` (math.h).
pub const PI: f32 = std::f32::consts::PI; // same bits as C++ `pi`, see the test below

/// C++: `vec2` (`vector2_base<float>`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

pub const fn vec2(x: f32, y: f32) -> Vec2 {
    Vec2 { x, y }
}

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, o: Vec2) -> Vec2 {
        vec2(self.x + o.x, self.y + o.y)
    }
}

impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, o: Vec2) -> Vec2 {
        vec2(self.x - o.x, self.y - o.y)
    }
}

impl Neg for Vec2 {
    type Output = Vec2;
    fn neg(self) -> Vec2 {
        vec2(-self.x, -self.y)
    }
}

impl Mul<f32> for Vec2 {
    type Output = Vec2;
    fn mul(self, s: f32) -> Vec2 {
        vec2(self.x * s, self.y * s)
    }
}

impl AddAssign for Vec2 {
    fn add_assign(&mut self, o: Vec2) {
        *self = *self + o;
    }
}

impl SubAssign for Vec2 {
    fn sub_assign(&mut self, o: Vec2) {
        *self = *self - o;
    }
}

impl MulAssign<f32> for Vec2 {
    fn mul_assign(&mut self, s: f32) {
        *self = *self * s;
    }
}

pub fn dot(a: Vec2, b: Vec2) -> f32 {
    a.x * b.x + a.y * b.y
}

pub fn length(a: Vec2) -> f32 {
    dot(a, a).sqrt()
}

pub fn distance(a: Vec2, b: Vec2) -> f32 {
    length(a - b)
}

/// C++: `normalize`: multiply by the reciprocal, not divide.
pub fn normalize(v: Vec2) -> Vec2 {
    let divisor = length(v);
    if divisor == 0.0 {
        return vec2(0.0, 0.0);
    }
    let l = 1.0 / divisor;
    vec2(v.x * l, v.y * l)
}

pub fn direction(angle: f32) -> Vec2 {
    vec2(angle.cos(), angle.sin())
}

/// C++: `angle` (vmath.h).
pub fn angle(a: Vec2) -> f32 {
    if a.x == 0.0 && a.y == 0.0 {
        return 0.0;
    }
    if a.x == 0.0 {
        return if a.y < 0.0 { -PI / 2.0 } else { PI / 2.0 };
    }
    let result = (a.y / a.x).atan();
    if a.x < 0.0 { result + PI } else { result }
}

/// C++: `mix` (math.h): `a + (b - a) * amount`.
pub fn mix(a: Vec2, b: Vec2, amount: f32) -> Vec2 {
    a + (b - a) * amount
}

/// C++: `closest_point_on_line` (vmath.h).
pub fn closest_point_on_line(a: Vec2, b: Vec2, target: Vec2) -> Option<Vec2> {
    let ab = b - a;
    let squared_magnitude_ab = dot(ab, ab);
    if squared_magnitude_ab > 0.0 {
        let t = dot(target - a, ab) / squared_magnitude_ab;
        Some(a + ab * t.clamp(0.0, 1.0))
    } else {
        None
    }
}

/// C++ `(int)f` on x86-64 (cvttss2si): truncates, `i32::MIN` for NaN and out-of-range
/// values, where Rust's `as` would saturate. Also C++ `round_truncate` (math.h).
pub fn trunc_i32(f: f32) -> i32 {
    if (-2_147_483_648.0..2_147_483_648.0).contains(&f) {
        f as i32
    } else {
        i32::MIN
    }
}

/// C++: `round_to_int` (math.h).
pub fn round_to_int(f: f32) -> i32 {
    if f > 0.0 {
        trunc_i32(f + 0.5)
    } else {
        trunc_i32(f - 0.5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x86_casts() {
        assert_eq!(trunc_i32(f32::NAN), i32::MIN);
        assert_eq!(trunc_i32(3e9), i32::MIN);
        assert_eq!(trunc_i32(-3e9), i32::MIN);
        assert_eq!(trunc_i32(-2.7), -2);
        assert_eq!(round_to_int(-2.5), -3);
        assert_eq!(round_to_int(2.5), 3);
        assert_eq!(PI.to_bits(), 0x4049_0FDB);
    }
}
