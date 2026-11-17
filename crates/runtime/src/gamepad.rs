//! Controllers, through the game's own joystick support: WINMM `joyGetPos`.
//!
//! The original polls `joyGetPos(0)` every frame in its input poll (0x411f70) and
//! reads it as a digital joystick: X below 14000 (of 0–65535) is left and above
//! 50000 right, Y below 14000 adds to "up", and any button fires. If the first call
//! fails it sets 0x98d500 and never asks again. So the runtime reports joystick 0
//! as always connected, centred while no controller is attached, which lets a
//! controller be plugged in at any time; the game code is unchanged.
//!
//! Every SDL gamepad drives that one joystick: the left stick or D-pad is the
//! position, and South/East/West/North (A/B/X/Y on an Xbox layout) are buttons 1–4,
//! with the right trigger also button 1. Scripted runs (`--input`) and headless
//! runs keep the joystick unplugged, as the reference captures of the original had it.

use crate::Runtime;
use sdl3_sys::gamepad::*;
use sdl3_sys::joystick::SDL_JoystickID;

/// JOYERR_UNPLUGGED.
pub const JOYERR_UNPLUGGED: u32 = 167;

/// The open SDL gamepads.
#[derive(Default)]
pub struct Gamepads {
    open: Vec<(SDL_JoystickID, *mut SDL_Gamepad)>,
}

/// JOYINFO: wXpos, wYpos, wZpos (0–65535, centre 32768) and wButtons (JOY_BUTTON1..4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JoyInfo {
    pub x: u32,
    pub y: u32,
    pub z: u32,
    pub buttons: u32,
}

impl JoyInfo {
    pub const CENTRED: JoyInfo = JoyInfo { x: 32768, y: 32768, z: 32768, buttons: 0 };
}

impl Gamepads {
    /// A gamepad was connected (SDL_EVENT_GAMEPAD_ADDED, also sent for ones present at start).
    pub fn added(&mut self, id: SDL_JoystickID) {
        // SAFETY: SDL is initialised with the gamepad subsystem; id comes from SDL.
        let pad = unsafe { SDL_OpenGamepad(id) };
        if pad.is_null() {
            return;
        }
        if crate::trace_enabled("gamepad") {
            // SAFETY: valid gamepad; SDL returns a NUL-terminated name or null.
            let name = unsafe { SDL_GetGamepadName(pad) };
            let name = if name.is_null() { "?".into() } else { unsafe { std::ffi::CStr::from_ptr(name) }.to_string_lossy() };
            eprintln!("[gamepad] connected: {name}");
        }
        self.open.push((id, pad));
    }

    /// A gamepad was disconnected (SDL_EVENT_GAMEPAD_REMOVED).
    pub fn removed(&mut self, id: SDL_JoystickID) {
        self.open.retain(|&(i, pad)| {
            if i != id {
                return true;
            }
            // SAFETY: opened by `added`, closed once.
            unsafe { SDL_CloseGamepad(pad) };
            if crate::trace_enabled("gamepad") {
                eprintln!("[gamepad] disconnected");
            }
            false
        });
    }

    /// Joystick 0's state: the most deflected stick or D-pad across all gamepads,
    /// and every gamepad's buttons.
    pub fn joy_info(&self) -> JoyInfo {
        let mut info = JoyInfo::CENTRED;
        let mut deflection = (0u32, 0u32);
        for &(_, pad) in &self.open {
            // SAFETY: valid open gamepad.
            let (axis, button) = unsafe {
                (|a| SDL_GetGamepadAxis(pad, a) as i32, |b| SDL_GetGamepadButton(pad, b))
            };
            let mut x = axis(SDL_GAMEPAD_AXIS_LEFTX) + 32768;
            let mut y = axis(SDL_GAMEPAD_AXIS_LEFTY) + 32768;
            if button(SDL_GAMEPAD_BUTTON_DPAD_LEFT) { x = 0 }
            if button(SDL_GAMEPAD_BUTTON_DPAD_RIGHT) { x = 65535 }
            if button(SDL_GAMEPAD_BUTTON_DPAD_UP) { y = 0 }
            if button(SDL_GAMEPAD_BUTTON_DPAD_DOWN) { y = 65535 }
            let (dx, dy) = (x.abs_diff(32768), y.abs_diff(32768));
            if dx > deflection.0 { deflection.0 = dx; info.x = x as u32 }
            if dy > deflection.1 { deflection.1 = dy; info.y = y as u32 }
            for (bit, b) in [(1, SDL_GAMEPAD_BUTTON_SOUTH), (2, SDL_GAMEPAD_BUTTON_EAST), (4, SDL_GAMEPAD_BUTTON_WEST), (8, SDL_GAMEPAD_BUTTON_NORTH)] {
                if button(b) { info.buttons |= bit }
            }
            if axis(SDL_GAMEPAD_AXIS_RIGHT_TRIGGER) > 16384 { info.buttons |= 1 }
        }
        info
    }
}

/// `joyGetPos(id, JOYINFO*)`: Some(state) for joystick 0 when controllers are in use,
/// None (JOYERR_UNPLUGGED) otherwise.
pub fn joy_get_pos(rt: &Runtime, id: u32) -> Option<JoyInfo> {
    let u = &rt.user;
    (id == 0 && !u.headless && !u.ignore_host_input && u.gamepads_ready).then(|| u.gamepads.joy_info())
}
