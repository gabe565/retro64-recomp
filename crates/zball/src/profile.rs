//! Z-Ball Gold's profile for the runtime (bugatron_runtime::game). Z-Ball runs on
//! the same engine as Bugatron, but its addresses differ and are not yet mapped:
//! until they are, the profile has its keys only, and the settings overlay, which
//! needs the game's pause flag, is not available.

use bugatron_runtime::bindings::{sc, Action};
use bugatron_runtime::game::Game;

pub const ZBALL: Game = Game {
    name: "Z-Ball",
    exe_name: "Z-Ball.exe",
    install_dir: "C:\\Z-Ball",
    save_folder: "zball",
    // The keyboard controls in its README and on its title screen. The paddle, on
    // the right edge, follows the mouse vertically; Up and Down move it too.
    actions: &[
        Action { id: "up", label: "Move up", key: sc::UP, defaults: &["Up"] },
        Action { id: "down", label: "Move down", key: sc::DOWN, defaults: &["Down"] },
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
    ],
    ..Game::NONE
};
