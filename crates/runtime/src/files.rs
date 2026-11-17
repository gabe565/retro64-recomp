//! Win32 file handles over the host file system.
//!
//! Guest paths (relative, or under `C:\Bugatron\`) resolve case-insensitively
//! against two directories:
//! - `save_dir`: files the game creates or modifies (prefs, high scores);
//! - `data_dir`: the user's original game files, opened read-only.
//!
//! Reads prefer `save_dir`. Opening an original file for writing first copies it
//! into `save_dir` (copy-on-write), so original game files are never modified.
//!
//! Instead of `data_dir`, the game files can come from a `pack` built into the
//! program (the game crate's `embed-game` feature); it behaves the same way.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const FIRST_HANDLE: u32 = 0x1000;
pub const INVALID_HANDLE_VALUE: u32 = 0xffff_ffff;
pub const ERROR_FILE_NOT_FOUND: u32 = 2;
pub const ERROR_PATH_NOT_FOUND: u32 = 3;
/// Guest path of the game directory.
pub const GAME_DIR: &str = "C:\\Bugatron";
pub const ERROR_ACCESS_DENIED: u32 = 5;
pub const ERROR_FILE_EXISTS: u32 = 80;

const GENERIC_WRITE: u32 = 0x4000_0000;

/// The per-user folder for saves: `~/Library/Application Support/bugatron/` on macOS,
/// `~/.local/share/bugatron/` on Linux, `%APPDATA%\\bugatron\\` on Windows
/// (SDL_GetPrefPath, which creates it). Bugatron releases share their save formats,
/// so the folder is not per version.
pub fn default_save_dir() -> Option<PathBuf> {
    // SAFETY: NUL-terminated strings; SDL allocates the result, freed with SDL_free.
    unsafe {
        let p = sdl3_sys::filesystem::SDL_GetPrefPath(c"".as_ptr(), c"bugatron".as_ptr());
        if p.is_null() {
            return None;
        }
        let path = PathBuf::from(std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned());
        sdl3_sys::stdinc::SDL_free(p as *mut std::ffi::c_void);
        Some(path)
    }
}

/// Read-only game files built into the program: (path relative to the game folder
/// with `/` separators, contents).
pub type Pack = &'static [(&'static str, &'static [u8])];

enum Handle {
    Disk(File),
    Pack(Cursor<&'static [u8]>),
}

/// An original game file, on disk or in the pack.
enum DataFile {
    Disk(PathBuf),
    Pack(&'static [u8]),
}

#[derive(Default)]
pub struct Files {
    pub data_dir: PathBuf,
    /// Game files built into the program; used instead of `data_dir` when set.
    pub pack: Option<Pack>,
    pub save_dir: PathBuf,
    open: HashMap<u32, (Handle, String)>,
    next: u32,
    /// Current directory relative to the game directory (empty = the game directory).
    pub cwd: GuestPath,
    /// Open FindFirstFile searches: handle → remaining entries.
    searches: HashMap<u32, Vec<FindEntry>>,
}

/// One `WIN32_FIND_DATAA` result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindEntry {
    pub name: String,
    pub size: u64,
    pub dir: bool,
}

/// Win32 wildcard match (`*`, `?`), case-insensitive. `*.*` matches every name.
pub fn wildcard_match(pattern: &str, name: &str) -> bool {
    fn m(p: &[u8], n: &[u8]) -> bool {
        match (p.first(), n.first()) {
            (None, None) => true,
            (Some(b'*'), _) => m(&p[1..], n) || (!n.is_empty() && m(p, &n[1..])),
            (Some(b'?'), Some(_)) => m(&p[1..], &n[1..]),
            (Some(a), Some(b)) if a.eq_ignore_ascii_case(b) => m(&p[1..], &n[1..]),
            // "x.*" also matches names without an extension, as on Windows.
            (Some(b'.'), None) => p[1..].iter().all(|&c| c == b'*'),
            _ => false,
        }
    }
    m(pattern.as_bytes(), name.as_bytes())
}

/// A path relative to the game directory, as components. Windows path rules (both
/// separators, `.`, `..`, the current directory, the `C:\\Bugatron` prefix) are
/// applied once, by [`GuestPath::parse`]; host paths are then built per component.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GuestPath(Vec<String>);

impl GuestPath {
    /// Resolve `name` against `cwd`. Paths outside the game directory are rejected.
    pub fn parse(cwd: &GuestPath, name: &str) -> Result<GuestPath, String> {
        let n = name.replace('/', "\\");
        let lower = n.to_ascii_lowercase();
        let game = GAME_DIR.to_ascii_lowercase();
        let (mut parts, rest): (Vec<String>, &str) = if lower == game {
            (Vec::new(), "")
        } else if lower.starts_with(&(game.clone() + "\\")) {
            (Vec::new(), &n[game.len() + 1..])
        } else if (n.len() >= 2 && n.as_bytes()[1] == b':') || n.starts_with('\\') {
            return Err(format!("path outside the game directory: {name}"));
        } else {
            (cwd.0.clone(), &n[..])
        };
        for c in rest.split('\\') {
            match c {
                "" | "." => {}
                ".." => {
                    if parts.pop().is_none() {
                        return Err(format!("path outside the game directory: {name}"));
                    }
                }
                c => parts.push(c.to_string()),
            }
        }
        Ok(GuestPath(parts))
    }

