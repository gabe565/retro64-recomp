//! Bugatron Worlds, statically recompiled: the launcher (bugatron-launcher) with
//! the game's profile. Options: `bugatron-worlds --help`.

mod profile;

// The translated game (bugatron-build), linked into this program only.
#[link(name = "game_gen", kind = "static", modifiers = "+whole-archive")]
extern "C" {}

/// The game folder built into this program (`--features embed-game`; empty without it).
mod embedded {
    include!(concat!(env!("OUT_DIR"), "/pack.rs"));
}

fn main() {
    bugatron_launcher::run(&bugatron_launcher::Title {
        game: &profile::TRIAL,
        title: "Bugatron Worlds v1.01a (free trial)",
        bin: "bugatron-worlds",
        exe_env: "BUGATRON_WORLDS_EXE",
        sources: &["BugatronWorlds.exe", "bugatron_worlds.exe"],
        install_folders: &["Bugatron Worlds"],
        sha256: env!("GAME_EXE_SHA256"),
        built_from: env!("GAME_BUILT_FROM"),
        pack: embedded::PACK,
        delta: None,
    })
}
