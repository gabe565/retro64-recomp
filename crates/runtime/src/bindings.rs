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
//! Bindings are read from `config.toml` in the save folder (written by the
//! settings overlay), as `[bindings]` with an action id and a list of inputs:
//! SDL key names ("PageUp", "Left Ctrl", "=") or "Pad " and an SDL gamepad button
//! name ("Pad start", "Pad back"). Actions missing from the file keep their defaults.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// One of the game's keys, by the SDL scancode of its original key.
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

/// The game's keys (Gold 1.20 input poll 0x411f70 and its title screen), with the
/// defaults: every original key, plus alternatives on keys every keyboard has
/// for the volume keys laptops lack, and controller buttons for pause and exit
/// (the joystick itself steers and fires; any fire button also starts a game).
pub const ACTIONS: &[Action] = &[
    Action { id: "left", label: "Move left", key: LEFT, defaults: &["Left"] },
    Action { id: "right", label: "Move right", key: RIGHT, defaults: &["Right"] },
    Action { id: "up", label: "Move up", key: UP, defaults: &["Up"] },
    Action { id: "down", label: "Move down", key: DOWN, defaults: &["Down"] },
    // Space also fires, but stays itself: high-score names and "press a key" screens read it.
    Action { id: "fire", label: "Fire (Space always fires too)", key: LCTRL, defaults: &["Left Ctrl", "Right Ctrl"] },
    Action { id: "start", label: "Start game", key: F1, defaults: &["F1"] },
    Action { id: "high_scores", label: "High scores", key: F2, defaults: &["F2"] },
    Action { id: "difficulty", label: "Difficulty", key: F3, defaults: &["F3"] },
    Action { id: "pause", label: "Pause", key: P, defaults: &["P", "Pad start"] },
    Action { id: "exit", label: "Exit", key: ESCAPE, defaults: &["Escape", "Pad back"] },
    Action { id: "music_up", label: "Music volume up", key: PAGE_UP, defaults: &["PageUp", "="] },
    Action { id: "music_down", label: "Music volume down", key: PAGE_DOWN, defaults: &["PageDown", "-"] },
    Action { id: "sound_up", label: "Sound volume up", key: HOME, defaults: &["Home", "]"] },
    Action { id: "sound_down", label: "Sound volume down", key: END, defaults: &["End", "["] },
    Action { id: "screenshot", label: "Screenshot", key: INSERT, defaults: &["Insert"] },
    Action { id: "frame_time", label: "Show frame time", key: DELETE, defaults: &["Delete"] },
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

/// The top of `config.toml`.
const HEADER: &str = "\
# Bugatron settings. Rewritten by the settings overlay; edit while the game is closed.
#
# [bindings]: for each of the game's keys, the host keys and controller buttons that
# press it. Keys are SDL key names (\"PageUp\", \"Left Ctrl\", \"=\"); buttons are \"Pad \" and
# an SDL button name (\"Pad start\", \"Pad leftshoulder\"). Keys bound to nothing type
# themselves (high-score names); a game key removed from its own list stops working.

";

/// The config file's shape.
#[derive(serde::Deserialize, serde::Serialize, Default)]
struct ConfigFile {
    #[serde(default)]
    bindings: BTreeMap<String, Vec<String>>,
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

    /// Read `config.toml`; a missing file means the defaults. Unknown actions and
    /// input names are reported and skipped.
    pub fn load(path: &Path) -> (Bindings, Vec<String>) {
        let mut warnings = Vec::new();
        let file: ConfigFile = match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                warnings.push(format!("{}: {e}", path.display()));
                ConfigFile::default()
            }),
            Err(_) => {
                // First run: write the defaults, so the file documents itself.
                if let Err(e) = Bindings::default().save(path) {
                    warnings.push(format!("{}: cannot write the defaults: {e}", path.display()));
                }
                ConfigFile::default()
            }
        };
        let mut inputs: Vec<Vec<Input>> = Bindings::default().inputs;
        for (id, names) in &file.bindings {
            let Some(a) = ACTIONS.iter().position(|a| a.id == id) else {
                warnings.push(format!("{}: unknown action {id:?}", path.display()));
                continue;
            };
            inputs[a] = names
                .iter()
                .filter_map(|n| {
                    let i = Input::parse(n);
                    if i.is_none() {
                        warnings.push(format!("{}: {id}: unknown key or button {n:?}", path.display()));
                    }
                    i
                })
                .collect();
        }
        (Bindings::from_inputs(inputs), warnings)
    }

    /// Write every action's inputs to `config.toml`, keeping the file's other
    /// settings. Each action gets a comment naming it.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut doc: toml::Table = std::fs::read_to_string(path).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default();
        doc.remove("bindings");
        let mut text = String::from(HEADER);
        let rest = toml::to_string(&doc).expect("a TOML table serialises");
        if !rest.is_empty() {
            text += &rest;
            text.push('\n');
        }
        text += "[bindings]\n";
        for (a, list) in ACTIONS.iter().zip(&self.inputs) {
            let names: Vec<String> = list.iter().map(|i| toml::Value::String(i.name()).to_string()).collect();
            text += &format!("# {}\n{} = [{}]\n", a.label, a.id, names.join(", "));
        }
        std::fs::write(path, text)
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
    }

    #[test]
    fn a_key_held_through_two_inputs_is_released_with_the_last() {
        let mut b = Bindings::default();
        assert_eq!(b.press(key("Left Ctrl"), true, false), [(LCTRL, true, false)]);
        assert_eq!(b.press(key("Right Ctrl"), true, false), [(LCTRL, true, true)]);
        assert_eq!(b.press(key("Left Ctrl"), false, false), []);
        assert_eq!(b.press(key("Right Ctrl"), false, false), [(LCTRL, false, false)]);
    }

    #[test]
    fn config_overrides_actions_and_unbinds_original_keys() {
        let dir = std::env::temp_dir().join(format!("bugatron-bindings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "[bindings]\nstart = [\"Return\", \"Pad y\"]\nfly = [\"F9\"]\nexit = [\"Escape\", \"Nope\"]\n").unwrap();
        let (mut b, warnings) = Bindings::load(&path);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert_eq!(b.press(key("Return"), true, false), [(F1, true, false)]);
        assert_eq!(b.press(key("F1"), true, false), [], "F1 no longer starts a game");
        assert_eq!(b.press(key("PageDown"), true, false), [(PAGE_DOWN, true, false)], "defaults kept");
        // Saving writes every action and keeps other settings.
        std::fs::write(&path, "volume = 3\n").unwrap();
        b.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# Bugatron settings"), "{text}");
        assert!(text.contains("volume = 3") && text.contains("# Start game\nstart = [\"Return\", \"Pad y\"]"), "{text}");
        let (again, warnings) = Bindings::load(&path);
        assert!(warnings.is_empty());
        assert_eq!(again.inputs, b.inputs);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_config_is_written_with_the_defaults() {
        let dir = std::env::temp_dir().join(format!("bugatron-bindings-new-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let (b, warnings) = Bindings::load(&path);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(b.inputs, Bindings::default().inputs);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# Music volume up\nmusic_up = [\"PageUp\", \"=\"]"), "{text}");
        let (again, warnings) = Bindings::load(&path);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(again.inputs, b.inputs);
        std::fs::remove_dir_all(&dir).ok();
    }
}
