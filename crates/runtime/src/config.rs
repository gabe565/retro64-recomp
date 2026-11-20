//! `config.toml` in the save folder: the runtime's settings. Settings for features
//! the game's profile lacks (game.rs) are left out. The first run writes
//! it with the defaults and comments; the settings overlay rewrites it. Scripted
//! runs (`--input`) ignore it.
//!
//! - `[display]`: resolution and fullscreen (command-line options override them).
//! - `[bindings]`: host keys and controller buttons for the game's keys (bindings.rs).
//! - `[fixes]`: corrections of bugs in the original game, each on by default; set
//!   one to `false` for the original's behaviour.

use crate::bindings::Bindings;
use crate::game::Game;
use crate::Runtime;
use std::path::{Path, PathBuf};

/// Corrections of bugs in the original game, made by the runtime (the game's code
/// is unchanged).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fixes {
    /// The title screen's difficulty label follows the actual difficulty
    /// (overlay::fix_difficulty_label; the profile's `Difficulty::title_label`).
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
    /// The title logo is the profile's alternative (`Game::logo`).
    pub alternative_logo: bool,
}

impl Default for Display {
    fn default() -> Display {
        Display { resolution: None, fullscreen: true, skip_loading: true, alternative_logo: false }
    }
}

/// Everything `config.toml` holds.
pub struct Config {
    pub display: Display,
    pub fixes: Fixes,
    pub bindings: Bindings,
}

impl Config {
    /// The defaults for `game`.
    pub fn new(game: &Game) -> Config {
        Config { display: Display::default(), fixes: Fixes::default(), bindings: Bindings::new(game.actions) }
    }
}

pub fn path(rt: &Runtime) -> PathBuf {
    rt.files.save_dir.join("config.toml")
}

/// Read `config.toml`; a missing file is written with the defaults, and a file that
/// differs from what this version writes (older comments, settings added since) is
/// rewritten with its values. A file with problems is left for the user to fix:
/// they are returned as warnings and the defaults used for what could not be read.
pub fn load(path: &Path, game: &Game) -> (Config, Vec<String>) {
    let mut warnings = Vec::new();
    let existing = std::fs::read_to_string(path).ok();
    let table: toml::Table = match &existing {
        Some(text) => toml::from_str(text).unwrap_or_else(|e| {
            warnings.push(format!("{e}"));
            toml::Table::new()
        }),
        None => {
            if let Err(e) = save(&Config::new(game), path, game) {
                warnings.push(format!("cannot write the defaults: {e}"));
            }
            toml::Table::new()
        }
    };
    let mut display = Display::default();
    for (k, v) in table.get("display").and_then(|v| v.as_table()).into_iter().flatten() {
        match (k.as_str(), v) {
            ("fullscreen", toml::Value::Boolean(b)) => display.fullscreen = *b,
            ("skip_loading", toml::Value::Boolean(b)) if game.screens.is_some() => display.skip_loading = *b,
            ("logo", toml::Value::String(l)) if game.logo.as_ref().is_some_and(|g| *l == g.original.0 || *l == g.alternative.0) => {
                display.alternative_logo = game.logo.as_ref().is_some_and(|g| *l == g.alternative.0)
            }
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
            ("difficulty_label", Some(b)) if title_label(game).is_some() => fixes.difficulty_label = b,
            _ => warnings.push(format!("fixes: unknown setting {k} = {v}")),
        }
    }
    let bindings = Bindings::from_table(game.actions, table.get("bindings").and_then(|v| v.as_table()), &mut warnings);
    let config = Config { display, fixes, bindings };
    if existing.as_ref().is_some_and(|text| warnings.is_empty() && *text != render(&config, game)) {
        if let Err(e) = save(&config, path, game) {
            warnings.push(format!("cannot update it: {e}"));
        }
    }
    let warnings = warnings.into_iter().map(|w| format!("{}: {w}", path.display())).collect();
    (config, warnings)
}

/// Write `config.toml`.
pub fn save(c: &Config, path: &Path, game: &Game) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, render(c, game))
}

