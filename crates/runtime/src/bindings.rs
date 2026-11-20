//! Input bindings: which host keys and controller buttons press which of the
//! game's keys.
//!
//! This is a runtime layer in front of the game. The game still receives its
//! original key messages and key state, only for whichever host input is bound to
//! them. Each action is one of the keys the game reads (its profile, game.rs) or
//! the runtime's own [`SETTINGS_ACTION`], bound to a list of host inputs. Host keys
//! bound to no action pass through as themselves (for example letters typed into
//! high-score names), except an action's original key once it has been unbound.
//!
//! Bindings are the `[bindings]` table of `config.toml` (config.rs): an action id
//! and a list of inputs:
//! SDL key names ("PageUp", "Left Ctrl", "=") or "Pad " and an SDL gamepad button
//! name ("Pad start", "Pad back"). Actions missing from the file keep their defaults.

use std::collections::HashMap;

/// One of the game's keys, by the SDL scancode of its original key, or a runtime
/// action ([`SETTINGS`]).
#[derive(Clone, Copy, Debug)]
pub struct Action {
    pub id: &'static str,
    pub label: &'static str,
    pub key: i32,
    pub defaults: &'static [&'static str],
}

/// SDL scancodes, for game profiles' actions.
pub mod sc {
    pub const A: i32 = 4;
    pub const P: i32 = 19;
    pub const ESCAPE: i32 = 41;
    pub const SPACE: i32 = 44;
    pub const F1: i32 = 58;
    pub const F2: i32 = 59;
    pub const F3: i32 = 60;
    pub const INSERT: i32 = 73;
    pub const HOME: i32 = 74;
    pub const PAGE_UP: i32 = 75;
    pub const DELETE: i32 = 76;
    pub const END: i32 = 77;
    pub const PAGE_DOWN: i32 = 78;
    pub const RIGHT: i32 = 79;
    pub const LEFT: i32 = 80;
    pub const DOWN: i32 = 81;
    pub const UP: i32 = 82;
    pub const LCTRL: i32 = 224;
}

/// Not a game key: opens the settings overlay (handled by the runtime).
pub const SETTINGS: i32 = -1;

/// The runtime's action, added after every game's own.
pub const SETTINGS_ACTION: Action = Action { id: "settings", label: "Open settings", key: SETTINGS, defaults: &["F10", "Pad back"] };

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
    /// The game's actions and the settings action.
    actions: Vec<Action>,
    /// Per action (`actions` order), its inputs.
    pub inputs: Vec<Vec<Input>>,
    /// Host input → game keys (scancodes of the original keys).
    map: HashMap<Input, Vec<i32>>,
    /// Game key → number of host inputs holding it down.
    held: HashMap<i32, u32>,
}

impl Default for Bindings {
    /// The settings action only.
    fn default() -> Bindings {
        Bindings::new(&[])
    }
}

impl Bindings {
    /// A game's actions (and the settings action) with their default inputs.
    pub fn new(actions: &[Action]) -> Bindings {
        let actions: Vec<Action> = actions.iter().copied().chain([SETTINGS_ACTION]).collect();
        let inputs = actions.iter().map(|a| a.defaults.iter().filter_map(|n| Input::parse(n)).collect()).collect();
        Bindings::with_inputs(actions, inputs)
    }

    fn with_inputs(actions: Vec<Action>, inputs: Vec<Vec<Input>>) -> Bindings {
        let mut map: HashMap<Input, Vec<i32>> = HashMap::new();
        for (a, list) in actions.iter().zip(&inputs) {
            for &i in list {
                map.entry(i).or_default().push(a.key);
            }
        }
        Bindings { actions, inputs, map, held: HashMap::new() }
    }

    /// The actions, in order: the game's, then the settings action.
    pub fn actions(&self) -> &[Action] {
        &self.actions
    }

