//! Scripted play of Z-Ball Gold (bugatron-scenario).

use bugatron_scenario::{run, Check::*, Scenario};

const DIFFICULTY: u32 = 0x4370f0;
const LABEL: u32 = 0x8a1090;
const MUSIC: u32 = 0x4370b8;
const CURRENT_SCREEN: u32 = 0x721b68;
const LAST_SCREEN: u32 = 0x721b70;
/// Normal and Hard: the base ball speed times 1 and 1.5 (profile.rs).
const NORMAL: u32 = 0x3c1ba5e3;
const HARD: u32 = 0x3c6978d4;
/// Start, the first level set, then the paddle by mouse, launch, and Up.
const GAME: &str = "700:F1,1300:F1,1900:move@600x100,2000:click@600x100,2100:move@600x300,2200:UP+,2300:UP-";

fn play(s: &Scenario) {
    run(env!("CARGO_BIN_EXE_zball"), env!("CARGO_MANIFEST_DIR"), s);
}

#[test]
fn game_session() {
    play(&Scenario { name: "game", input: GAME, frames: 2500, args: &["--video-mode", "640x480x32"], checks: &[FrameSize(2400, 640, 480), FrameHash(1400), FrameHash(2400)] });
}

/// The overlay sets the difficulty (with the title's label) and the music volume.
#[test]
fn overlay_difficulty_and_music() {
    play(&Scenario {
        name: "overlay",
        input: "600:SETTINGS[DOWN RIGHT DOWN LEFT ESC]",
        frames: 700,
        args: &["--video-mode", "640x480x32"],
        checks: &[Peek(590, DIFFICULTY, NORMAL), Peek(610, DIFFICULTY, HARD), Peek(610, LABEL, u32::from_le_bytes(*b"HARD")), Peek(610, MUSIC, 50)],
    });
}

/// High scores from the overlay mid-game: the game moves on to its last screen.
#[test]
fn overlay_high_scores_mid_game() {
    play(&Scenario {
        name: "high-scores",
        input: "700:F1,1300:F1,1900:move@600x240,2000:CTRL,2150:SETTINGS[DOWN DOWN DOWN DOWN DOWN DOWN DOWN ENTER]",
        frames: 2600,
        args: &["--video-mode", "640x480x32"],
        checks: &[PeekSame(2450, CURRENT_SCREEN, LAST_SCREEN), FrameHash(2450)],
    });
}
