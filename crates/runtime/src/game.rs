//! What the runtime knows about the game it runs: its names and paths, its keys,
//! and where the game keeps the state the runtime's modern features read or set
//! (the settings overlay, fixes, asset edits).
//!
//! The game's binary supplies its profile (`Runtime::game`). Every feature group
//! is optional: without it, the feature is off for that game (no overlay row, no
//! `config.toml` setting). Addresses are the game's own, from its analysis record.

use crate::bindings::Action;

pub struct Game {
    /// For messages, the config file and the overlay ("QUIT <NAME>").
    pub name: &'static str,
    /// The executable's file name and the folder the game sees itself installed in.
    pub exe_name: &'static str,
    pub install_dir: &'static str,
    /// The per-user save folder's name (files.rs `default_save_dir`).
    pub save_folder: &'static str,
    /// The keys the game reads, with their default bindings (bindings.rs). The
    /// runtime adds its own action to open the settings overlay.
    pub actions: &'static [Action],
    /// More about the game's keys, for the `[bindings]` comment in `config.toml`
    /// (empty: nothing).
    pub bindings_note: &'static str,
    /// The overlay's font: a 16×16 24-bit glyph bitmap per character, by file name
    /// pattern with `{}` for the character's name (for example `Fonts\\nj{}.bmp`).
    /// Without it the overlay is not available.
    pub font: Option<Font>,
    /// The game's pause flag (nonzero: paused). The overlay pauses the game with it.
    pub pause: Option<u32>,
    pub screens: Option<Screens>,
    pub difficulty: Option<Difficulty>,
    pub volumes: Option<Volumes>,
    /// A flag that shows the game's performance counters (nonzero: shown).
    pub fps_flag: Option<u32>,
    pub logo: Option<Logo>,
}

/// The overlay's font.
pub struct Font {
    pub pattern: &'static str,
    /// Characters whose file name differs from the character, as (character, name).
    pub renamed: &'static [(char, &'static str)],
}

/// The game's screen state machine, for its screen transitions (high scores from
/// the overlay, skipping the loading screen).
pub struct Screens {
    /// The main loop's state; `running` runs a screen, `fade_out` leaves it.
    pub state: u32,
    pub running: u32,
    pub fade_out: u32,
    /// The config the next screen is loaded from, and the high-score screen's.
    pub next_screen: u32,
    pub high_scores: Option<&'static str>,
    /// Fade-out start time and the camera's last three heights during it (floats).
    pub fade_start: u32,
    pub fade_history: [u32; 3],
    /// The fade's length and history start value, in the game's ticks.
    pub fade_ticks: f32,
    pub fade_history_start: f32,
    /// The game's current time in ticks (u32).
    pub tick_now: u32,
    /// The camera's height (float): 0 on a screen, `sign_height` at the loading
    /// sign between screens.
    pub camera_y: u32,
    pub sign_height: f32,
    /// Fade time by which every screen has slid out of view, and the height where
    /// the next screen's camera starts, past the sign (overlay.rs `skip_loading`).
    pub skip_at: f32,
    pub enter_from: f32,
}

/// The difficulty setting.
pub struct Difficulty {
    /// The value (float) and its levels in the order the game cycles them:
    /// (value, name shown in the overlay).
    pub value: u32,
    pub levels: &'static [(f32, &'static str)],
    /// The level names as the game writes them on its title screen (one per
    /// level, in `levels` order; equal lengths).
    pub labels: &'static [&'static [u8]],
    /// Where the game keeps the current label as a string (NUL-terminated), which
    /// it copies onto the title screen itself (Z-Ball).
    pub label_string: Option<u32>,
    /// The title screen's text naming the difficulty, which only the game's
    /// difficulty key updates (Bugatron).
    pub title_label: Option<TitleLabel>,
}

/// A text object on the title screen that ends with the difficulty's name, as the
/// game's own difficulty handler writes it.
pub struct TitleLabel {
    /// Nonzero while the title menu runs.
    pub title_menu: u32,
    /// The screen's text objects (a list linked through each object's first word).
    pub text_list: u32,
    /// Letter records' size (the character first, from record 1) and the letter
    /// count's offset in a text object.
    pub letter_size: u32,
    pub letter_count: u32,
    /// The text object's first two letters (its last letters are the label).
    pub starts_with: [u8; 2],
    /// The `config.toml` comment of the fix (`[fixes] difficulty_label`): what the
    /// original shows instead.
    pub fix_comment: &'static str,
}

/// The music and sound-effect volumes (int, 0..=100).
pub struct Volumes {
    pub music: u32,
    pub sound: u32,
    /// The game's function that applies them: `apply(music + music_offset, sound,
    /// 0)`, cdecl floats, called only while sound is on (`sound_on` nonzero).
    pub apply: u32,
    pub music_offset: i32,
    pub sound_on: u32,
}

/// An alternative title logo.
pub struct Logo {
    /// The logo texture's path in the game folder (components).
    pub path: &'static [&'static str],
    /// The `config.toml` values for the original and the alternative, and the
    /// overlay's names for them.
    pub original: (&'static str, &'static str),
    pub alternative: (&'static str, &'static str),
    /// The `config.toml` comment of the setting (`[display] logo`).
    pub comment: &'static str,
    /// Make the alternative from the original file (None: not the expected file).
    pub edit: crate::files::Edit,
}

impl Game {
    /// No game: the runtime's own features only (tests).
    pub const NONE: Game = Game {
        name: "Game",
        exe_name: "game.exe",
        install_dir: crate::files::GAME_DIR,
        save_folder: "game",
        actions: &[],
        bindings_note: "",
        font: None,
        pause: None,
        screens: None,
        difficulty: None,
        volumes: None,
        fps_flag: None,
        logo: None,
    };
}
