//! Mouse and game input. C++: `CControls` (game/client/components/controls.cpp) for one
//! playing player (no dummy, no spectating, no chat or menus yet), the console
//! commands binds run (`+left`, `+fire`, `zoom+`, ...), and the winit to C++ key id
//! mapping (C++ key ids are SDL scancodes).

use ddnet::physics::vmath::{Vec2, length, normalize_pre_length, trunc_i32, vec2};
use ddnet::protocol::generated as g;
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

use crate::camera::{Camera, Frame};
use crate::config::{Config, find_key_by_name, parse_args, split_line};

/// C++: `CControls::EMouseInputType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseInputType {
    Absolute,
    Relative,
    Automated,
}

#[derive(Clone, Debug)]
pub struct Controls {
    /// C++ `m_aMousePos[0]`: the aim, relative to the tee.
    pub mouse_pos: Vec2,
    pub input_type: MouseInputType,
    /// C++ `m_aInputData[0]` (the input fields the commands set).
    pub input: g::ObjPlayerInput,
    /// C++ `m_aInputDirectionLeft[0]`, `m_aInputDirectionRight[0]`.
    pub left: i32,
    pub right: i32,
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            mouse_pos: vec2(0.0, 0.0),
            input_type: MouseInputType::Absolute,
            input: g::ObjPlayerInput::default(),
            left: 0,
            right: 0,
        }
    }
}

/// What the input commands need to know about the game.
#[derive(Clone, Copy, Debug, Default)]
pub struct GameState {
    /// C++ `m_GameInfo.m_BugDDRaceInput && m_Snap.m_SpecInfo.m_Active`: movement keys
    /// are ignored.
    pub frozen_input: bool,
}

impl Controls {
    /// C++: `CControls::GetMinMouseDistance`.
    pub fn min_mouse_distance(config: &Config) -> f32 {
        (if config.cl_dyncam != 0 {
            config.cl_dyncam_min_distance
        } else {
            config.cl_mouse_min_distance
        }) as f32
    }

    /// C++: `CControls::GetMaxMouseDistance` (controls.cpp:519).
    pub fn max_mouse_distance(config: &Config) -> f32 {
        let dyncam = config.cl_dyncam != 0;
        let camera_max_distance = 200.0f32;
        let follow_factor = (if dyncam {
            config.cl_dyncam_follow_factor
        } else {
            config.cl_mouse_followfactor
        }) as f32
            / 100.0;
        let dead_zone = (if dyncam {
            config.cl_dyncam_deadzone
        } else {
            config.cl_mouse_deadzone
        }) as f32;
        let max_distance = (if dyncam {
            config.cl_dyncam_max_distance
        } else {
            config.cl_mouse_max_distance
        }) as f32;
        let d = if follow_factor != 0.0 {
            camera_max_distance / follow_factor + dead_zone
        } else {
            max_distance
        };
        // C++ std::min(d, max_distance)
        if max_distance < d { max_distance } else { d }
    }

    /// C++: `CControls::ClampMousePos` (controls.cpp:487), not spectating.
    pub fn clamp_mouse_pos(&mut self, config: &Config) {
        let mouse_min = Self::min_mouse_distance(config);
        let mouse_max = Self::max_mouse_distance(config);
        let mut distance = length(self.mouse_pos);
        if distance < 0.001 {
            self.mouse_pos = vec2(0.001, 0.0);
            distance = 0.001;
        }
        if distance < mouse_min {
            self.mouse_pos = normalize_pre_length(self.mouse_pos, distance) * mouse_min;
        }
        distance = length(self.mouse_pos);
        if distance > mouse_max {
            self.mouse_pos = normalize_pre_length(self.mouse_pos, distance) * mouse_max;
        }
    }

    /// C++: `CControls::OnCursorMove` (controls.cpp:442) for relative mouse motion,
    /// not spectating. Ignored while the world is paused.
    pub fn on_cursor_move(&mut self, x: f32, y: f32, config: &Config, world_paused: bool) {
        if world_paused {
            return;
        }
        let factor = if config.cl_dyncam != 0 && config.cl_dyncam_mousesens != 0 {
            config.cl_dyncam_mousesens as f32 / 100.0
        } else {
            config.inp_mousesens as f32 / 100.0
        };
        self.mouse_pos += vec2(x, y) * factor;
        self.input_type = MouseInputType::Relative;
        self.clamp_mouse_pos(config);
    }

