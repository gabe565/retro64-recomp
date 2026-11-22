//! Bugatron Worlds' profiles for the runtime (bugatron_runtime::game), one per
//! release. Only the v1.01a free trial is known to survive; its time limit (30
//! minutes a session, counted down on screen) is the game's own and is left as it
//! is.
//!
//! Worlds is a later version of Bugatron's engine (Visual C++ 2003, its images
//! through DevIL and its files in smash.wad). It always runs at 800x600, choosing
//! that mode itself; its setup dialog only appears when the mode is missing.

use bugatron_runtime::bindings::{sc, Action};
use bugatron_runtime::game::Game;

/// Bugatron Worlds v1.01a, the free trial.
pub const TRIAL: Game = Game {
    name: "Bugatron Worlds",
    exe_name: "BugatronWorlds.exe",
    install_dir: "C:\\Bugatron Worlds",
    save_folder: "bugatron-worlds",
    // The key table (0x43d0f8, read by the input poll 0x41b700) is Bugatron's, key
    // for key. Its menus are driven with the mouse; F2 opens the high scores. F3
    // and Delete are read as in Bugatron but not yet checked.
    actions: &[
        Action { id: "left", label: "Move left", key: sc::LEFT, defaults: &["Left"] },
        Action { id: "right", label: "Move right", key: sc::RIGHT, defaults: &["Right"] },
        Action { id: "up", label: "Move up", key: sc::UP, defaults: &["Up"] },
        Action { id: "down", label: "Move down", key: sc::DOWN, defaults: &["Down"] },
        // Space also fires, but stays itself: high-score names read it.
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
    ..Game::NONE
};
