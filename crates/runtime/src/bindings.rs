//! Input bindings: which host keys and controller buttons press which of the
//! game's keys.
//!
//! This is a runtime layer in front of the game. The game still receives its
//! original key messages and key state, only for whichever host input is bound to
//! them. Each [`ACTIONS`] entry is one of the keys the game's input poll reads
//! (table 0x437f9c, read by 0x411f70), bound to a list of host inputs. Host keys
//! bound to no action pass through as themselves (letters, digits, Enter and
//! Backspace for high-score names, F8+F9...), except an action's original key
//! once it has been unbound from it.
//!
//! Bindings are the `[bindings]` table of `config.toml` (config.rs): an action id
//! and a list of inputs:
//! SDL key names ("PageUp", "Left Ctrl", "=") or "Pad " and an SDL gamepad button
//! name ("Pad start", "Pad back"). Actions missing from the file keep their defaults.

use std::collections::HashMap;

/// One of the game's keys, by the SDL scancode of its original key, or a runtime
/// action ([`SETTINGS`]).
pub struct Action {
    pub id: &'static str,
    pub label: &'static str,
    pub key: i32,
    pub defaults: &'static [&'static str],
}

// SDL scancodes of the original keys.
const LEFT: i32 = 80;
const RIGHT: i32 = 79;
const UP: i32 = 82;
const DOWN: i32 = 81;
const LCTRL: i32 = 224;
const INSERT: i32 = 73;
const DELETE: i32 = 76;
const F1: i32 = 58;
const F2: i32 = 59;
const F3: i32 = 60;
const ESCAPE: i32 = 41;
const PAGE_UP: i32 = 75;
const PAGE_DOWN: i32 = 78;
const HOME: i32 = 74;
const END: i32 = 77;
const P: i32 = 19;
/// Not a game key: opens the settings overlay (handled by the runtime).
pub const SETTINGS: i32 = -1;

/// The game's keys (Gold 1.20 input poll 0x411f70 and its title screen), with the
/// defaults: every original key, plus alternatives on keys every keyboard has
/// for the volume keys laptops lack, and controller Start for pause (the joystick
/// itself steers and fires, and any fire button also starts a game), and F10 or
/// controller Back for the settings overlay.
pub const ACTIONS: &[Action] = &[
    Action { id: "left", label: "Move left", key: LEFT, defaults: &["Left"] },
    Action { id: "right", label: "Move right", key: RIGHT, defaults: &["Right"] },
    Action { id: "up", label: "Move up", key: UP, defaults: &["Up"] },
    Action { id: "down", label: "Move down", key: DOWN, defaults: &["Down"] },
    // Space also fires, but stays itself: high-score names and "press a key" screens read it.
    Action { id: "fire", label: "Fire", key: LCTRL, defaults: &["Left Ctrl", "Right Ctrl"] },
    Action { id: "start", label: "Start game", key: F1, defaults: &["F1"] },
    Action { id: "high_scores", label: "High scores", key: F2, defaults: &["F2"] },
    Action { id: "difficulty", label: "Difficulty", key: F3, defaults: &["F3"] },
    Action { id: "pause", label: "Pause", key: P, defaults: &["P", "Pad start"] },
    Action { id: "exit", label: "Exit", key: ESCAPE, defaults: &["Escape"] },
    Action { id: "music_up", label: "Music volume up", key: PAGE_UP, defaults: &["PageUp", "="] },
    Action { id: "music_down", label: "Music volume down", key: PAGE_DOWN, defaults: &["PageDown", "-"] },
    Action { id: "sound_up", label: "Sound volume up", key: HOME, defaults: &["Home", "]"] },
    Action { id: "sound_down", label: "Sound volume down", key: END, defaults: &["End", "["] },
    Action { id: "screenshot", label: "Screenshot", key: INSERT, defaults: &["Insert"] },
    Action { id: "frame_time", label: "Show FPS", key: DELETE, defaults: &["Delete"] },
    Action { id: "settings", label: "Open settings", key: SETTINGS, defaults: &["F10", "Pad back"] },
];

/// A host input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Input {
    /// SDL scancode.
    Key(i32),
    /// SDL gamepad button.
    Pad(i32),
}

