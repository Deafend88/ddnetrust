//! The camera: zoom levels and smoothing, the dyncam offset and the center. C++:
//! `CCamera` (game/client/components/camera.cpp) for a playing (not spectating)
//! player; the spectator camera (free view, auto-sync, smoothing between targets) is
//! not ported yet.

use ddnet::physics::vmath::{Vec2, length, normalize_pre_length, vec2};

use crate::config::{Config, arg_float};
use crate::controls::{Controls, MouseInputType};

/// C++: `CCubicBezier` (base/bezier.cpp).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CubicBezier {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
}

impl CubicBezier {
    pub fn with(start: f32, start_derivative: f32, end_derivative: f32, end: f32) -> Self {
        Self {
            a: start,
            b: start + start_derivative / 3.0,
            c: end - end_derivative / 3.0,
            d: end,
        }
    }

    pub fn evaluate(&self, t: f32) -> f32 {
        let Self { a, b, c, d } = *self;
        (1.0 - t) * (1.0 - t) * (1.0 - t) * a
            + 3.0 * (1.0 - t) * (1.0 - t) * t * b
            + 3.0 * (1.0 - t) * t * t * c
            + t * t * t * d
    }

    pub fn derivative(&self, t: f32) -> f32 {
        let Self { a, b, c, d } = *self;
        3.0 * (1.0 - t) * (1.0 - t) * (b - a)
            + 6.0 * (1.0 - t) * t * (c - b)
            + 3.0 * t * t * (d - c)
    }
}

/// C++ `std::clamp`, `std::max`, `std::min` (NaN behaviour included).
fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if hi < v {
        hi
    } else {
        v
    }
}
fn max(a: f32, b: f32) -> f32 {
    if a < b { b } else { a }
}
fn min(a: f32, b: f32) -> f32 {
    if b < a { b } else { a }
}

/// C++: `CCamera::ZOOM_STEP`, `ZoomStepsToValue`.
const ZOOM_STEP: f32 = 0.866025;
pub fn zoom_steps_to_value(steps: f32) -> f32 {
    ZOOM_STEP.powf(steps)
}

/// The per-frame values `CCamera` reads from the client.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    /// C++ `Client()->LocalTime()`: seconds since the client started.
    pub local_time: f32,
    /// C++ `Client()->RenderFrameTime()`: seconds since the previous frame.
    pub frame_time: f32,
    /// C++ `ZoomAllowed()`: `m_GameInfo.m_AllowZoom` while playing.
    pub allow_zoom: bool,
}

#[derive(Clone, Debug)]
pub struct Camera {
    pub center: Vec2,
    zoom_set: bool,
    pub zooming: bool,
    pub zoom: f32,
    /// C++ leaves it unset until the first zoom; it is only read while zooming.
    pub zoom_smoothing_target: f32,
    pub user_zoom_target: f32,
    zoom_smoothing: CubicBezier,
    zoom_smoothing_start: f32,
    zoom_smoothing_end: f32,
    /// C++ `m_CamType == CAMTYPE_PLAYER` (else `CAMTYPE_UNDEFINED`).
    player_cam: bool,
    pub dyncam_target_offset: Vec2,
    pub dyncam_current_offset: Vec2,
    pub dyncam_speed_bias: f32,
    last_target_pos: Vec2,
}

impl Default for Camera {
    /// C++: `CCamera::CCamera`.
    fn default() -> Self {
        Self {
            center: vec2(0.0, 0.0),
            zoom_set: false,
            zooming: false,
            zoom: 1.0,
            zoom_smoothing_target: 0.0,
            user_zoom_target: 0.0,
            zoom_smoothing: CubicBezier::default(),
            zoom_smoothing_start: 0.0,
            zoom_smoothing_end: 0.0,
            player_cam: false,
            dyncam_target_offset: vec2(0.0, 0.0),
            dyncam_current_offset: vec2(0.0, 0.0),
            dyncam_speed_bias: 0.5,
            last_target_pos: vec2(0.0, 0.0),
        }
    }
}

/// C++: `CCamera::MinZoomLevel`.
const MIN_ZOOM: f32 = 0.01;

/// C++: `CCamera::MaxZoomLevel` with tile buffering (our renderer draws tiles like the
/// buffered backends).
fn max_zoom(config: &Config) -> f32 {
    if config.cl_limit_max_zoom_level != 0 {
        240.0
    } else {
        2000.0
    }
}

/// C++: `CCamera::Deadzone`, `FollowFactor`.
pub fn deadzone(config: &Config) -> i32 {
    if config.cl_dyncam != 0 {
        config.cl_dyncam_deadzone
    } else {
        config.cl_mouse_deadzone
    }
}
pub fn follow_factor(config: &Config) -> i32 {
    if config.cl_dyncam != 0 {
        config.cl_dyncam_follow_factor
    } else {
        config.cl_mouse_followfactor
    }
}

impl Camera {
    /// C++: `CCamera::OnReset`.
    pub fn on_reset(&mut self, config: &Config) {
        self.zoom = zoom_steps_to_value((config.cl_default_zoom - 10) as f32);
        self.zooming = false;
        self.user_zoom_target = zoom_steps_to_value((config.cl_default_zoom - 10) as f32);
    }

    fn zoom_progress(&self, t: f32) -> f32 {
        (t - self.zoom_smoothing_start) / (self.zoom_smoothing_end - self.zoom_smoothing_start)
    }

