//! Animation helpers: a critically damped spring (no overshoot, settles in a set time)
//! and an easing curve.

/// A critically damped spring towards a target. Uses the exact solution, so the result
/// does not depend on how a time span is split into frames.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spring {
    pub value: f32,
    pub velocity: f32,
}

impl Spring {
    pub fn new(value: f32) -> Self {
        Self {
            value,
            velocity: 0.0,
        }
    }

    /// Advances `dt` seconds towards `target`; within 1% of a step after `settle_time`.
    pub fn update(&mut self, target: f32, settle_time: f32, dt: f32) -> f32 {
        // critically damped: x(t) = (d + (v + w d) t) e^(-w t); 1% after ~6.64 / w
        let w = 6.64 / settle_time.max(1e-3);
        let d = self.value - target;
        let c = self.velocity + w * d;
        let e = (-w * dt).exp();
        self.value = target + (d + c * dt) * e;
        self.velocity = (self.velocity - w * c * dt) * e;
        self.value
    }

    /// Like `update`, but underdamped (`zeta` in (0, 1)): a small overshoot, then
    /// settled within 1% of a step after `settle_time`. Exact solution too.
    pub fn update_damped(&mut self, target: f32, settle_time: f32, zeta: f32, dt: f32) -> f32 {
        // x(t) = e^(-z w t) (d cos(wd t) + (v + z w d) / wd sin(wd t)); from rest the
        // amplitude is d / sqrt(1 - z^2), so it is under 1% after (4.6 + ln that) / (z w)
        let k = (1.0 - zeta * zeta).sqrt();
        let w = (4.6 - k.ln()) / (zeta * settle_time.max(1e-3));
        let wd = w * k;
        let (d, v) = (self.value - target, self.velocity);
        let e = (-zeta * w * dt).exp();
        let (s, c) = (wd * dt).sin_cos();
        self.value = target + e * (d * c + (v + zeta * w * d) / wd * s);
        self.velocity = e * (v * c - (w * w * d + zeta * w * v) / wd * s);
        self.value
    }
}

/// Ease-out cubic on `t` in [0, 1].
pub fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_settles_without_overshoot() {
        let mut s = Spring::new(0.0);
        let mut last = 0.0;
        let mut t = 0.0f32;
        while t < 1.0 {
            let v = s.update(1.0, 0.5, 1.0 / 240.0);
            assert!(v >= last - 1e-6 && v <= 1.0 + 1e-6, "{v} after {last}");
            last = v;
            t += 1.0 / 240.0;
            if (t - 0.5).abs() < 1.0 / 480.0 {
                assert!((1.0 - v).abs() < 0.01, "{v} at the settle time");
            }
        }
        assert!((1.0 - s.value).abs() < 1e-4);
    }

    #[test]
    fn spring_is_frame_rate_independent() {
        let (mut a, mut b) = (Spring::new(-3.0), Spring::new(-3.0));
        a.update(2.0, 0.3, 0.1);
        for _ in 0..10 {
            b.update(2.0, 0.3, 0.01);
        }
        assert!((a.value - b.value).abs() < 1e-4 && (a.velocity - b.velocity).abs() < 1e-3);
    }

    #[test]
    fn damped_spring_overshoots_a_little_and_settles() {
        let (mut s, mut peak, mut t) = (Spring::new(0.0), 0.0f32, 0.0f32);
        while t < 0.45 {
            peak = peak.max(s.update_damped(1.0, 0.45, 0.75, 1.0 / 240.0));
            t += 1.0 / 240.0;
        }
        assert!(peak > 1.01 && peak < 1.05, "peak {peak}");
        assert!(
            (1.0 - s.value).abs() < 0.01,
            "{} at the settle time",
            s.value
        );
        let (mut a, mut b) = (Spring::new(-3.0), Spring::new(-3.0));
        a.update_damped(2.0, 0.3, 0.75, 0.1);
        for _ in 0..10 {
            b.update_damped(2.0, 0.3, 0.75, 0.01);
        }
        assert!((a.value - b.value).abs() < 1e-4 && (a.velocity - b.velocity).abs() < 1e-2);
    }

    #[test]
    fn ease_out_ends() {
        assert_eq!((ease_out(0.0), ease_out(1.0)), (0.0, 1.0));
        assert!(ease_out(0.5) > 0.5 && ease_out(0.5) < 1.0);
    }
}
