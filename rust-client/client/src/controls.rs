//! Mouse and game input. C++: `CControls` (game/client/components/controls.cpp) for the
//! main tee and the dummy (not spectating), the console commands binds run (`+left`,
//! `+fire`, `zoom+`, ...), and the winit to C++ key id mapping (C++ key ids are SDL
//! scancodes).

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

/// What the local player is doing, for C++ `SnapInput`'s player flags: chat first, then
/// a menu (controls.cpp:244).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerState {
    Playing,
    Chatting,
    InMenu,
}

/// One tee's part of `CControls` (C++ `m_aMousePos[i]`, `m_aInputData[i]`, ...).
#[derive(Clone, Debug)]
pub struct TeeControls {
    /// C++ `m_aMousePos`: the aim, relative to the tee.
    pub mouse_pos: Vec2,
    pub input_type: MouseInputType,
    /// C++ `m_aInputData` (the input fields the commands set).
    pub input: g::ObjPlayerInput,
    /// C++ `m_aInputDirectionLeft`, `m_aInputDirectionRight`.
    pub left: i32,
    pub right: i32,
    /// C++ `m_aLastData`: the input of the last SnapInput.
    pub last: g::ObjPlayerInput,
}

impl Default for TeeControls {
    fn default() -> Self {
        Self {
            mouse_pos: vec2(0.0, 0.0),
            input_type: MouseInputType::Absolute,
            input: g::ObjPlayerInput::default(),
            left: 0,
            right: 0,
            last: g::ObjPlayerInput::default(),
        }
    }
}

impl TeeControls {
    /// C++: `CControls::ResetInput`: release movement, jump and fire (from the last
    /// snapped input).
    pub fn reset_input(&mut self) {
        let l = &mut self.last;
        l.direction = 0;
        if l.fire & 1 != 0 {
            l.fire += 1;
        }
        l.fire &= ddnet::physics::core::INPUT_STATE_MASK;
        l.jump = 0;
        self.input = self.last;
        (self.left, self.right) = (0, 0);
    }
}

/// `CControls`: the active tee's state (C++ index `cl_dummy`, reached through `Deref`)
/// and the other tee's (index `!cl_dummy`); `on_dummy_swap` exchanges them.
#[derive(Clone, Debug, Default)]
pub struct Controls {
    active: TeeControls,
    pub other: TeeControls,
}

impl std::ops::Deref for Controls {
    type Target = TeeControls;
    fn deref(&self) -> &TeeControls {
        &self.active
    }
}

impl std::ops::DerefMut for Controls {
    fn deref_mut(&mut self) -> &mut TeeControls {
        &mut self.active
    }
}

/// The dummy settings `SnapInput` reads (C++ `cl_dummy_copy_moves`, `cl_dummy_control`,
/// `cl_dummy_jump`, `cl_dummy_fire`, `cl_dummy_hook`) and whether the local player
/// follows someone else (C++ `m_Snap.m_SpecInfo.m_Active && m_SpectatorId >= 0`).
#[derive(Clone, Copy, Debug, Default)]
pub struct DummyControl {
    pub copy_moves: bool,
    pub control: bool,
    pub jump: i32,
    pub fire: i32,
    pub hook: i32,
    pub spectating_other: bool,
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

