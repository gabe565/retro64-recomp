//! Scripted input for automated play and regression runs.
//!
//! A script is a comma-separated list of `FRAME:EVENT` items, delivered after
//! the given `Flip` through the same message and async-key path as SDL input:
//! - `KEY` taps (pressed, released two flips later), `KEY+` holds, `KEY-` releases;
//!   keys: A–Z, 0–9, F1–F12, ESC, ENTER, SPACE, TAB, LEFT, RIGHT, UP, DOWN,
//!   CTRL, SHIFT, ALT, PGUP, PGDN, HOME, END;
//! - `move@XxY` moves the pointer, `click@XxY` clicks the left button (released
//!   two flips later), in guest screen coordinates;
//! - `rel@DXxDY` is host mouse motion of (DX, DY) points (may be negative);
//! - `QUIT` asks the window to close, as the host's close button or Cmd-Q does;
//! - `SETTINGS[KEY KEY ...]` opens the settings overlay and gives it those keys,
//!   one per overlay tick (flips stop while it is open); it closes when they run
//!   out, if they haven't closed it.
//!
//! The simulation is deterministic, so a script replays identically.

use crate::{user, Runtime};

/// SDL scancode for a key name used by `--input`.
pub fn scancode(name: &str) -> Result<i32, String> {
    let n = name.to_ascii_uppercase();
    let b = n.as_bytes();
    Ok(match n.as_str() {
        _ if b.len() == 1 && b[0].is_ascii_uppercase() => 4 + (b[0] - b'A') as i32,
        _ if b.len() == 1 && b[0] == b'0' => 39,
        _ if b.len() == 1 && b[0].is_ascii_digit() => 30 + (b[0] - b'1') as i32,
        _ if n.starts_with('F') && n[1..].parse::<i32>().is_ok_and(|k| (1..=12).contains(&k)) => 57 + n[1..].parse::<i32>().unwrap(),
        "ENTER" => 40,
        "ESC" => 41,
        "TAB" => 43,
        "SPACE" => 44,
        "HOME" => 74,
        "PGUP" => 75,
        "END" => 77,
        "PGDN" => 78,
        "RIGHT" => 79,
        "LEFT" => 80,
        "DOWN" => 81,
        "UP" => 82,
        "CTRL" => 224,
        "SHIFT" => 225,
        "ALT" => 226,
        _ => return Err(format!("unknown key {name}")),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Key(i32, bool),
    Move((i32, i32)),
    Rel((i32, i32)),
    Button(bool, (i32, i32)),
    /// The host asks the window to close (as SDL_EVENT_QUIT / the close button).
    Quit,
    /// Open the settings overlay and give it these keys (SDL scancodes).
    Settings(&'static [i32]),
}

/// Parse a script into (flip, event) pairs.
pub fn parse(spec: &str) -> Result<Vec<(u64, Input)>, String> {
    let mut ev = Vec::new();
    let point = |s: &str| -> Result<(i32, i32), String> {
        let (x, y) = s.split_once('x').ok_or_else(|| format!("expected XxY, got {s}"))?;
        Ok((x.parse().map_err(|_| format!("bad x in {s}"))?, y.parse().map_err(|_| format!("bad y in {s}"))?))
    };
    for item in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (frame, key) = item.split_once(':').ok_or_else(|| format!("expected FRAME:KEY, got {item}"))?;
        let f: u64 = frame.parse().map_err(|_| format!("bad frame in {item}"))?;
        if key == "QUIT" {
            ev.push((f, Input::Quit));
        } else if let Some(keys) = key.strip_prefix("SETTINGS[").and_then(|k| k.strip_suffix(']')) {
            let keys: Vec<i32> = keys.split_whitespace().map(scancode).collect::<Result<_, _>>()?;
            ev.push((f, Input::Settings(Box::leak(keys.into_boxed_slice()))));
        } else if let Some(p) = key.strip_prefix("rel@") {
            ev.push((f, Input::Rel(point(p)?)));
        } else if let Some(p) = key.strip_prefix("move@") {
            ev.push((f, Input::Move(point(p)?)));
        } else if let Some(p) = key.strip_prefix("click@") {
            let p = point(p)?;
            ev.push((f, Input::Move(p)));
            ev.push((f, Input::Button(true, p)));
            ev.push((f + 2, Input::Button(false, p)));
        } else if let Some(k) = key.strip_suffix('+') {
            ev.push((f, Input::Key(scancode(k)?, true)));
        } else if let Some(k) = key.strip_suffix('-') {
            ev.push((f, Input::Key(scancode(k)?, false)));
        } else {
            ev.push((f, Input::Key(scancode(key)?, true)));
            ev.push((f + 2, Input::Key(scancode(key)?, false)));
        }
    }
    Ok(ev)
}

/// Deliver the events scheduled for flip `n` (call from `Runtime::frame_hook`).
pub fn apply(rt: &mut Runtime, events: &[(u64, Input)], n: u64) {
    for (_, ev) in events.iter().filter(|e| e.0 == n) {
        match *ev {
            Input::Key(sc, down) => user::key_input_rt(rt, sc, down, false, false),
            Input::Move(p) => user::mouse_move_rt(rt, p),
            Input::Rel((dx, dy)) => user::mouse_motion_rt(rt, dx as f32, dy as f32),
            Input::Button(down, p) => user::mouse_button_rt(rt, 1, down, p),
            Input::Quit => user::close_requested_rt(rt),
            Input::Settings(keys) => crate::overlay::open_scripted(rt, keys),
        }
    }
}
