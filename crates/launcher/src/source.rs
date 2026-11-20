//! Where the game comes from: the user's `Bug3D.exe` with its folder, the original
//! Inno Setup installer (`buggold_120.exe`), the downloaded zip holding it, or the
//! copy built into the program (`embed-game`). An installer or zip is unpacked in
//! memory at startup (about 0.1 s) and served read-only, like the embedded copy;
//! nothing is written to disk. In every case Bug3D.exe's SHA-256 is checked by the
//! caller.

use crate::unpack::{unpack, Unpacked};
use bugatron_runtime::files::Pack;
use std::path::{Path, PathBuf};

/// The game's executable and files.
pub struct Game {
    /// Bug3D.exe.
    pub exe: Vec<u8>,
    /// Where it came from, for messages.
    pub label: String,
    /// The game folder on disk, or
    pub dir: Option<PathBuf>,
    /// the game files in memory.
    pub pack: Option<Pack>,
}

/// Names the lookup tries in each search folder, in order.
pub const NAMES: &[&str] = &["Bug3D.exe", "buggold_120.exe", "Bugatron_Win_EN_Gold_120.zip"];

/// Open `path`: Bug3D.exe, the installer, or a zip containing either.
pub fn open(path: &Path) -> Result<Game, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let label = path.display().to_string();
    match unpack(bytes, &label)? {
        Unpacked::Exe(exe) => Ok(Game { exe, label, dir: path.parent().map(Path::to_path_buf), pack: None }),
        Unpacked::Folder(files) => {
            let files = files
                .into_iter()
                .map(|(p, d)| (&*Box::leak(p.into_boxed_str()), &*Box::leak(d.into_boxed_slice())))
                .collect();
            from_pack(files, label)
        }
    }
}

fn from_pack(files: Vec<(&'static str, &'static [u8])>, label: String) -> Result<Game, String> {
    let pack: Pack = Box::leak(files.into_boxed_slice());
    let exe = pack
        .iter()
        .find(|(p, _)| p.eq_ignore_ascii_case("Bug3D.exe"))
        .ok_or_else(|| format!("{label} does not contain Bug3D.exe"))?
        .1;
    Ok(Game { exe: exe.to_vec(), label, dir: None, pack: Some(pack) })
}

/// The game built into the program, if any.
pub fn embedded(pack: Pack) -> Option<Game> {
    from_pack(pack.to_vec(), "embedded".into()).ok().filter(|_| !pack.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;

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
            let g = open(&p).unwrap();
            let sha: String = sha2::Sha256::digest(&g.exe).iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(sha, env!("BUGATRON_EXE_SHA256"));
            let pack = g.pack.expect("files in memory");
            assert_eq!(pack.len(), 408);
            for (rel, data) in pack {
                assert_eq!(&std::fs::read(installed.join(rel)).unwrap()[..], *data, "{rel}");
            }
        }
    }

    #[test]
    fn other_files_are_refused() {
        let p = std::env::temp_dir().join(format!("bugatron-src-{}.txt", std::process::id()));
        std::fs::write(&p, b"hello").unwrap();
        assert!(open(&p).err().unwrap().contains("neither Bug3D.exe"));
        let _ = std::fs::remove_file(&p);
    }
}