    /// C++: `CConsole::ExecuteLineStroked` for a bind's command line, with the commands
    /// of `CControls::OnConsoleInit` and the zoom commands of `CCamera`; others are
    /// ignored. `+` commands run on press and release, others on press only.
    pub fn execute(
        &mut self,
        stroke: bool,
        line: &str,
        game: GameState,
        camera: &mut Camera,
        frame: &Frame,
        config: &Config,
    ) {
        for (command, args) in split_line(line) {
            let name = command.to_ascii_lowercase();
            let is_stroke = name.starts_with('+');
            if !stroke && !is_stroke {
                continue;
            }
            let params = match name.as_str() {
                "zoom+" | "zoom-" => "?f[amount]",
                "zoom" => "?f",
                _ => "",
            };
            let mut a = if is_stroke {
                vec![(stroke as i32).to_string()]
            } else {
                Vec::new()
            };
            if !parse_args(&args, params, &mut a) {
                continue;
            }
            let down = stroke as i32;
            match name.as_str() {
                // C++: ConKeyInputState
                "+left" | "+right" | "+jump" | "+hook" if !game.frozen_input => {
                    *match name.as_str() {
                        "+left" => &mut self.left,
                        "+right" => &mut self.right,
                        "+jump" => &mut self.input.jump,
                        _ => &mut self.input.hook,
                    } = down;
                }
                // C++: ConKeyInputCounter
                "+fire" if !game.frozen_input => counter(&mut self.input.fire, down),
                // C++: ConKeyInputNextPrevWeapon (the counter bails out, the reset not)
                "+nextweapon" | "+prevweapon" => {
                    if !game.frozen_input {
                        let v = if name == "+nextweapon" {
                            &mut self.input.next_weapon
                        } else {
                            &mut self.input.prev_weapon
                        };
                        counter(v, down);
                    }
                    self.input.wanted_weapon = 0;
                }
                // C++: ConKeyInputSet
                "+weapon1" | "+weapon2" | "+weapon3" | "+weapon4" | "+weapon5" if stroke => {
                    self.input.wanted_weapon = (name.as_bytes()[7] - b'0') as i32;
                }
                "zoom+" => camera.zoom_step(&a, 1.0, frame, config),
                "zoom-" => camera.zoom_step(&a, -1.0, frame, config),
                "zoom" => camera.zoom_to(&a, frame, config),
                _ => {}
            }
        }
    }

    /// C++: `CControls::SnapInput` (controls.cpp:241) while playing: the input to send,
    /// with the mouse position as the target.
    pub fn snap_input(&mut self) -> g::ObjPlayerInput {
        let mut input = self.input;
        input.player_flags = g::PLAYERFLAG_PLAYING
            | match self.input_type {
                MouseInputType::Automated => g::PLAYERFLAG_INPUT_ABSOLUTE,
                MouseInputType::Absolute => {
                    g::PLAYERFLAG_INPUT_ABSOLUTE | g::PLAYERFLAG_INPUT_MANUAL
                }
                MouseInputType::Relative => g::PLAYERFLAG_INPUT_MANUAL,
            };
        input.target_x = trunc_i32(self.mouse_pos.x);
        input.target_y = trunc_i32(self.mouse_pos.y);
        if input.target_x == 0 && input.target_y == 0 {
            input.target_x = 1;
            self.mouse_pos.x = 1.0;
        }
        input.direction = match (self.left != 0, self.right != 0) {
            (true, false) => -1,
            (false, true) => 1,
            _ => 0,
        };
        input
    }
}

/// C++: `ConKeyInputCounter`: counts presses and releases, kept in `INPUT_STATE_MASK`.
fn counter(v: &mut i32, stroke: i32) {
    if *v & 1 != stroke {
        *v += 1;
    }
    *v &= ddnet::physics::core::INPUT_STATE_MASK;
}

