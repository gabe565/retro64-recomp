//! Scripted play of the Bugatron Worlds free trial (bugatron-scenario).

use bugatron_scenario::{run, Check::*, Scenario};

/// Past the trial screen (Maybe later), Start Game, Level 1, through How to Play,
/// then steer and fire.
const GAME: &str = "350:click@620x540,1100:click@400x443,1500:click@140x180,2100:click@735x578,2500:click@735x578,2900:click@735x578,3600:CTRL+,3600:LEFT+,3800:LEFT-,3900:RIGHT+,4200:RIGHT-,4400:CTRL-";

fn play(s: &Scenario) {
    run(env!("CARGO_BIN_EXE_bugatron-worlds"), env!("CARGO_MANIFEST_DIR"), s);
}

/// The trial starts at 800x600, its first-start mode (no prefs in a scripted run).
#[test]
fn game_session() {
    play(&Scenario { name: "game", input: GAME, frames: 4500, args: &["--video-mode", "1024x768x32"], checks: &[FrameSize(1000, 800, 600), FrameHash(1000), FrameHash(4300)] });
}

/// Options > Video Prefs restarts the game in the same process; its setup dialog
/// gets the chosen mode.
#[test]
fn video_prefs_changes_the_resolution() {
    play(&Scenario {
        name: "video-prefs",
        input: "350:click@620x540,1100:click@400x495,1700:click@400x352",
        frames: 2600,
        args: &["--video-mode", "1024x768x32"],
        checks: &[FrameSize(1600, 800, 600), FrameSize(2500, 1024, 768), FrameHash(2500)],
    });
}
