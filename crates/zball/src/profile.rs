//! Z-Ball Gold's profile for the runtime (bugatron_runtime::game). Z-Ball runs on
//! an earlier version of Bugatron's engine: the same key table, pause, volume and
//! text code at other addresses, found by matching it with Bugatron's.
//!
//! Its screens are a list it walks rather than Bugatron's fade-out and loading
//! states (`ScreenList`); it has no loading screen to skip.

use bugatron_runtime::bindings::{sc, Action};
use bugatron_runtime::game::{Difficulty, Font, Game, ScreenList, Volumes};

pub const ZBALL: Game = Game {
    name: "Z-Ball",
    exe_name: "Z-Ball.exe",
    install_dir: "C:\\Z-Ball",
    save_folder: "zball",
    // The keys the input poll reads (table 0x437bb8, as Bugatron's 0x437f9c less
    // Space; flags at 0x743258..). The paddle, on the right edge, follows the mouse
    // vertically; Up and Down move it too. Left and Right are read but do nothing.
    actions: &[
        Action { id: "up", label: "Move up", key: sc::UP, defaults: &["Up"] },
        Action { id: "down", label: "Move down", key: sc::DOWN, defaults: &["Down"] },
        Action { id: "launch", label: "Launch ball", key: sc::LCTRL, defaults: &["Left Ctrl", "Right Ctrl"] },
        Action { id: "start", label: "Start game", key: sc::F1, defaults: &["F1"] },
        Action { id: "high_scores", label: "High scores", key: sc::F2, defaults: &["F2"] },
        Action { id: "difficulty", label: "Difficulty", key: sc::F3, defaults: &["F3"] },
        Action { id: "pause", label: "Pause", key: sc::P, defaults: &["P", "Pad start"] },
        Action { id: "exit", label: "Exit", key: sc::ESCAPE, defaults: &["Escape"] },
        Action { id: "music_up", label: "Music volume up", key: sc::PAGE_UP, defaults: &["PageUp", "="] },
        Action { id: "music_down", label: "Music volume down", key: sc::PAGE_DOWN, defaults: &["PageDown", "-"] },
        Action { id: "sound_up", label: "Sound volume up", key: sc::HOME, defaults: &["Home", "]"] },
        Action { id: "sound_down", label: "Sound volume down", key: sc::END, defaults: &["End", "["] },
        Action { id: "screenshot", label: "Screenshot", key: sc::F12, defaults: &["F12"] },
        Action { id: "frame_time", label: "Show FPS", key: sc::F11, defaults: &["F11"] },
    ],
    bindings_note: "",
    // The title font, as Bugatron's ('/' and ':' are named "^" and "`").
    font: Some(Font { pattern: "Fonts\\nj{}.bmp", renamed: &[('/', "^"), (':', "`")] }),
    // The P key's flag: the frame function (0x40f050) returns at once, so nothing
    // updates or is drawn; the music plays on.
    pause: Some(0x756958),
    screens: None,
    // One node per screen, built from CONFIG.XML.lev (0x419430): the intro, the
    // title, the menus and every level of every set, then "LEVEL SET COMPLETED!"
    // and last the high scores (name entry when there is a new one). The main loop
    // (0x40f050) runs a screen in state 5; F2 on the title (0x40de40) sets the
    // current node to the one before the last and starts the fade with the F1 flag
    // (0x40e011: start_fade with ECX = the fade list, then kind 0x20). That needs
    // the title or a menu; the overlay starts the fade itself, so it works in
    // gameplay too, where the game ends as when it is lost.
    screen_list: Some(ScreenList {
        current: 0x721b68,
        last: 0x721b70,
        state: 0x756990,
        running: 5,
        fades: 0x756900,
        start_fade: 0x405340,
        fade_kind: 0x20,
    }),
    // F3 (0x40dee0, in 0x40de40) cycles Normal → Hard → Easy: the ball speed 0x4370f0 is the
    // base 0x4370ec (0.0095) times 1, 1.5 (0x4352dc) or 0.66 (0x4353d8), the
    // float products below. It copies the name to 0x8a1090, which the title
    // screen's "F2 ... F3: DIFFICULTY - NORMAL" line shows (rewritten every frame,
    // so unlike Bugatron's it stays right).
    difficulty: Some(Difficulty {
        value: 0x4370f0,
        levels: &[(f32::from_bits(0x3c1ba5e3), "NORMAL"), (f32::from_bits(0x3c6978d4), "HARD"), (f32::from_bits(0x3bcd7493), "EASY")],
        labels: &[b"NORMAL", b"HARD  ", b"EASY  "],
        label_string: Some(0x8a1090),
        title_label: None,
    }),
    // Defaults 55 and 80, changed in steps of 10 (1..=100) by Page Up/Down and
    // Home/End (in the screen function 0x40de40), each change applied by
    // 0x4160b0(music, sound, 0) while sound is on.
    volumes: Some(Volumes { music: 0x4370b8, sound: 0x4370bc, apply: 0x4160b0, music_offset: 0, sound_on: 0x437e40 }),
    // Toggled by F11: FPS, polygons, and logic and render times in milliseconds.
    fps_flag: Some(0x756924),
    logo: None,
};