    pub fn parts(&self) -> &[String] {
        &self.0
    }

    /// The folder and the last component (empty for the game directory itself).
    fn split_last(&self) -> (&[String], &str) {
        match self.0.split_last() {
            Some((last, dir)) => (dir, last),
            None => (&[], ""),
        }
    }

    /// As a Windows path relative to the game directory (`Audio\\boing.wav`).
    pub fn to_windows(&self) -> String {
        self.0.join("\\")
    }
}

/// Find `parts` under `root`, matching each component case-insensitively as Windows does.
fn resolve_ci(root: &Path, parts: &[String]) -> Option<PathBuf> {
    let mut cur = root.to_path_buf();
    for comp in parts {
        let exact = cur.join(comp);
        if exact.exists() {
            cur = exact;
            continue;
        }
        let found = std::fs::read_dir(&cur).ok()?.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(comp))?;
        cur = found.path();
    }
    Some(cur)
}

/// Lower-cased `/`-joined key, for matching pack paths.
fn pack_key(parts: &[String]) -> String {
    parts.join("/").to_ascii_lowercase()
}

/// Whether pack path `p` lies inside the folder `key` (a [`pack_key`], non-empty).
fn in_pack_dir<'a>(p: &'a str, key: &str) -> Option<&'a str> {
    (p.len() > key.len() && p.as_bytes()[key.len()] == b'/' && p[..key.len()].eq_ignore_ascii_case(key)).then(|| &p[key.len() + 1..])
}

impl Files {
    /// An original game file, case-insensitively.
    fn data_file(&self, rel: &[String]) -> Option<DataFile> {
        match self.pack {
            Some(pack) => {
                let key = pack_key(rel);
                pack.iter().find(|(p, _)| p.eq_ignore_ascii_case(&key)).map(|(_, d)| DataFile::Pack(d))
            }
            None => resolve_ci(&self.data_dir, rel).filter(|p| p.is_file()).map(DataFile::Disk),
        }
    }