    /// The `[bindings]` table of `config.toml` (config.rs). Actions missing from it
    /// keep their defaults; unknown actions and input names are reported and skipped.
    pub(crate) fn from_table(actions: &[Action], table: Option<&toml::Table>, warnings: &mut Vec<String>) -> Bindings {
        let defaults = Bindings::new(actions);
        let mut inputs = defaults.inputs.clone();
        for (id, value) in table.into_iter().flatten() {
            let Some(a) = defaults.actions.iter().position(|a| a.id == id) else {
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
        let s = defaults.actions.iter().position(|a| a.key == SETTINGS).expect("the settings action");
        if inputs[s].is_empty() {
            warnings.push("bindings: settings: nothing would open the settings; using the default".into());
            inputs[s] = defaults.inputs[s].clone();
        }
        Bindings::with_inputs(defaults.actions, inputs)
    }

    /// For `config.toml`: each action's id, label and input names.
    pub(crate) fn entries(&self) -> impl Iterator<Item = (&'static str, &'static str, Vec<String>)> + '_ {
        self.actions.iter().zip(&self.inputs).map(|(a, l)| (a.id, a.label, l.iter().map(|i| i.name()).collect()))
    }

    /// Whether `input` opens the settings overlay.
    pub fn opens_settings(&self, input: Input) -> bool {
        self.map.get(&input).is_some_and(|k| k.contains(&SETTINGS))
    }

    /// Replace every action's inputs (the settings overlay). Release held keys first.
    pub fn set(&mut self, inputs: Vec<Vec<Input>>) {
        *self = Bindings::with_inputs(std::mem::take(&mut self.actions), inputs);
    }

    /// Every action's default inputs.
    pub fn defaults(&self) -> Vec<Vec<Input>> {
        self.actions.iter().map(|a| a.defaults.iter().filter_map(|n| Input::parse(n)).collect()).collect()
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
            (None, Input::Key(sc)) if !self.actions.iter().any(|a| a.key == sc) => vec![sc],
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

    /// A few game keys, as a profile would give them.
    const TEST_ACTIONS: &[Action] = &[
        Action { id: "fire", label: "Fire", key: sc::LCTRL, defaults: &["Left Ctrl", "Right Ctrl"] },
        Action { id: "pause", label: "Pause", key: sc::P, defaults: &["P", "Pad start"] },
        Action { id: "music_up", label: "Music volume up", key: sc::PAGE_UP, defaults: &["PageUp", "="] },
    ];

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
        let mut b = Bindings::new(TEST_ACTIONS);
        assert_eq!(b.press(key("PageUp"), true, false), [(sc::PAGE_UP, true, false)]);
        assert_eq!(b.press(key("PageUp"), false, false), [(sc::PAGE_UP, false, false)]);
        assert_eq!(b.press(key("="), true, false), [(sc::PAGE_UP, true, false)]);
        assert_eq!(b.press(key("Pad start"), true, false), [(sc::P, true, false)]);
        // Unbound keys are themselves (high-score names); unbound buttons do nothing.
        assert_eq!(b.press(key("A"), true, false), [(sc::A, true, false)]);
        assert_eq!(b.press(key("Space"), true, false), [(sc::SPACE, true, false)], "Space stays itself");
        assert_eq!(b.press(key("Pad dpup"), true, false), []);
        // The overlay key is a runtime action, added after the game's.
        assert_eq!(b.actions().last().map(|a| a.id), Some("settings"));
        assert!(b.opens_settings(key("F10")) && b.opens_settings(key("Pad back")));
        assert!(!b.opens_settings(key("F1")));
        assert_eq!(b.press(key("F10"), true, false), [(SETTINGS, true, false)]);
    }

    #[test]
    fn the_settings_binding_cannot_be_emptied() {
        let table: toml::Table = toml::from_str("settings = []").unwrap();
        let mut warnings = Vec::new();
        let b = Bindings::from_table(TEST_ACTIONS, Some(&table), &mut warnings);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(b.opens_settings(key("F10")));
    }

    #[test]
    fn a_key_held_through_two_inputs_is_released_with_the_last() {
        let mut b = Bindings::new(TEST_ACTIONS);
        assert_eq!(b.press(key("Left Ctrl"), true, false), [(sc::LCTRL, true, false)]);
        assert_eq!(b.press(key("Right Ctrl"), true, false), [(sc::LCTRL, true, true)]);
        assert_eq!(b.press(key("Left Ctrl"), false, false), []);
        assert_eq!(b.press(key("Right Ctrl"), false, false), [(sc::LCTRL, false, false)]);
    }
}
