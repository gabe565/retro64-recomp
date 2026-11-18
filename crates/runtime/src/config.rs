//! `config.toml` in the save folder: the runtime's settings. The first run writes
//! it with the defaults and comments; the settings overlay rewrites it. Scripted
//! runs (`--input`) ignore it.
//!
//! - `[bindings]`: host keys and controller buttons for the game's keys (bindings.rs).

use crate::bindings::Bindings;
use crate::Runtime;
use std::path::{Path, PathBuf};

/// Everything `config.toml` holds.
#[derive(Default)]
pub struct Config {
    pub bindings: Bindings,
}

const HEADER: &str = "\
# Bugatron settings. Rewritten by the settings overlay (F10); edit while the game is closed.
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
    let bindings = Bindings::from_table(table.get("bindings").and_then(|v| v.as_table()), &mut warnings);
    let warnings = warnings.into_iter().map(|w| format!("{}: {w}", path.display())).collect();
    (Config { bindings }, warnings)
}

/// Write `config.toml`.
pub fn save(c: &Config, path: &Path) -> std::io::Result<()> {
    let mut text = format!("{HEADER}\n{BINDINGS}");
    c.bindings.write(&mut text);
    std::fs::write(path, text)
}

/// Load the settings into the runtime (host-input runs only).
pub fn load_into(rt: &mut Runtime) {
    let (c, warnings) = load(&path(rt));
    for w in warnings {
        eprintln!("config: {w}");
    }
    rt.user.bindings = c.bindings;
}

/// Save the runtime's settings (after the overlay changes them).
pub fn save_from(rt: &mut Runtime) {
    let c = Config { bindings: std::mem::take(&mut rt.user.bindings) };
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
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# Bugatron settings"), "{text}");
        assert!(text.contains("# Music volume up\nmusic_up = [\"PageUp\", \"=\"]"), "{text}");
        let (again, warnings) = load(&path);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(again.bindings.inputs, c.bindings.inputs);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn bindings_override_defaults_and_unknown_ones_are_reported() {
        let d = dir("edit");
        let path = d.join("config.toml");
        std::fs::write(&path, "[bindings]\nstart = [\"Return\", \"Pad y\"]\nfly = [\"F9\"]\nexit = [\"Escape\", \"Nope\"]\n").unwrap();
        let (mut c, warnings) = load(&path);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert_eq!(c.bindings.press(key("Return"), true, false), [(58, true, false)]);
        assert_eq!(c.bindings.press(key("F1"), true, false), [], "F1 no longer starts a game");
        assert_eq!(c.bindings.press(key("PageDown"), true, false), [(78, true, false)], "defaults kept");
        save(&c, &path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# Start game\nstart = [\"Return\", \"Pad y\"]"), "{text}");
        let (again, warnings) = load(&path);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(again.bindings.inputs, c.bindings.inputs);
        std::fs::remove_dir_all(&d).ok();
    }
}