fn title_label(game: &Game) -> Option<&crate::game::TitleLabel> {
    game.difficulty.as_ref()?.title_label.as_ref()
}

/// The text of `config.toml` for `c`: every setting, each with a comment.
fn render(c: &Config, game: &Game) -> String {
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
        &format!(
            "{} settings. The game rewrites this file when the settings overlay (F10) changes \
             them, and on start when it was written by an older version; edit it while the game is closed.\n\n\
             [display]: --video-mode and --windowed override these.",
            game.name
        ),
        true,
    );
    let d = &c.display;
    let res = d.resolution.map_or("auto".to_string(), |(w, h)| format!("{w}x{h}"));
    set(&mut display, "resolution", value(res), "\"auto\" (the largest 4:3 mode that fits the display) or \"WIDTHxHEIGHT\"; takes effect when the game starts.");
    set(&mut display, "fullscreen", value(d.fullscreen), "Fullscreen, or a window.");
    if game.screens.is_some() {
        set(&mut display, "skip_loading", value(d.skip_loading), "Go straight from one screen to the next. The original holds an empty \"LOADING\" screen between them for most of a second.");
    }
    if let Some(logo) = &game.logo {
        let v = if d.alternative_logo { logo.alternative.0 } else { logo.original.0 };
        set(&mut display, "logo", value(v), logo.comment);
    }

    let mut bindings = section(
        "[bindings]: for each of the game's keys, the host keys and controller buttons that press it. \
         Keys are SDL key names (\"PageUp\", \"Left Ctrl\", \"=\"); buttons are \"Pad \" and an SDL button name \
         (\"Pad start\", \"Pad leftshoulder\"). Keys bound to nothing type themselves (high-score names); \
         a game key removed from its own list stops working.",
        false,
    );
    if !game.bindings_note.is_empty() {
        let prefix = bindings.decor().prefix().and_then(|p| p.as_str()).unwrap_or_default().trim_end().to_string();
        bindings.decor_mut().set_prefix(format!("{prefix}\n{}", comment_lines(game.bindings_note)));
    }
    for (id, label, names) in c.bindings.entries() {
        set(&mut bindings, id, value(names.into_iter().collect::<Array>()), label);
    }

    let mut doc = DocumentMut::new();
    doc.insert("display", Item::Table(display));
    doc.insert("bindings", Item::Table(bindings));
    if let Some(label) = title_label(game) {
        let mut fixes = section("[fixes]: corrections of bugs in the original game; false keeps the original's behaviour.", false);
        set(&mut fixes, "difficulty_label", value(c.fixes.difficulty_label), label.fix_comment);
        doc.insert("fixes", Item::Table(fixes));
    }
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
    let (c, warnings) = load(&path(rt), rt.game);
    for w in warnings {
        eprintln!("config: {w}");
    }
    rt.display = c.display;
    crate::overlay::apply_logo(rt);
    rt.fixes = c.fixes;
    rt.user.bindings = c.bindings;
}

