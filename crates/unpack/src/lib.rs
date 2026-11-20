//! Unpacking a game from what the user has: its executable itself, its original
//! Inno Setup installer, or a zip holding either (or the game folder). Shared by
//! the launcher and the games' build scripts, so it depends only on the `inno` and
//! `zip` crates. `exe` is the executable's file name (`Bug3D.exe`), matched
//! case-insensitively.

use std::io::{Cursor, Read};

/// The game's files, relative to the game folder with `/` separators.
pub type Files = Vec<(String, Vec<u8>)>;

/// What `bytes` holds.
pub enum Unpacked {
    /// The executable on its own (its folder holds the other files).
    Exe(Vec<u8>),
    /// The whole game folder, the executable included.
    Folder(Files),
}

/// The executable `exe`, an installer, or a zip holding either; `label` names it in errors.
pub fn unpack(bytes: Vec<u8>, label: &str, exe: &str) -> Result<Unpacked, String> {
    if bytes.starts_with(b"PK\x03\x04") {
        return from_zip(&bytes, label, exe).map(Unpacked::Folder);
    }
    if !bytes.starts_with(b"MZ") {
        return Err(format!("{label} is neither {exe}, the game's installer, nor a zip"));
    }
    match from_installer(&bytes, label) {
        Some(files) => files.map(Unpacked::Folder),
        None => Ok(Unpacked::Exe(bytes)),
    }
}

/// The files of the Inno Setup installer, or None if `bytes` is not one.
fn from_installer(bytes: &[u8], label: &str) -> Option<Result<Files, String>> {
    let mut inno = inno::Inno::new(Cursor::new(bytes)).ok()?;
    let mut files = Vec::new();
    for r in inno.files() {
        let (entry, data) = match r {
            Ok(x) => x,
            Err(e) => return Some(Err(format!("{label}: cannot unpack the installer: {e}"))),
        };
        // Only what installs into the game folder ({app}); other entries are setup's own.
        let Some(rel) = entry.file().destination().and_then(|d| d.strip_prefix("{app}\\")) else { continue };
        files.push((rel.replace('\\', "/"), data));
    }
    Some(Ok(files))
}

/// A zip holding the installer, or a game folder with the executable.
fn from_zip(bytes: &[u8], label: &str, exe: &str) -> Result<Files, String> {
    let bad = |e: zip::result::ZipError| format!("{label}: {e}");
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(bad)?;
    let mut files = Vec::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(bad)?;
        if f.is_dir() {
            continue;
        }
        let name = f.name().map_err(bad)?.replace('\\', "/");
        let mut data = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut data).map_err(|e| format!("{label}: {e}"))?;
        files.push((name, data));
    }
    // The download holds only the installer.
    for (name, data) in &files {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".exe") && !lower.ends_with(&exe.to_ascii_lowercase()) {
            if let Some(r) = from_installer(data, &format!("{label}: {name}")) {
                return r;
            }
        }
    }
    // Otherwise a zipped game folder: files relative to the folder holding the executable.
    let root = files
        .iter()
        .find_map(|(n, _)| n.rsplit('/').next().is_some_and(|b| b.eq_ignore_ascii_case(exe)).then(|| n[..n.len() - exe.len()].to_string()))
        .ok_or_else(|| format!("{label} holds neither the game's installer nor {exe}"))?;
    Ok(files.into_iter().filter_map(|(n, d)| Some((n.strip_prefix(&root)?.to_string(), d))).collect())
}

/// The executable `exe` in a game folder.
pub fn exe_of<'a>(files: &'a Files, exe: &str) -> Option<&'a [u8]> {
    files.iter().find(|(p, _)| p.eq_ignore_ascii_case(exe)).map(|(_, d)| &d[..])
}
