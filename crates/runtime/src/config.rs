//! `config.toml` in the save folder: the runtime's settings. The first run writes
//! it with the defaults and comments; the settings overlay rewrites it. Scripted
//! runs (`--input`) ignore it.
//!
//! - `[display]`: resolution and fullscreen (command-line options override them).
//! - `[bindings]`: host keys and controller buttons for the game's keys (bindings.rs).
//! - `[fixes]`: corrections of bugs in the original game, each on by default; set
//!   one to `false` for the original's behaviour.

use crate::bindings::Bindings;
use crate::Runtime;
use std::path::{Path, PathBuf};

/// Corrections of bugs in the original game, made by the runtime (the game's code
/// is unchanged).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fixes {
    /// The title screen's "F3: DIFFICULTY - NORMAL" follows the actual difficulty
    /// (overlay::fix_difficulty_label).
    pub difficulty_label: bool,
}

impl Default for Fixes {
    fn default() -> Fixes {
        Fixes { difficulty_label: true }
    }
}

/// Display settings, applied at startup (the game sets its mode once).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Display {
    /// The guest resolution (32 bpp); None: the largest 4:3 mode that fits the display.
    pub resolution: Option<(u32, u32)>,
    pub fullscreen: bool,
}

impl Default for Display {
    fn default() -> Display {
        Display { resolution: None, fullscreen: true }
    }
}

/// Everything `config.toml` holds.
#[derive(Default)]
pub struct Config {
    pub display: Display,
    pub fixes: Fixes,
    pub bindings: Bindings,
}

const HEADER: &str = "\
# Bugatron settings. Rewritten by the settings overlay (F10); edit while the game is closed.
";

const DISPLAY: &str = "\
# [display]: resolution is \"auto\" (the largest 4:3 mode that fits the display) or
# \"WIDTHxHEIGHT\"; it takes effect when the game starts. --video-mode and
# --windowed override these.
[display]
";

const FIXES: &str = "\
# [fixes]: corrections of bugs in the original game. Set one to false for the
# original's behaviour.
[fixes]
# The title screen shows the difficulty actually set. The original updates the
# label only when F3 is pressed, so it says NORMAL again whenever the title is
# rebuilt (after a game or the high scores), whatever the difficulty.
";

const BINDINGS: &str = "\
# [bindings]: for each of the game's keys, the host keys and controller buttons that
# press it. Keys are SDL key names (\"PageUp\", \"Left Ctrl\", \"=\"); buttons are \"Pad \" and
# an SDL button name (\"Pad start\", \"Pad leftshoulder\"). Keys bound to nothing type
# themselves (high-score names); a game key removed from its own list stops working.
# Space always fires and can't be rebound: high-score names use it.
";

pub fn path(rt: &Runtime) -> PathBuf {
    rt.files.save_dir.join("config.toml")
}

/// Read `config.toml`; a missing file is written with the defaults. Problems are
/// returned as warnings and the defaults used for what could not be read.
pub fn load(path: &Path) -> (Config, Vec<String>) {
    let mut warnings = Vec::new();
    let table: toml::Table = match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
            warnings.push(format!("{e}"));
            toml::Table::new()
        }),
        Err(_) => {
            if let Err(e) = save(&Config::default(), path) {
                warnings.push(format!("cannot write the defaults: {e}"));
            }
            toml::Table::new()
        }
    };
    let mut display = Display::default();
    for (k, v) in table.get("display").and_then(|v| v.as_table()).into_iter().flatten() {
        match (k.as_str(), v) {
            ("fullscreen", toml::Value::Boolean(b)) => display.fullscreen = *b,
            ("resolution", toml::Value::String(r)) if r == "auto" => display.resolution = None,
            ("resolution", toml::Value::String(r)) => match r.split_once('x').and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?))) {
                Some((w, h)) if w >= 320 && h >= 240 => display.resolution = Some((w, h)),
                _ => warnings.push(format!("display: resolution {r:?} is not \"auto\" or WIDTHxHEIGHT")),
            },
            _ => warnings.push(format!("display: unknown setting {k} = {v}")),
        }
    }
    let mut fixes = Fixes::default();
    for (k, v) in table.get("fixes").and_then(|v| v.as_table()).into_iter().flatten() {
        match (k.as_str(), v.as_bool()) {
            ("difficulty_label", Some(b)) => fixes.difficulty_label = b,
            _ => warnings.push(format!("fixes: unknown setting {k} = {v}")),
        }
    }
    let bindings = Bindings::from_table(table.get("bindings").and_then(|v| v.as_table()), &mut warnings);
    let warnings = warnings.into_iter().map(|w| format!("{}: {w}", path.display())).collect();
    (Config { display, fixes, bindings }, warnings)
}