/// Save the runtime's settings (after the overlay changes them).
pub fn save_from(rt: &mut Runtime) {
    let c = Config { display: rt.display.clone(), fixes: rt.fixes.clone(), bindings: std::mem::take(&mut rt.user.bindings) };
    let path = path(rt);
    if let Err(e) = save(&c, &path, rt.game) {
        eprintln!("config: {}: {e}", path.display());
    }
    rt.user.bindings = c.bindings;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::{sc, Action, Input};
    use crate::game::{Difficulty, Logo, Screens, TitleLabel};

    /// A profile with every setting (addresses unused).
    const GAME: Game = Game {
        name: "Testgame",
        actions: &[
            Action { id: "start", label: "Start game", key: sc::F1, defaults: &["F1"] },
            Action { id: "music_up", label: "Music volume up", key: sc::PAGE_UP, defaults: &["PageUp", "="] },
            Action { id: "music_down", label: "Music volume down", key: sc::PAGE_DOWN, defaults: &["PageDown", "-"] },
            Action { id: "exit", label: "Exit", key: sc::ESCAPE, defaults: &["Escape"] },
        ],
        screens: Some(Screens {
            state: 0,
            running: 5,
            fade_out: 3,
            next_screen: 0,
            high_scores: None,
            fade_start: 0,
            fade_history: [0; 3],
            fade_ticks: 2000.0,
            fade_history_start: 5.0,
            tick_now: 0,
            camera_y: 0,
            sign_height: 16.0,
            skip_at: 1450.0,
            enter_from: 12.0,
        }),
        difficulty: Some(Difficulty {
            value: 0,
            levels: &[(4.0, "NORMAL")],
            title_label: Some(TitleLabel {
                title_menu: 0,
                text_list: 0,
                letter_size: 164,
                letter_count: 0,
                starts_with: *b"F2",
                labels: &[b"NORMAL"],
                fix_comment: "The title shows the difficulty actually set.",
            }),
        }),
        logo: Some(Logo {
            path: &["m", "logo.bmp"],
            original: ("gold", "GOLD"),
            alternative: ("plain", "PLAIN"),
            comment: "The title logo.",
            edit: |_| None,
        }),
        ..Game::NONE
    };

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
        let (c, warnings) = load(&path, &GAME);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(c.fixes, Fixes::default());
        assert_eq!(c.display, Display::default());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# Testgame settings"), "{text}");
        assert!(text.contains("[fixes]") && text.contains("difficulty_label = true"), "{text}");
        assert!(text.contains("# Music volume up\nmusic_up = [\"PageUp\", \"=\"]"), "{text}");
        let (again, warnings) = load(&path, &GAME);
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
        let (c, warnings) = load(&path, &GAME);
        assert!(warnings.is_empty(), "{warnings:?}");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# Testgame settings"), "{text}");
        assert!(text.contains("resolution = \"1280x960\"") && text.contains("fullscreen = false") && text.contains("skip_loading = true"), "{text}");
        assert!(text.contains("start = [\"Return\"]"), "{text}");
        assert_eq!(c.display.resolution, Some((1280, 960)));
        // Loading it again changes nothing.
        load(&path, &GAME);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        // A file with problems is left as it is, for the user to fix.
        let broken = "[display]\nresolution = \"huge\"\n";
        std::fs::write(&path, broken).unwrap();
        let (_, warnings) = load(&path, &GAME);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn settings_override_defaults_and_unknown_ones_are_reported() {
        let d = dir("edit");
        let path = d.join("config.toml");
        std::fs::write(&path, "[display]\nresolution = \"1280x960\"\nfullscreen = false\nskip_loading = false\nlogo = \"plain\"\n[fixes]\ndifficulty_label = false\nwings = true\n[bindings]\nstart = [\"Return\", \"Pad y\"]\nfly = [\"F9\"]\nexit = [\"Escape\", \"Nope\"]\n").unwrap();
        let (mut c, warnings) = load(&path, &GAME);
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(!c.fixes.difficulty_label);
        assert_eq!(c.display, Display { resolution: Some((1280, 960)), fullscreen: false, skip_loading: false, alternative_logo: true });
        assert_eq!(c.bindings.press(key("Return"), true, false), [(58, true, false)]);
        assert_eq!(c.bindings.press(key("F1"), true, false), [], "F1 no longer starts a game");
        assert_eq!(c.bindings.press(key("PageDown"), true, false), [(78, true, false)], "defaults kept");
        save(&c, &path, &GAME).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("difficulty_label = false") && text.contains("# Start game\nstart = [\"Return\", \"Pad y\"]"), "{text}");
        let (again, warnings) = load(&path, &GAME);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(again.bindings.inputs, c.bindings.inputs);
        assert_eq!(again.fixes, c.fixes);
        assert_eq!(again.display, c.display);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn settings_for_features_the_game_lacks_are_left_out() {
        let text = render(&Config::new(&Game::NONE), &Game::NONE);
        assert!(text.starts_with("# Game settings"), "{text}");
        assert!(!text.contains("skip_loading") && !text.contains("logo") && !text.contains("[fixes]"), "{text}");
        assert!(text.contains("settings = [\"F10\", \"Pad back\"]"), "{text}");
    }
}