    /// C++: `CCamera::ChangeZoom` (camera.cpp:81).
    fn change_zoom(
        &mut self,
        target: f32,
        smoothness: i32,
        is_user: bool,
        now: f32,
        config: &Config,
    ) {
        if target > max_zoom(config) || target < MIN_ZOOM {
            return;
        }
        let (mut current, mut derivative) = (self.zoom, 0.0);
        if self.zooming {
            let progress = self.zoom_progress(now);
            current = self.zoom_smoothing.evaluate(progress);
            derivative = self.zoom_smoothing.derivative(progress);
        }
        self.zoom_smoothing_target = target;
        self.zoom_smoothing = CubicBezier::with(current, derivative, 0.0, target);
        self.zoom_smoothing_start = now;
        self.zoom_smoothing_end = now + smoothness as f32 / 1000.0;
        if is_user {
            self.user_zoom_target = target;
        }
        self.zooming = true;
    }

    /// C++: `CCamera::ScaleZoom`.
    fn scale_zoom(&mut self, factor: f32, now: f32, config: &Config) {
        let current = if self.zooming {
            self.zoom_smoothing_target
        } else {
            self.zoom
        };
        self.change_zoom(
            current * factor,
            config.cl_smooth_zoom_time,
            true,
            now,
            config,
        );
    }

    /// C++: `ConZoomPlus` / `ConZoomMinus` (`zoom+` / `zoom-` with an optional amount).
    pub fn zoom_step(&mut self, args: &[String], sign: f32, frame: &Frame, config: &Config) {
        if !frame.allow_zoom {
            return;
        }
        let mut amount = if args.is_empty() {
            1.0
        } else {
            arg_float(args, 0)
        };
        amount *= sign;
        self.scale_zoom(zoom_steps_to_value(amount), frame.local_time, config);
    }

    /// C++: `ConZoom` (`zoom` with an optional level; none resets to `cl_default_zoom`).
    pub fn zoom_to(&mut self, args: &[String], frame: &Frame, config: &Config) {
        if !frame.allow_zoom {
            return;
        }
        let level = if args.is_empty() {
            config.cl_default_zoom as f32
        } else {
            arg_float(args, 0)
        };
        let target = zoom_steps_to_value(level - 10.0);
        self.change_zoom(
            target,
            config.cl_smooth_zoom_time,
            true,
            frame.local_time,
            config,
        );
    }

    /// C++: `CCamera::UpdateCamera` (camera.cpp:114), not spectating: zoom smoothing and
    /// the dyncam offset towards the mouse.
    pub fn update(&mut self, mouse_pos: Vec2, frame: &Frame, config: &Config) {
        let current_zoom = if self.zooming {
            self.zoom_smoothing_target
        } else {
            self.zoom
        };
        if current_zoom != self.user_zoom_target {
            let target = self.user_zoom_target;
            self.change_zoom(
                target,
                config.cl_smooth_zoom_time,
                false,
                frame.local_time,
                config,
            );
        }
        if self.zooming {
            let time = frame.local_time;
            if time >= self.zoom_smoothing_end {
                self.zoom = self.zoom_smoothing_target;
                self.zooming = false;
            } else {
                let old = self.zoom;
                self.zoom = self.zoom_smoothing.evaluate(self.zoom_progress(time));
                let target = self.zoom_smoothing_target;
                if (old < target && self.zoom > target) || (old > target && self.zoom < target) {
                    self.zoom = target;
                    self.zooming = false;
                }
            }
            self.zoom = clamp(self.zoom, MIN_ZOOM, max_zoom(config));
        }
        if !frame.allow_zoom {
            self.zoom_set = false;
            self.zoom = 1.0;
            self.zooming = false;
        } else if !self.zoom_set && config.cl_default_zoom != 10 {
            self.zoom_set = true;
            self.on_reset(config);
        }

        let target_pos = mouse_pos;
        let smoothness = config.cl_dyncam_smoothness;
        let stabilizing = config.cl_dyncam_stabilizing;
        let is_dyncam = config.cl_dyncam != 0;
        let delta_time = frame.frame_time;
        if smoothness > 0 {
            let camera_speed = (1.0 - (smoothness as f32 / 100.0)) * 9.5 + 0.5;
            let stabilizing_factor = 1.0 + stabilizing as f32 / 100.0;
            self.dyncam_speed_bias += camera_speed * delta_time;
            if is_dyncam {
                self.dyncam_speed_bias -=
                    length(target_pos - self.last_target_pos) * stabilizing_factor.log10() * 0.02;
                self.dyncam_speed_bias = clamp(self.dyncam_speed_bias, 0.5, camera_speed);
            } else {
                self.dyncam_speed_bias = max(5.0, camera_speed);
            }
        }
        self.dyncam_target_offset = vec2(0.0, 0.0);
        let l = length(target_pos);
        if l > 0.0001 {
            let offset_amount =
                max(l - deadzone(config) as f32, 0.0) * (follow_factor(config) as f32 / 100.0);
            self.dyncam_target_offset = normalize_pre_length(target_pos, l) * offset_amount;
        }
        self.last_target_pos = target_pos;
        let mut current = self.dyncam_current_offset;
        if smoothness > 0 {
            current += (self.dyncam_target_offset - current)
                * min(delta_time * self.dyncam_speed_bias, 1.0);
        } else {
            current = self.dyncam_target_offset;
        }
        self.dyncam_current_offset = current;
    }

    /// C++: `CCamera::OnRender` (camera.cpp:289), not spectating: the center is the
    /// local tee (C++ `m_LocalCharacterPos`) plus the dyncam offset. The first frame
    /// resets the mouse like C++ does when switching to the player camera.
    pub fn on_render(&mut self, controls: &mut Controls, local_pos: Vec2, config: &Config) {
        if !self.player_cam {
            controls.mouse_pos = vec2(0.0, 0.0); // C++ m_aLastPos: only set when spectating
            controls.input_type = MouseInputType::Automated;
            controls.clamp_mouse_pos(config);
            self.player_cam = true;
        }
        self.center = local_pos + self.dyncam_current_offset;
    }
}
