//! Z-Ball Gold, statically recompiled: the launcher (bugatron-launcher) with
//! Z-Ball's profile. Options: `zball --help`.

mod profile;

/// The game folder built into this program (`--features embed-game`; empty without it).
mod embedded {
    include!(concat!(env!("OUT_DIR"), "/pack.rs"));
}

fn main() {
    bugatron_launcher::run(&bugatron_launcher::Title {
        game: &profile::ZBALL,
        title: "Z-Ball Gold",
        bin: "zball",
        exe_env: "ZBALL_EXE",
        sources: &["Z-Ball.exe", "Z-Ball_Win_EN_Full-Rip.zip"],
        install_folders: &["Z-Ball Gold"],
        sha256: env!("GAME_EXE_SHA256"),
        built_from: env!("GAME_BUILT_FROM"),
        pack: embedded::PACK,
        delta: None,
    })
}
