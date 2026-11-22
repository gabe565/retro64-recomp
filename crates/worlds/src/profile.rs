//! Bugatron Worlds' profiles for the runtime (bugatron_runtime::game), one per
//! release. Only the v1.01a free trial is known to survive; its time limit (30
//! minutes a session, counted down on screen) is the game's own and is left as it
//! is.
//!
//! Worlds is a later version of Bugatron's engine (Visual C++ 2003, its images
//! through DevIL and its files in smash.wad). Its setup dialog, which the runtime
//! answers with the chosen display mode, appears when the mode saved in its prefs
//! is missing, and from Options > Video Prefs (which restarts it in the same
//! process). On its first start, with no prefs, it picks the mode itself: 800x600,
//! or 1280x1024 when a display driver's name has "radeon", "geforce" or "xabre"
//! in it (0x406470). So the first start gets prefs whose mode is missing.

use bugatron_runtime::bindings::{sc, Action};
use bugatron_runtime::game::Game;

/// The first start's prefs (see `first_run_saves`).
const PREFS: [u8; 32] = {
    let v: [u32; 8] = [0, 1, 31, 1_800_000, 0, 0, 90, 25];
    let mut b = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        b[i] = (v[i / 4] >> (8 * (i % 4))) as u8;
        i += 1;
    }
    b
};

/// Bugatron Worlds v1.01a, the free trial.
pub const TRIAL: Game = Game {
    name: "Bugatron Worlds",
    exe_name: "BugatronWorlds.exe",
    install_dir: "C:\\Bugatron Worlds",
    save_folder: "bugatron-worlds",
    // prefs_bg.dll (read by 0x406470): driver, device and mode indices, trial time
    // left (ms), two flags, music and sound volumes, as u32s. Mode 31 is an empty
    // slot of the mode table, so the setup dialog appears; the rest is what the
    // game uses without the file (a full trial, volumes 90 and 25).
    first_run_saves: &[("prefs_bg.dll", &PREFS)],
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

#[cfg(test)]
mod tests {
    use super::PREFS;

    #[test]
    fn first_run_prefs_ask_for_the_setup_dialog_with_a_full_trial() {
        let v: Vec<u32> = PREFS.chunks(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect();
        assert_eq!(v, [0, 1, 31, 1_800_000, 0, 0, 90, 25]);
    }
}