/// Write `config.toml`.
pub fn save(c: &Config, path: &Path) -> std::io::Result<()> {
    let res = c.display.resolution.map_or("auto".to_string(), |(w, h)| format!("{w}x{h}"));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut text = format!("{HEADER}\n{DISPLAY}resolution = \"{res}\"\nfullscreen = {}\n\n{BINDINGS}", c.display.fullscreen);
    c.bindings.write(&mut text);
    text += &format!("\n{FIXES}difficulty_label = {}\n", c.fixes.difficulty_label);
    std::fs::write(path, text)
}

/// Load the settings into the runtime (host-input runs only).
pub fn load_into(rt: &mut Runtime) {
    let (c, warnings) = load(&path(rt));
    for w in warnings {
        eprintln!("config: {w}");
    }
    rt.display = c.display;
    rt.fixes = c.fixes;
    rt.user.bindings = c.bindings;
}

/// Save the runtime's settings (after the overlay changes them).
pub fn save_from(rt: &mut Runtime) {
    let c = Config { display: rt.display.clone(), fixes: rt.fixes.clone(), bindings: std::mem::take(&mut rt.user.bindings) };
    let path = path(rt);
    if let Err(e) = save(&c, &path) {
        eprintln!("config: {}: {e}", path.display());
    }
    rt.user.bindings = c.bindings;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::Input;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bugatron-config-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn key(name: &str) -> Input {
        Input::parse(name).unwrap()
    }

    #[test]
    fn a_missing_config_is_written_with_the_defaults() {
        let d = dir("new");
        let path = d.join("config.toml");
        let (c, warnings) = load(&path);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(c.fixes, Fixes::default());
        assert_eq!(c.display, Display::default());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# Bugatron settings"), "{text}");
        assert!(text.contains("[fixes]") && text.contains("difficulty_label = true"), "{text}");
        assert!(text.contains("# Music volume up\nmusic_up = [\"PageUp\", \"=\"]"), "{text}");
        let (again, warnings) = load(&path);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(again.bindings.inputs, c.bindings.inputs);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn settings_override_defaults_and_unknown_ones_are_reported() {
        let d = dir("edit");
        let path = d.join("config.toml");
        std::fs::write(&path, "[display]\nresolution = \"1280x960\"\nfullscreen = false\n[fixes]\ndifficulty_label = false\nwings = true\n[bindings]\nstart = [\"Return\", \"Pad y\"]\nfly = [\"F9\"]\nexit = [\"Escape\", \"Nope\"]\n").unwrap();
        let (mut c, warnings) = load(&path);
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(!c.fixes.difficulty_label);
        assert_eq!(c.display, Display { resolution: Some((1280, 960)), fullscreen: false });
        assert_eq!(c.bindings.press(key("Return"), true, false), [(58, true, false)]);
        assert_eq!(c.bindings.press(key("F1"), true, false), [], "F1 no longer starts a game");
        assert_eq!(c.bindings.press(key("PageDown"), true, false), [(78, true, false)], "defaults kept");
        save(&c, &path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("difficulty_label = false") && text.contains("# Start game\nstart = [\"Return\", \"Pad y\"]"), "{text}");
        let (again, warnings) = load(&path);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(again.bindings.inputs, c.bindings.inputs);
        assert_eq!(again.fixes, c.fixes);
        assert_eq!(again.display, c.display);
        std::fs::remove_dir_all(&d).ok();
    }
}