/// The C++ key id of a winit key (C++ key ids are SDL scancodes; looked up by their
/// C++ key names). `None` for keys C++ has no id for.
pub fn key_id(code: KeyCode) -> Option<usize> {
    use KeyCode::*;
    let name = match code {
        KeyA => "a",
        KeyB => "b",
        KeyC => "c",
        KeyD => "d",
        KeyE => "e",
        KeyF => "f",
        KeyG => "g",
        KeyH => "h",
        KeyI => "i",
        KeyJ => "j",
        KeyK => "k",
        KeyL => "l",
        KeyM => "m",
        KeyN => "n",
        KeyO => "o",
        KeyP => "p",
        KeyQ => "q",
        KeyR => "r",
        KeyS => "s",
        KeyT => "t",
        KeyU => "u",
        KeyV => "v",
        KeyW => "w",
        KeyX => "x",
        KeyY => "y",
        KeyZ => "z",
        Digit0 => "0",
        Digit1 => "1",
        Digit2 => "2",
        Digit3 => "3",
        Digit4 => "4",
        Digit5 => "5",
        Digit6 => "6",
        Digit7 => "7",
        Digit8 => "8",
        Digit9 => "9",
        Enter => "return",
        Escape => "escape",
        Backspace => "backspace",
        Tab => "tab",
        Space => "space",
        Minus => "minus",
        Equal => "equals",
        BracketLeft => "leftbracket",
        BracketRight => "rightbracket",
        Backslash => "backslash",
        Semicolon => "semicolon",
        Quote => "apostrophe",
        Backquote => "grave",
        Comma => "comma",
        Period => "period",
        Slash => "slash",
        CapsLock => "capslock",
        F1 => "f1",
        F2 => "f2",
        F3 => "f3",
        F4 => "f4",
        F5 => "f5",
        F6 => "f6",
        F7 => "f7",
        F8 => "f8",
        F9 => "f9",
        F10 => "f10",
        F11 => "f11",
        F12 => "f12",
        F13 => "f13",
        F14 => "f14",
        F15 => "f15",
        F16 => "f16",
        F17 => "f17",
        F18 => "f18",
        F19 => "f19",
        F20 => "f20",
        F21 => "f21",
        F22 => "f22",
        F23 => "f23",
        F24 => "f24",
        PrintScreen => "printscreen",
        ScrollLock => "scrolllock",
        Pause => "pause",
        Insert => "insert",
        Home => "home",
        PageUp => "pageup",
        Delete => "delete",
        End => "end",
        PageDown => "pagedown",
        ArrowRight => "right",
        ArrowLeft => "left",
        ArrowDown => "down",
        ArrowUp => "up",
        NumLock => "numlockclear",
        NumpadDivide => "kp_divide",
        NumpadMultiply => "kp_multiply",
        NumpadSubtract => "kp_minus",
        NumpadAdd => "kp_plus",
        NumpadEnter => "kp_enter",
        Numpad1 => "kp_1",
        Numpad2 => "kp_2",
        Numpad3 => "kp_3",
        Numpad4 => "kp_4",
        Numpad5 => "kp_5",
        Numpad6 => "kp_6",
        Numpad7 => "kp_7",
        Numpad8 => "kp_8",
        Numpad9 => "kp_9",
        Numpad0 => "kp_0",
        NumpadDecimal => "kp_period",
        NumpadEqual => "kp_equals",
        IntlBackslash => "nonusbackslash",
        ContextMenu => "application",
        ControlLeft => "lctrl",
        ShiftLeft => "lshift",
        AltLeft => "lalt",
        SuperLeft => "lgui",
        ControlRight => "rctrl",
        ShiftRight => "rshift",
        AltRight => "ralt",
        SuperRight => "rgui",
        _ => return None,
    };
    Some(find_key_by_name(name)).filter(|&k| k != 0)
}

/// C++: `TranslateMouseButtonEventKey` (input.cpp): left, right, middle, X1, X2.
pub fn mouse_key_id(button: MouseButton) -> Option<usize> {
    let n = match button {
        MouseButton::Left => 1,
        MouseButton::Right => 2,
        MouseButton::Middle => 3,
        MouseButton::Back => 4,
        MouseButton::Forward => 5,
        MouseButton::Other(_) => return None,
    };
    Some(crate::config::KEY_MOUSE_1 + n - 1)
}