impl Input {
    /// Parse a config name: an SDL key name, or "Pad " and an SDL gamepad button name.
    pub fn parse(name: &str) -> Option<Input> {
        let name = name.trim();
        if let Some(b) = name.strip_prefix("Pad ").or_else(|| name.strip_prefix("pad ")) {
            let c = std::ffi::CString::new(b.trim()).ok()?;
            // SAFETY: NUL-terminated string; SDL only reads it.
            let b = unsafe { sdl3_sys::gamepad::SDL_GetGamepadButtonFromString(c.as_ptr()) };
            return (b.0 >= 0).then_some(Input::Pad(b.0));
        }
        let c = std::ffi::CString::new(name).ok()?;
        // SAFETY: NUL-terminated string; SDL only reads it.
        let sc = unsafe { sdl3_sys::keyboard::SDL_GetScancodeFromName(c.as_ptr()) };
        (sc.0 > 0).then_some(Input::Key(sc.0))
    }

    /// The config name.
    pub fn name(self) -> String {
        // SAFETY: SDL returns static NUL-terminated strings (or null for buttons).
        unsafe {
            match self {
                Input::Key(sc) => {
                    let p = sdl3_sys::keyboard::SDL_GetScancodeName(sdl3_sys::scancode::SDL_Scancode(sc));
                    std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
                }
                Input::Pad(b) => {
                    let p = sdl3_sys::gamepad::SDL_GetGamepadStringForButton(sdl3_sys::gamepad::SDL_GamepadButton(b));
                    let s = if p.is_null() { "?".into() } else { std::ffi::CStr::from_ptr(p).to_string_lossy() };
                    format!("Pad {s}")
                }
            }
        }
    }
}

/// The current bindings and which game keys are held through them.
pub struct Bindings {
    /// Per action (ACTIONS order), its inputs.
    pub inputs: Vec<Vec<Input>>,
    /// Host input → game keys (scancodes of the original keys).
    map: HashMap<Input, Vec<i32>>,
    /// Game key → number of host inputs holding it down.
    held: HashMap<i32, u32>,
}

impl Default for Bindings {
    fn default() -> Bindings {
        Bindings::from_inputs(ACTIONS.iter().map(|a| a.defaults.iter().filter_map(|n| Input::parse(n)).collect()).collect())
    }
}

impl Bindings {
    fn from_inputs(inputs: Vec<Vec<Input>>) -> Bindings {
        let mut map: HashMap<Input, Vec<i32>> = HashMap::new();
        for (a, list) in ACTIONS.iter().zip(&inputs) {
            for &i in list {
                map.entry(i).or_default().push(a.key);
            }
        }
        Bindings { inputs, map, held: HashMap::new() }
    }

    /// The `[bindings]` table of `config.toml` (config.rs). Actions missing from it
    /// keep their defaults; unknown actions and input names are reported and skipped.
    pub(crate) fn from_table(table: Option<&toml::Table>, warnings: &mut Vec<String>) -> Bindings {
        let mut inputs: Vec<Vec<Input>> = Bindings::default().inputs;
        for (id, value) in table.into_iter().flatten() {
            let Some(a) = ACTIONS.iter().position(|a| a.id == id) else {
                warnings.push(format!("bindings: unknown action {id:?}"));
                continue;
            };
            let Some(names) = value.as_array() else {
                warnings.push(format!("bindings: {id}: not a list"));
                continue;
            };
            inputs[a] = names
                .iter()
                .filter_map(|n| {
                    let i = n.as_str().and_then(Input::parse);
                    if i.is_none() {
                        warnings.push(format!("bindings: {id}: unknown key or button {n}"));
                    }
                    i
                })
                .collect();
        }
        // Keep a way to open the overlay.
        let s = ACTIONS.iter().position(|a| a.key == SETTINGS).expect("the settings action");
        if inputs[s].is_empty() {
            warnings.push("bindings: settings: nothing would open the settings; using the default".into());
            inputs[s] = Bindings::default().inputs[s].clone();
        }
        Bindings::from_inputs(inputs)
    }

    /// The `[bindings]` section of `config.toml`, with a comment naming each action.
    pub(crate) fn write(&self, text: &mut String) {
        *text += "[bindings]\n";
        for (a, list) in ACTIONS.iter().zip(&self.inputs) {
            let names: Vec<String> = list.iter().map(|i| toml::Value::String(i.name()).to_string()).collect();
            *text += &format!("# {}\n{} = [{}]\n", a.label, a.id, names.join(", "));
        }
    }

