//! Bugatron Gold v1.20a's profile for the runtime (bugatron_runtime::game): its
//! keys, and where it keeps the state the settings overlay and fixes use.
//! Addresses are documented in docs/ (function-map, initial-investigation).

use bugatron_runtime::bindings::{sc, Action};
use bugatron_runtime::game::{Difficulty, Font, FrameWait, Game, Logo, Screens, TitleLabel, Volumes};

pub const BUGATRON: Game = Game {
    name: "Bugatron",
    exe_name: "Bug3D.exe",
    install_dir: "C:\\Bugatron",
    save_folder: "bugatron",
    first_run_saves: &[],
    // The keys the input poll reads (table 0x437f9c, read by 0x411f70).
    actions: &[
        Action { id: "left", label: "Move left", key: sc::LEFT, defaults: &["Left"] },
        Action { id: "right", label: "Move right", key: sc::RIGHT, defaults: &["Right"] },
        Action { id: "up", label: "Move up", key: sc::UP, defaults: &["Up"] },
        Action { id: "down", label: "Move down", key: sc::DOWN, defaults: &["Down"] },
        // Space also fires, but stays itself: high-score names and "press a key" screens read it.
        Action { id: "fire", label: "Fire", key: sc::LCTRL, defaults: &["Left Ctrl", "Right Ctrl"] },
        Action { id: "start", label: "Start game", key: sc::F1, defaults: &["F1"] },
        Action { id: "high_scores", label: "High scores", key: sc::F2, defaults: &["F2"] },
        Action { id: "difficulty", label: "Difficulty", key: sc::F3, defaults: &["F3"] },
        Action { id: "pause", label: "Pause", key: sc::P, defaults: &["P", "Pad start"] },
        Action { id: "exit", label: "Exit", key: sc::ESCAPE, defaults: &["Escape"] },
        Action { id: "music_up", label: "Music volume up", key: sc::PAGE_UP, defaults: &["PageUp", "="] },
        Action { id: "music_down", label: "Music volume down", key: sc::PAGE_DOWN, defaults: &["PageDown", "-"] },
        Action { id: "sound_up", label: "Sound volume up", key: sc::HOME, defaults: &["Home", "]"] },
        Action { id: "sound_down", label: "Sound volume down", key: sc::END, defaults: &["End", "["] },
        Action { id: "screenshot", label: "Screenshot", key: sc::INSERT, defaults: &["Insert"] },
        Action { id: "frame_time", label: "Show FPS", key: sc::DELETE, defaults: &["Delete"] },
    ],
    bindings_note: "Space always fires and can't be rebound: high-score names use it.",
    // The title font; '/' and ':' are named "^" and "`".
    font: Some(Font { pattern: "Fonts\\nj{}.bmp", renamed: &[('/', "^"), (':', "`")] }),
    // The P key's flag (0x4122f7): the main loop's frame function returns at once
    // (0x40ba8c), so nothing updates or is drawn; the music plays on.
    pause: Some(0x8429c8),
    // The main loop (0x40ba70): state 5 runs a screen (menus, high scores and
    // gameplay alike), 3 fades out and tears it down, 1 loads the next. Ticks are
    // 1/1024 s; the camera object is 0x81d64c, its position 0x81d670...
    screens: Some(Screens {
        state: 0x4370a0,
        running: 5,
        fade_out: 3,
        next_screen: 0x81d548,
        high_scores: Some("highscores.xml"),
        fade_start: 0x437864,
        fade_history: [0x98d4c4, 0x98d4b8, 0x98d4bc],
        fade_ticks: 2000.0,
        fade_history_start: 5.0,
        tick_now: 0x8429dc,
        camera_y: 0x81d674,
        sign_height: 16.0,
        // By 1450 ticks every screen tested has slid out of view; the sign is out
        // of view below about 12.2, and a screen's top edge comes into view at 10.8.
        skip_at: 1450.0,
        enter_from: 12.0,
    }),
    // F3 cycles Normal → Hard → Easy, writing the labels at 0x437e60...
    // The frame wait at the end of 0x41c180: QPC (returning to 0x41c30b), Sleep(0),
    // against the last frame's counter 0x98de28 and the period 0x98de40.
    frame_wait: Some(FrameWait { site: 0x41c30b, last: 0x98de28, period: 0x98de40 }),
    screen_list: None,
    difficulty: Some(Difficulty {
        value: 0x4370e4,
        levels: &[(4.0, "NORMAL"), (1.0, "HARD"), (8.0, "EASY")],
        labels: &[b"NORMAL", b"HARD  ", b"EASY  "],
        label_string: None,
        title_label: Some(TitleLabel {
            title_menu: 0x8429d0,
            text_list: 0x84223c,
            letter_size: 164,
            letter_count: 0x55b9c,
            // The title's second line, "F2: ... F3: DIFFICULTY - NORMAL".
            starts_with: *b"F2",
            fix_comment: "The title screen shows the difficulty actually set. The original says NORMAL again whenever \
                          the title is rebuilt (after a game or the high scores), whatever the difficulty.",
        }),
    }),
    // Defaults 90 and 25, changed in steps of 10 by Page Up/Down and Home/End (0x413377..).
    // The music is applied as volume - 1, so 1 is silent.
    volumes: Some(Volumes { music: 0x4370a8, sound: 0x4370ac, apply: 0x417210, music_offset: -1, sound_on: 0x438144 }),
    // Toggled by Delete (0x412df8): FPS and render and logic times in milliseconds.
    fps_flag: Some(0x8429b8),
    logo: Some(Logo {
        path: &["m", "junk2.bmp"],
        original: ("gold", "GOLD"),
        alternative: ("bugatron", "BUGATRON"),
        comment: "The title logo: \"gold\" (BUGATRON GOLD) or \"bugatron\" (BUGATRON, drawn from the Gold logo).",
        edit: crate::logo::plain_logo,
    }),
};

#[cfg(test)]
mod tests {
    use super::BUGATRON;
    use bugatron_runtime::config;

    #[test]
    fn config_has_every_bugatron_setting() {
        let d = std::env::temp_dir().join(format!("bugatron-profile-{}", std::process::id()));
        let path = d.join("config.toml");
        let (_, warnings) = config::load(&path, &BUGATRON);
        assert!(warnings.is_empty(), "{warnings:?}");
        let text = std::fs::read_to_string(&path).unwrap();
        if let Ok(out) = std::env::var("PROFILE_CONFIG_OUT") {
            std::fs::write(out, &text).unwrap();
        }
        assert!(text.starts_with("# Bugatron settings"), "{text}");
        for want in ["skip_loading = true", "logo = \"gold\"", "difficulty_label = true", "Space always fires", "frame_time = [\"Delete\"]", "settings = [\"F10\", \"Pad back\"]"] {
            assert!(text.contains(want), "{want}: {text}");
        }
        std::fs::remove_dir_all(&d).ok();
    }
}
