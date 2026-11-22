//! C++: `CAnimState` (`src/game/client/animstate.cpp`).

use super::content::{ANIM_BASE, ANIM_IDLE, ANIMATIONS, Animation, Keyframe};

/// C++: `mix` (math.h) for floats.
fn mix(a: f32, b: f32, amount: f32) -> f32 {
    a + (b - a) * amount
}

/// C++: `AnimSeqEval`. A time outside every keyframe range leaves `frame` unchanged,
/// like C++ (which never happens: callers pass times within [0, 1]).
fn seq_eval(seq: &[Keyframe], time: f32, frame: &mut Keyframe) {
    match seq {
        [] => *frame = Keyframe::default(),
        [only] => *frame = *only,
        _ => {
            if let Some(w) = seq
                .windows(2)
                .find(|w| w[0].time <= time && w[1].time >= time)
            {
                let blend = (time - w[0].time) / (w[1].time - w[0].time);
                *frame = Keyframe {
                    time,
                    x: mix(w[0].x, w[1].x, blend),
                    y: mix(w[0].y, w[1].y, blend),
                    angle: mix(w[0].angle, w[1].angle, blend),
                };
            }
        }
    }
}

/// C++: `AnimAddKeyframe`.
fn add_keyframe(seq: &mut Keyframe, added: &Keyframe, amount: f32) {
    seq.x += added.x * amount;
    seq.y += added.y * amount;
    seq.angle += added.angle * amount;
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AnimState {
    pub body: Keyframe,
    pub back_foot: Keyframe,
    pub front_foot: Keyframe,
    pub attach: Keyframe,
}

impl AnimState {
    /// C++: `CAnimState::Set`.
    pub fn set(&mut self, anim: &Animation, time: f32) {
        seq_eval(anim.body, time, &mut self.body);
        seq_eval(anim.back_foot, time, &mut self.back_foot);
        seq_eval(anim.front_foot, time, &mut self.front_foot);
        seq_eval(anim.attach, time, &mut self.attach);
    }

    /// C++: `CAnimState::Add`. C++ evaluates into an uninitialized temporary; ours
    /// starts at zero (only differs for times outside [0, 1], see `seq_eval`).
    pub fn add(&mut self, anim: &Animation, time: f32, amount: f32) {
        let mut added = AnimState::default();
        added.set(anim, time);
        add_keyframe(&mut self.body, &added.body, amount);
        add_keyframe(&mut self.back_foot, &added.back_foot, amount);
        add_keyframe(&mut self.front_foot, &added.front_foot, amount);
        add_keyframe(&mut self.attach, &added.attach, amount);
    }

    /// C++: `CAnimState::GetIdle`.
    pub fn idle() -> Self {
        let mut s = AnimState::default();
        s.set(&ANIMATIONS[ANIM_BASE], 0.0);
        s.add(&ANIMATIONS[ANIM_IDLE], 0.0, 1.0);
        s
    }
}