    /// Whether `input` opens the settings overlay.
    pub fn opens_settings(&self, input: Input) -> bool {
        self.map.get(&input).is_some_and(|k| k.contains(&SETTINGS))
    }

    /// Replace every action's inputs (the settings overlay). Release held keys first.
    pub fn set(&mut self, inputs: Vec<Vec<Input>>) {
        *self = Bindings::from_inputs(inputs);
    }

    /// Release every game key held through a binding: the transitions to send.
    pub fn release_all(&mut self) -> Vec<(i32, bool, bool)> {
        let out = self.held.iter().filter(|&(_, &n)| n > 0).map(|(&k, _)| (k, false, false)).collect();
        self.held.clear();
        out
    }

    /// A host input went down or up: the game key transitions to send, as
    /// (scancode, down, repeat). Keys bound to several inputs stay down until all
    /// are released.
    pub fn press(&mut self, input: Input, down: bool, repeat: bool) -> Vec<(i32, bool, bool)> {
        let keys = match (self.map.get(&input), input) {
            (Some(k), _) => k.clone(),
            // An original key unbound from its action does nothing; other keys are themselves.
            (None, Input::Key(sc)) if !ACTIONS.iter().any(|a| a.key == sc) => vec![sc],
            _ => return Vec::new(),
        };
        let mut out = Vec::new();
        for k in keys {
            let n = self.held.entry(k).or_default();
            match (down, repeat) {
                (true, true) => out.push((k, true, true)),
                (true, false) => {
                    *n += 1;
                    out.push((k, true, *n > 1));
                }
                (false, _) => {
                    *n = n.saturating_sub(1);
                    if *n == 0 {
                        out.push((k, false, false));
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str) -> Input {
        Input::parse(name).unwrap_or_else(|| panic!("{name}"))
    }

    #[test]
    fn names_round_trip() {
        for n in ["PageUp", "Left Ctrl", "=", "]", "F1", "Space", "Pad start", "Pad back", "Pad a"] {
            assert_eq!(key(n).name(), n);
        }
        assert_eq!(Input::parse("NoSuchKey"), None);
        assert_eq!(Input::parse("Pad nosuchbutton"), None);
    }

    #[test]
    fn defaults_keep_original_keys_and_add_alternatives() {
        let mut b = Bindings::default();
        assert_eq!(b.press(key("PageUp"), true, false), [(PAGE_UP, true, false)]);
        assert_eq!(b.press(key("PageUp"), false, false), [(PAGE_UP, false, false)]);
        assert_eq!(b.press(key("="), true, false), [(PAGE_UP, true, false)]);
        assert_eq!(b.press(key("Pad start"), true, false), [(P, true, false)]);
        // Unbound keys are themselves (high-score names); unbound buttons do nothing.
        assert_eq!(b.press(key("A"), true, false), [(4, true, false)]);
        assert_eq!(b.press(key("Space"), true, false), [(44, true, false)], "Space stays itself");
        assert_eq!(b.press(key("Pad dpup"), true, false), []);
        // The overlay key is a runtime action, not a game key.
        assert!(b.opens_settings(key("F10")) && b.opens_settings(key("Pad back")));
        assert!(!b.opens_settings(key("F1")));
        assert_eq!(b.press(key("F10"), true, false), [(SETTINGS, true, false)]);
    }

    #[test]
    fn the_settings_binding_cannot_be_emptied() {
        let table: toml::Table = toml::from_str("settings = []").unwrap();
        let mut warnings = Vec::new();
        let b = Bindings::from_table(Some(&table), &mut warnings);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(b.opens_settings(key("F10")));
    }

    #[test]
    fn a_key_held_through_two_inputs_is_released_with_the_last() {
        let mut b = Bindings::default();
        assert_eq!(b.press(key("Left Ctrl"), true, false), [(LCTRL, true, false)]);
        assert_eq!(b.press(key("Right Ctrl"), true, false), [(LCTRL, true, true)]);
        assert_eq!(b.press(key("Left Ctrl"), false, false), []);
        assert_eq!(b.press(key("Right Ctrl"), false, false), [(LCTRL, false, false)]);
    }
}