    /// C++: `CControls::SnapInput` (controls.cpp:241): the input to send. Playing, the
    /// mouse position becomes the target and the dummy settings feed `dummy` (C++
    /// `m_DummyInput`) and the other tee's input; chatting or in a menu the input is
    /// reset (unless the server has the DDRace input bug) and only the target follows
    /// the mouse. `scoreboard`: C++ `m_Scoreboard.IsActive()`.
    pub fn snap_input(
        &mut self,
        state: PlayerState,
        scoreboard: bool,
        bug_ddrace_input: bool,
        ctl: DummyControl,
        dummy: &mut g::ObjPlayerInput,
    ) -> g::ObjPlayerInput {
        let in_menu = state != PlayerState::Playing;
        let t = &mut self.active;
        t.input.player_flags = match state {
            PlayerState::Chatting => g::PLAYERFLAG_CHATTING,
            PlayerState::InMenu => g::PLAYERFLAG_IN_MENU,
            PlayerState::Playing => g::PLAYERFLAG_PLAYING,
        } | if scoreboard {
            g::PLAYERFLAG_SCOREBOARD
        } else {
            0
        } | match t.input_type {
            MouseInputType::Automated => g::PLAYERFLAG_INPUT_ABSOLUTE,
            MouseInputType::Absolute => g::PLAYERFLAG_INPUT_ABSOLUTE | g::PLAYERFLAG_INPUT_MANUAL,
            MouseInputType::Relative => g::PLAYERFLAG_INPUT_MANUAL,
        };
        t.last.player_flags = t.input.player_flags;
        if in_menu {
            if !bug_ddrace_input {
                t.reset_input();
            }
            // C++ copies the input here too, but the copy at the end is what is sent
            t.input.target_x = trunc_i32(t.mouse_pos.x);
            t.input.target_y = trunc_i32(t.mouse_pos.y);
        } else {
            let i = &mut t.input;
            i.target_x = trunc_i32(t.mouse_pos.x);
            i.target_y = trunc_i32(t.mouse_pos.y);
            if i.target_x == 0 && i.target_y == 0 {
                i.target_x = 1;
                t.mouse_pos.x = 1.0;
            }
            i.direction = match (t.left != 0, t.right != 0) {
                (true, false) => -1,
                (false, true) => 1,
                _ => 0,
            };
            // dummy copy moves (controls.cpp:319)
            if ctl.copy_moves {
                // don't copy any input to dummy when spectating others
                if !ctl.spectating_other {
                    let (i, l) = (&t.input, &t.last);
                    dummy.direction = i.direction;
                    dummy.hook = i.hook;
                    dummy.jump = i.jump;
                    dummy.player_flags = i.player_flags;
                    dummy.target_x = i.target_x;
                    dummy.target_y = i.target_y;
                    dummy.wanted_weapon = i.wanted_weapon;
                    if !ctl.control {
                        dummy.fire = dummy.fire.wrapping_add(i.fire.wrapping_sub(l.fire));
                    }
                    let next = i.next_weapon.wrapping_sub(l.next_weapon);
                    dummy.next_weapon = dummy.next_weapon.wrapping_add(next);
                    let prev = i.prev_weapon.wrapping_sub(l.prev_weapon);
                    dummy.prev_weapon = dummy.prev_weapon.wrapping_add(prev);
                }
                self.other.input = *dummy;
            }
            if ctl.control {
                dummy.jump = ctl.jump;
                if ctl.fire != 0 {
                    dummy.fire = ctl.fire;
                } else if dummy.fire & 1 != 0 {
                    dummy.fire = dummy.fire.wrapping_add(1);
                }
                dummy.hook = ctl.hook;
                self.other.input = *dummy;
            }
        }
        self.active.last = self.active.input;
        self.active.input
    }

    /// C++: `CGameClient::DummyResetInput` (gameclient.cpp:4314), the dummy connected:
    /// the other tee lets go of everything, keeping `dummy`'s fire count released.
    pub fn dummy_reset(&mut self, dummy: &mut g::ObjPlayerInput) {
        if dummy.fire & 1 != 0 {
            dummy.fire += 1;
        }
        let t = &mut self.other;
        t.reset_input();
        t.input.hook = 0;
        t.input.fire = dummy.fire;
        *dummy = t.input;
    }

    /// C++: `CGameClient::OnDummySwap` (gameclient.cpp:513), after `cl_dummy` changed:
    /// the tees' states change places, `cl_dummy_resetonswitch` (1 the dummy, 2 the
    /// player) releases one tee's keys, the tee left behind keeps its input as `dummy`
    /// (C++ `m_DummyInput`), and the newly active one gets that input's fire count.
    pub fn on_dummy_swap(&mut self, reset_on_switch: i32, dummy: &mut g::ObjPlayerInput) {
        std::mem::swap(&mut self.active, &mut self.other);
        if reset_on_switch != 0 {
            let t = if reset_on_switch == 2 {
                &mut self.active
            } else {
                &mut self.other
            };
            t.reset_input();
            t.input.hook = 0;
        }
        let prev_dummy_fire = dummy.fire;
        *dummy = self.other.input;
        self.active.input.fire = prev_dummy_fire;
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