    /// `rel` as spelled in the original game files, for every leading component that
    /// exists there. Windows keeps an existing file's or folder's name when it is
    /// opened under another case; the save copy does the same.
    fn data_spelling(&self, rel: &[String]) -> PathBuf {
        let mut out = PathBuf::new();
        for (i, part) in rel.iter().enumerate() {
            let found = match self.pack {
                Some(pack) => {
                    let key = pack_key(&rel[..=i]);
                    pack.iter().find_map(|(p, _)| {
                        (p.eq_ignore_ascii_case(&key) || in_pack_dir(p, &key).is_some()).then(|| p.split('/').nth(i).unwrap_or(part).to_string())
                    })
                }
                // Read the directory: on a case-insensitive file system a lookup
                // would echo the requested spelling instead of the stored one.
                None => std::fs::read_dir(self.data_dir.join(&out)).ok().and_then(|it| {
                    it.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).find(|n| n.eq_ignore_ascii_case(part))
                }),
            };
            out.push(found.as_deref().unwrap_or(part));
        }
        out
    }

    /// Whether `rel` is a folder of the original game files.
    fn data_dir_exists(&self, rel: &[String]) -> bool {
        match self.pack {
            Some(pack) => {
                let key = pack_key(rel);
                key.is_empty() || pack.iter().any(|(p, _)| in_pack_dir(p, &key).is_some())
            }
            None => resolve_ci(&self.data_dir, rel).is_some_and(|p| p.is_dir()),
        }
    }

    /// The entries of a folder of the original game files (None if it does not exist).
    fn data_entries(&self, rel: &[String]) -> Option<Vec<FindEntry>> {
        match self.pack {
            Some(pack) => {
                if !self.data_dir_exists(rel) {
                    return None;
                }
                let key = pack_key(rel);
                let mut out: Vec<FindEntry> = Vec::new();
                for (p, d) in pack {
                    let rest = if key.is_empty() {
                        *p
                    } else {
                        match in_pack_dir(p, &key) {
                            Some(rest) => rest,
                            None => continue,
                        }
                    };
                    let e = match rest.split_once('/') {
                        Some((dir, _)) => FindEntry { name: dir.to_string(), size: 0, dir: true },
                        None => FindEntry { name: rest.to_string(), size: d.len() as u64, dir: false },
                    };
                    if !out.iter().any(|f| f.name.eq_ignore_ascii_case(&e.name)) {
                        out.push(e);
                    }
                }
                Some(out)
            }
            None => {
                let d = resolve_ci(&self.data_dir, rel).filter(|p| p.is_dir())?;
                let mut out = Vec::new();
                for e in std::fs::read_dir(d).ok()?.flatten() {
                    let md = e.metadata().ok()?;
                    out.push(FindEntry { name: e.file_name().to_string_lossy().into_owned(), size: md.len(), dir: md.is_dir() });
                }
                Some(out)
            }
        }
    }

    /// CreateFileA. Returns a handle or a Win32 error code.
    pub fn create(&mut self, name: &str, access: u32, disposition: u32) -> Result<u32, u32> {
        let rel = GuestPath::parse(&self.cwd, name).map_err(|e| {
            eprintln!("[files] {e}");
            ERROR_ACCESS_DENIED
        })?;
        let rel = rel.parts();
        let write = access & GENERIC_WRITE != 0;
        let in_save = resolve_ci(&self.save_dir, rel).filter(|p| p.is_file());
        let in_data = self.data_file(rel);
        let exists = in_save.is_some() || in_data.is_some();
        // CREATE_NEW 1, CREATE_ALWAYS 2, OPEN_EXISTING 3, OPEN_ALWAYS 4, TRUNCATE_EXISTING 5
        match disposition {
            1 if exists => return Err(ERROR_FILE_EXISTS),
            3 | 5 if !exists => return Err(ERROR_FILE_NOT_FOUND),
            1..=5 => {}
            d => crate::fatal(&format!("CreateFileA({name}): unsupported disposition {d}")),
        }
        let file = if !write {
            match (in_save, in_data) {
                (Some(p), _) | (None, Some(DataFile::Disk(p))) => Handle::Disk(File::open(p).map_err(|_| ERROR_ACCESS_DENIED)?),
                (None, Some(DataFile::Pack(d))) => Handle::Pack(Cursor::new(d)),
                (None, None) => return Err(ERROR_FILE_NOT_FOUND),
            }
        } else {
            let target = in_save.unwrap_or_else(|| self.save_dir.join(self.data_spelling(rel)));
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|_| ERROR_ACCESS_DENIED)?;
            }
            let truncate = matches!(disposition, 2 | 5);
            if !target.exists() && !truncate {
                // copy-on-write
                match &in_data {
                    Some(DataFile::Disk(orig)) => { std::fs::copy(orig, &target).map_err(|_| ERROR_ACCESS_DENIED)?; }
                    Some(DataFile::Pack(d)) => std::fs::write(&target, d).map_err(|_| ERROR_ACCESS_DENIED)?,
                    None => {}
                }
            }
            Handle::Disk(OpenOptions::new().read(true).write(true).create(true).truncate(truncate).open(&target).map_err(|_| ERROR_ACCESS_DENIED)?)
        };
        if self.next == 0 {
            self.next = FIRST_HANDLE;
        }
        let h = self.next;
        self.next += 4;
        self.open.insert(h, (file, name.to_string()));
        Ok(h)
    }

    /// SetCurrentDirectoryA: the directory must exist in the game or save directory.
    pub fn set_cwd(&mut self, path: &str) -> Result<(), u32> {
        let rel = GuestPath::parse(&self.cwd, path).map_err(|_| ERROR_PATH_NOT_FOUND)?;
        let in_save = resolve_ci(&self.save_dir, rel.parts()).is_some_and(|p| p.is_dir());
        if !self.data_dir_exists(rel.parts()) && !in_save {
            return Err(ERROR_PATH_NOT_FOUND);
        }
        self.cwd = rel;
        Ok(())
    }

    /// The guest current directory as an absolute guest path.
    pub fn cwd_path(&self) -> String {
        if self.cwd.parts().is_empty() { GAME_DIR.to_string() } else { format!("{GAME_DIR}\\{}", self.cwd.to_windows()) }
    }

    /// FindFirstFileA: entries of the game and save directories matching the
    /// pattern's file part, in NTFS order (case-insensitive). Returns a search handle.
    pub fn find_first(&mut self, pattern: &str) -> Result<(u32, FindEntry), u32> {
        let rel = GuestPath::parse(&self.cwd, pattern).map_err(|_| ERROR_PATH_NOT_FOUND)?;
        let (dir, pat) = rel.split_last();
        let mut found: Vec<FindEntry> = Vec::new();
        let mut any_dir = false;
        let mut in_save = Vec::new();
        if let Some(d) = resolve_ci(&self.save_dir, dir).filter(|p| p.is_dir()) {
            any_dir = true;
            for e in std::fs::read_dir(d).map_err(|_| ERROR_PATH_NOT_FOUND)?.flatten() {
                let md = e.metadata().map_err(|_| ERROR_ACCESS_DENIED)?;
                in_save.push(FindEntry { name: e.file_name().to_string_lossy().into_owned(), size: md.len(), dir: md.is_dir() });
            }
        }
        let in_data = self.data_entries(dir);
        any_dir |= in_data.is_some();
        for e in in_save.into_iter().chain(in_data.into_iter().flatten()) {
            if wildcard_match(pat, &e.name) && !found.iter().any(|f| f.name.eq_ignore_ascii_case(&e.name)) {
                found.push(e);
            }
        }
        if !any_dir {
            return Err(ERROR_PATH_NOT_FOUND);
        }
        found.sort_by_key(|f| f.name.to_ascii_uppercase());
        found.reverse(); // popped from the end
        let first = found.pop().ok_or(ERROR_FILE_NOT_FOUND)?;
        if self.next == 0 {
            self.next = FIRST_HANDLE;
        }
        let h = self.next;
        self.next += 4;
        self.searches.insert(h, found);
        Ok((h, first))
    }

    /// FindNextFileA: the next entry, or None (ERROR_NO_MORE_FILES).
    pub fn find_next(&mut self, h: u32) -> Option<Option<FindEntry>> {
        self.searches.get_mut(&h).map(|v| v.pop())
    }

    pub fn find_close(&mut self, h: u32) -> bool {
        self.searches.remove(&h).is_some()
    }

    pub fn is_file(&self, h: u32) -> bool {
        self.open.contains_key(&h)
    }

    pub fn read(&mut self, h: u32, buf: &mut [u8]) -> Option<usize> {
        let f = &mut self.open.get_mut(&h)?.0;
        let mut n = 0;
        while n < buf.len() {
            let r = match f {
                Handle::Disk(f) => f.read(&mut buf[n..]),
                Handle::Pack(c) => c.read(&mut buf[n..]),
            };
            match r {
                Ok(0) => break,
                Ok(k) => n += k,
                Err(_) => return None,
            }
        }
        Some(n)
    }

    pub fn write(&mut self, h: u32, data: &[u8]) -> Option<usize> {
        let Handle::Disk(f) = &mut self.open.get_mut(&h)?.0 else { return None };
        f.write_all(data).ok()?;
        Some(data.len())
    }

    /// SetFilePointer with a 32-bit signed distance. Returns the new position.
    pub fn seek(&mut self, h: u32, dist: i32, method: u32) -> Option<u64> {
        let f = &mut self.open.get_mut(&h)?.0;
        let pos = match method {
            0 => SeekFrom::Start(dist as u32 as u64),
            1 => SeekFrom::Current(dist as i64),
            2 => SeekFrom::End(dist as i64),
            _ => return None,
        };
        match f {
            Handle::Disk(f) => f.seek(pos).ok(),
            Handle::Pack(c) => c.seek(pos).ok(),
        }
    }

    /// FlushFileBuffers.
    pub fn flush(&mut self, h: u32) -> bool {
        match self.open.get_mut(&h).map(|o| &mut o.0) {
            Some(Handle::Disk(f)) => f.sync_all().is_ok(),
            _ => false,
        }
    }

    /// SetEndOfFile: truncate (or extend) the file at its current position.
    pub fn set_end(&mut self, h: u32) -> bool {
        match self.open.get_mut(&h).map(|o| &mut o.0) {
            Some(Handle::Disk(f)) => f.stream_position().and_then(|p| f.set_len(p)).is_ok(),
            _ => false,
        }
    }

    pub fn size(&mut self, h: u32) -> Option<u64> {
        match &self.open.get(&h)?.0 {
            Handle::Disk(f) => Some(f.metadata().ok()?.len()),
            Handle::Pack(c) => Some(c.get_ref().len() as u64),
        }
    }

    pub fn close(&mut self, h: u32) -> bool {
        self.open.remove(&h).is_some()
    }
}
