//! Scripted play of Bugatron Gold v1.20a (bugatron-scenario).

use bugatron_scenario::{run, Check::*, Scenario};

const DIFFICULTY: u32 = 0x4370e4;
const MUSIC: u32 = 0x4370a8;
const NEXT_SCREEN: u32 = 0x81d548;

fn play(s: &Scenario) {
    run(env!("CARGO_BIN_EXE_bugatron"), env!("CARGO_MANIFEST_DIR"), s);
}

/// The reference session: start a game from the title, fire, and steer left and
/// right every 100 flips. Its frame 2000 is the one compared with the original.
#[test]
fn reference_session() {
    let mut input = vec!["600:click@70x120".to_string(), "800:click@70x120".into(), "1000:CTRL+".into()];
    for f in (1000..2100).step_by(200) {
        input.extend([format!("{f}:LEFT+"), format!("{}:LEFT-", f + 45), format!("{}:RIGHT+", f + 100), format!("{}:RIGHT-", f + 145)]);
    }
    play(&Scenario { name: "reference", input: &input.join(","), frames: 2100, args: &["--video-mode", "640x480x32"], checks: &[FrameSize(2000, 640, 480), FrameHash(2000)] });
}

/// The title music plays (recorded unheard).
#[test]
fn title_music() {
    play(&Scenario { name: "music", input: "1:A", frames: 600, args: &[], checks: &[Audio(5.0)] });
}

/// The settings overlay sets the difficulty (Normal → Hard) and the music volume
/// (90 → 85) on the title screen.
#[test]
fn overlay_difficulty_and_music() {
    play(&Scenario {
        name: "overlay",
        input: "500:SETTINGS[DOWN RIGHT DOWN LEFT ESC]",
        frames: 600,
        args: &["--video-mode", "640x480x32"],
        checks: &[Peek(490, DIFFICULTY, 4.0f32.to_bits()), Peek(510, DIFFICULTY, 1.0f32.to_bits()), Peek(510, MUSIC, 85)],
    });
}

/// High scores from the overlay, mid-game: the game leaves for highscores.xml.
#[test]
fn overlay_high_scores_mid_game() {
    play(&Scenario {
        name: "high-scores",
        input: "600:click@70x120,800:click@70x120,1100:SETTINGS[DOWN DOWN DOWN DOWN DOWN DOWN DOWN DOWN DOWN ENTER]",
        frames: 1500,
        args: &["--video-mode", "640x480x32"],
        checks: &[Peek(1110, NEXT_SCREEN, u32::from_le_bytes(*b"high")), FrameHash(1400)],
    });
}
