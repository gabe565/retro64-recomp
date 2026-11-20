//! Unpacking Bugatron Gold from what the user has: `Bug3D.exe` itself, the original
//! Inno Setup installer (`buggold_120.exe`), or the download zip holding either.
//! Shared by the program (source.rs) and its build script (build.rs, via `#[path]`),
//! so it depends only on the `inno` and `zip` crates.

use std::io::{Cursor, Read};

/// The game's files, relative to the game folder with `/` separators.
pub type Files = Vec<(String, Vec<u8>)>;

/// What `bytes` holds.
pub enum Unpacked {
    /// Bug3D.exe on its own (its folder holds the other files).
    Exe(Vec<u8>),
    /// The whole game folder, Bug3D.exe included.
    Folder(Files),
}

/// Bug3D.exe, an installer, or a zip holding either; `label` names it in errors.
pub fn unpack(bytes: Vec<u8>, label: &str) -> Result<Unpacked, String> {
    if bytes.starts_with(b"PK\x03\x04") {
        return from_zip(&bytes, label).map(Unpacked::Folder);
    }
    if !bytes.starts_with(b"MZ") {
        return Err(format!("{label} is neither Bug3D.exe, the Bugatron Gold installer, nor a zip"));
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

/// A zip holding the installer, or a game folder with Bug3D.exe.
fn from_zip(bytes: &[u8], label: &str) -> Result<Files, String> {
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
        if lower.ends_with(".exe") && !lower.ends_with("bug3d.exe") {
            if let Some(r) = from_installer(data, &format!("{label}: {name}")) {
                return r;
            }
        }
    }
    // Otherwise a zipped game folder: files relative to the folder holding Bug3D.exe.
    let root = files
        .iter()
        .find_map(|(n, _)| n.rsplit('/').next().is_some_and(|b| b.eq_ignore_ascii_case("Bug3D.exe")).then(|| n[..n.len() - "Bug3D.exe".len()].to_string()))
        .ok_or_else(|| format!("{label} holds neither the Bugatron Gold installer nor Bug3D.exe"))?;
    Ok(files.into_iter().filter_map(|(n, d)| Some((n.strip_prefix(&root)?.to_string(), d))).collect())
}

/// Bug3D.exe in a game folder (used by build.rs).
#[allow(dead_code)]
pub fn exe_of(files: &Files) -> Option<&[u8]> {
    files.iter().find(|(p, _)| p.eq_ignore_ascii_case("Bug3D.exe")).map(|(_, d)| &d[..])
}
