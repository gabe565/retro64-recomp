//! Where the game comes from: the user's executable (`Bug3D.exe`) with its folder,
//! the original Inno Setup installer, a zip holding either or the game folder, or
//! the copy built into the program (`embed-game`). An installer or zip is unpacked
//! in memory at startup (about 0.1 s) and served read-only, like the embedded copy;
//! nothing is written to disk. In every case the executable's SHA-256 is checked
//! by the caller.

use bugatron_unpack::{unpack, Unpacked};
use bugatron_runtime::files::Pack;
use std::path::{Path, PathBuf};

/// The game's executable and files.
pub struct Game {
    /// The executable.
    pub exe: Vec<u8>,
    /// Where it came from, for messages.
    pub label: String,
    /// The game folder on disk, or
    pub dir: Option<PathBuf>,
    /// the game files in memory.
    pub pack: Option<Pack>,
}

/// Open `path`: the executable `exe`, the installer, or a zip containing either.
pub fn open(path: &Path, exe: &str) -> Result<Game, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let label = path.display().to_string();
    match unpack(bytes, &label, exe)? {
        Unpacked::Exe(exe) => Ok(Game { exe, label, dir: path.parent().map(Path::to_path_buf), pack: None }),
        Unpacked::Folder(files) => {
            let files = files
                .into_iter()
                .map(|(p, d)| (&*Box::leak(p.into_boxed_str()), &*Box::leak(d.into_boxed_slice())))
                .collect();
            from_pack(files, label, exe)
        }
    }
}

fn from_pack(files: Vec<(&'static str, &'static [u8])>, label: String, exe: &str) -> Result<Game, String> {
    let pack: Pack = Box::leak(files.into_boxed_slice());
    let data = pack.iter().find(|(p, _)| p.eq_ignore_ascii_case(exe)).ok_or_else(|| format!("{label} does not contain {exe}"))?.1;
    Ok(Game { exe: data.to_vec(), label, dir: None, pack: Some(pack) })
}

/// The game built into the program, if any.
pub fn embedded(pack: Pack, exe: &str) -> Option<Game> {
    from_pack(pack.to_vec(), "embedded".into(), exe).ok().filter(|_| !pack.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_files_are_refused() {
        let p = std::env::temp_dir().join(format!("launcher-src-{}.txt", std::process::id()));
        std::fs::write(&p, b"hello").unwrap();
        assert!(open(&p, "Game.exe").err().unwrap().contains("neither Game.exe"));
        let _ = std::fs::remove_file(&p);
    }
}

