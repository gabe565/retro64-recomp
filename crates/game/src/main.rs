//! Bugatron Gold v1.20a, statically recompiled: the launcher (bugatron-launcher)
//! with Bugatron's profile. Options: `bugatron --help`.

mod logo;
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
        game: &profile::BUGATRON,
        title: "Bugatron Gold v1.20a",
        bin: "bugatron",
        exe_env: "BUGATRON_EXE",
        sources: &["Bug3D.exe", "buggold_120.exe", "Bugatron_Win_EN_Gold_120.zip"],
        install_folders: &["Bugatron Gold"],
        sha256: env!("GAME_EXE_SHA256"),
        built_from: env!("GAME_BUILT_FROM"),
        pack: embedded::PACK,
        // The frame delta (1/1024 s ticks) and tick_now.
        delta: Some((0x8429e0, 0x8429dc)),
    })
}

#[cfg(test)]
mod tests {
    use bugatron_launcher::source::open;
    use sha2::Digest;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// The download and the installer unpack to the same 408 files as innoextract,
    /// with the right Bug3D.exe. Skipped without the original files.
    #[test]
    fn download_and_installer_unpack_to_the_game_folder() {
        let installed = root().join("_work/ext/gold120/app");
        for p in [root().join("reference/Bugatron_Win_EN_Gold_120.zip"), root().join("_work/zips/buggold_120.exe")] {
            if !p.exists() || !installed.exists() {
                eprintln!("{} not present: skipping", p.display());
                continue;
            }
            let g = open(&p, "Bug3D.exe").unwrap();
            let sha: String = sha2::Sha256::digest(&g.exe).iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(sha, env!("GAME_EXE_SHA256"));
            let pack = g.pack.expect("files in memory");
            assert_eq!(pack.len(), 408);
            for (rel, data) in pack {
                assert_eq!(&std::fs::read(installed.join(rel)).unwrap()[..], *data, "{rel}");
            }
        }
    }
}
