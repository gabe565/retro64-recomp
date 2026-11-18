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
    /// Go straight from one screen to the next, without the "LOADING" hold
    /// (overlay.rs `skip_loading`).
    pub skip_loading: bool,
}

impl Default for Display {
    fn default() -> Display {
        Display { resolution: None, fullscreen: true, skip_loading: true }
    }
}

/// Everything `config.toml` holds.
#[derive(Default)]
pub struct Config {
    pub display: Display,
    pub fixes: Fixes,
    pub bindings: Bindings,
}

pub fn path(rt: &Runtime) -> PathBuf {
    rt.files.save_dir.join("config.toml")
}

/// Read `config.toml`; a missing file is written with the defaults, and a file that
/// differs from what this version writes (older comments, settings added since) is
/// rewritten with its values. A file with problems is left for the user to fix:
/// they are returned as warnings and the defaults used for what could not be read.
pub fn load(path: &Path) -> (Config, Vec<String>) {
    let mut warnings = Vec::new();
    let existing = std::fs::read_to_string(path).ok();
    let table: toml::Table = match &existing {
        Some(text) => toml::from_str(text).unwrap_or_else(|e| {
            warnings.push(format!("{e}"));
            toml::Table::new()
        }),
        None => {
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
            ("skip_loading", toml::Value::Boolean(b)) => display.skip_loading = *b,
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
    let config = Config { display, fixes, bindings };
    if existing.as_ref().is_some_and(|text| warnings.is_empty() && *text != render(&config)) {
        if let Err(e) = save(&config, path) {
            warnings.push(format!("cannot update it: {e}"));
        }
    }
    let warnings = warnings.into_iter().map(|w| format!("{}: {w}", path.display())).collect();
    (config, warnings)
}

/// Write `config.toml`.
pub fn save(c: &Config, path: &Path) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, render(c))
}

/// The text of `config.toml` for `c`: every setting, each with a comment.
fn render(c: &Config) -> String {
    use toml_edit::{value, Array, DocumentMut, Item, Table};
    /// A table with a comment above its header (and a blank line, after the first).
    fn section(comment: &str, first: bool) -> Table {
        let mut t = Table::new();
        t.decor_mut().set_prefix(if first { "" } else { "\n" }.to_string() + &comment_lines(comment));
        t
    }
    /// A setting with a comment above it.
    fn set(t: &mut Table, key: &str, v: impl Into<Item>, comment: &str) {
        t.insert(key, v.into());
        t.key_mut(key).expect("just inserted").leaf_decor_mut().set_prefix(comment_lines(comment));
    }

    let mut display = section(
        "Bugatron settings. The game rewrites this file when the settings overlay (F10) changes \
         them, and on start when it was written by an older version; edit it while the game is closed.\n\n\
         [display]: --video-mode and --windowed override these.",
        true,
    );
    let d = &c.display;
    let res = d.resolution.map_or("auto".to_string(), |(w, h)| format!("{w}x{h}"));
    set(&mut display, "resolution", value(res), "\"auto\" (the largest 4:3 mode that fits the display) or \"WIDTHxHEIGHT\"; takes effect when the game starts.");
    set(&mut display, "fullscreen", value(d.fullscreen), "Fullscreen, or a window.");
    set(&mut display, "skip_loading", value(d.skip_loading), "Go straight from one screen to the next. The original holds an empty \"LOADING\" screen between them for most of a second.");

    let mut bindings = section(
        "[bindings]: for each of the game's keys, the host keys and controller buttons that press it. \
         Keys are SDL key names (\"PageUp\", \"Left Ctrl\", \"=\"); buttons are \"Pad \" and an SDL button name \
         (\"Pad start\", \"Pad leftshoulder\"). Keys bound to nothing type themselves (high-score names); \
         a game key removed from its own list stops working. Space always fires and can't be rebound: \
         high-score names use it.",
        false,
    );
    for (id, label, names) in c.bindings.entries() {
        set(&mut bindings, id, value(names.into_iter().collect::<Array>()), label);
    }

    let mut fixes = section("[fixes]: corrections of bugs in the original game; false keeps the original's behaviour.", false);
    set(
        &mut fixes,
        "difficulty_label",
        value(c.fixes.difficulty_label),
        "The title screen shows the difficulty actually set. The original says NORMAL again whenever the title is \
         rebuilt (after a game or the high scores), whatever the difficulty.",
    );

    let mut doc = DocumentMut::new();
    doc.insert("display", Item::Table(display));
    doc.insert("bindings", Item::Table(bindings));
    doc.insert("fixes", Item::Table(fixes));
    doc.to_string()
}

/// `text` as `#` comment lines wrapped at 88 columns, ending with a newline;
/// a blank line between sections.
fn comment_lines(text: &str) -> String {
    let mut out = String::new();
    for para in text.split("\n\n") {
        if !out.is_empty() {
            out.push('\n');
        }
        let mut line = String::from("#");
        for word in para.split_whitespace() {
            if line.len() + 1 + word.len() > 88 && line.len() > 1 {
                out += &line;
                out.push('\n');
                line = String::from("#");
            }
            line.push(' ');
            line += word;
        }
        out += &line;
        out.push('\n');
    }
    out
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
    fn an_older_file_is_brought_up_to_date_keeping_its_values() {
        let d = dir("old");
        let path = d.join("config.toml");
        // An older layout: no skip_loading, other comments.
        std::fs::write(&path, "# old\n[display]\nresolution = \"1280x960\"\nfullscreen = false\n[bindings]\nstart = [\"Return\"]\n").unwrap();
        let (c, warnings) = load(&path);
        assert!(warnings.is_empty(), "{warnings:?}");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# Bugatron settings"), "{text}");
        assert!(text.contains("resolution = \"1280x960\"") && text.contains("fullscreen = false") && text.contains("skip_loading = true"), "{text}");
        assert!(text.contains("start = [\"Return\"]"), "{text}");
        assert_eq!(c.display.resolution, Some((1280, 960)));
        // Loading it again changes nothing.
        load(&path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        // A file with problems is left as it is, for the user to fix.
        let broken = "[display]\nresolution = \"huge\"\n";
        std::fs::write(&path, broken).unwrap();
        let (_, warnings) = load(&path);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn settings_override_defaults_and_unknown_ones_are_reported() {
        let d = dir("edit");
        let path = d.join("config.toml");
        std::fs::write(&path, "[display]\nresolution = \"1280x960\"\nfullscreen = false\nskip_loading = false\n[fixes]\ndifficulty_label = false\nwings = true\n[bindings]\nstart = [\"Return\", \"Pad y\"]\nfly = [\"F9\"]\nexit = [\"Escape\", \"Nope\"]\n").unwrap();
        let (mut c, warnings) = load(&path);
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(!c.fixes.difficulty_label);
        assert_eq!(c.display, Display { resolution: Some((1280, 960)), fullscreen: false, skip_loading: false });
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
