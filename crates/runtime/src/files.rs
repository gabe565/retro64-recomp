//! Win32 file handles over the host file system.
//!
//! Guest paths (relative, or under `C:\Bugatron\`) resolve case-insensitively
//! against two directories:
//! - `save_dir`: files the game creates or modifies (prefs, high scores);
//! - `data_dir`: the user's original game files, opened read-only.
//!
//! Reads prefer `save_dir`. Opening an original file for writing first copies it
//! into `save_dir` (copy-on-write), so original game files are never modified.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
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

#[derive(Default)]
pub struct Files {
    pub data_dir: PathBuf,
    pub save_dir: PathBuf,
    open: HashMap<u32, (File, String)>,
    next: u32,
    /// Current directory relative to the game directory ("" = the game directory itself).
    pub cwd: String,
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

/// Find `rel` (components separated by `\` or `/`) under `root`, matching each
/// component case-insensitively as Windows does.
fn resolve_ci(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut cur = root.to_path_buf();
    for comp in rel.split(['\\', '/']).filter(|c| !c.is_empty() && *c != ".") {
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

/// Resolve a guest path to a path relative to the game directory, applying the
/// current directory and `.`/`..`. Paths outside the game directory are rejected.
pub fn guest_relative(cwd: &str, name: &str) -> Result<String, String> {
    let n = name.replace('/', "\\");
    let lower = n.to_ascii_lowercase();
    let game = GAME_DIR.to_ascii_lowercase();
    let (base, rest): (Vec<&str>, &str) = if lower == game {
        (Vec::new(), "")
    } else if lower.starts_with(&(game.clone() + "\\")) {
        (Vec::new(), &n[game.len() + 1..])
    } else if (n.len() >= 2 && n.as_bytes()[1] == b':') || n.starts_with('\\') {
        return Err(format!("path outside the game directory: {name}"));
    } else {
        (cwd.split('\\').filter(|c| !c.is_empty()).collect(), &n[..])
    };
    let mut parts = base;
    for c in rest.split('\\') {
        match c {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(format!("path outside the game directory: {name}"));
                }
            }
            c => parts.push(c),
        }
    }
    Ok(parts.join("\\"))
}

impl Files {
    /// CreateFileA. Returns a handle or a Win32 error code.
    pub fn create(&mut self, name: &str, access: u32, disposition: u32) -> Result<u32, u32> {
        let rel = guest_relative(&self.cwd, name).map_err(|e| {
            eprintln!("[files] {e}");
            ERROR_ACCESS_DENIED
        })?;
        let write = access & GENERIC_WRITE != 0;
        let in_save = resolve_ci(&self.save_dir, &rel).filter(|p| p.is_file());
        let in_data = resolve_ci(&self.data_dir, &rel).filter(|p| p.is_file());
        let existing = in_save.clone().or(in_data.clone());
        // CREATE_NEW 1, CREATE_ALWAYS 2, OPEN_EXISTING 3, OPEN_ALWAYS 4, TRUNCATE_EXISTING 5
        match disposition {
            1 if existing.is_some() => return Err(ERROR_FILE_EXISTS),
            3 | 5 if existing.is_none() => return Err(ERROR_FILE_NOT_FOUND),
            1..=5 => {}
            d => crate::fatal(&format!("CreateFileA({name}): unsupported disposition {d}")),
        }
        let file = if !write {
            File::open(existing.ok_or(ERROR_FILE_NOT_FOUND)?).map_err(|_| ERROR_ACCESS_DENIED)?
        } else {
            let target = in_save.unwrap_or_else(|| self.save_dir.join(rel.replace('\\', "/")));
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|_| ERROR_ACCESS_DENIED)?;
            }
            let truncate = matches!(disposition, 2 | 5);
            if !target.exists() && !truncate {
                if let Some(orig) = &in_data {
                    std::fs::copy(orig, &target).map_err(|_| ERROR_ACCESS_DENIED)?; // copy-on-write
                }
            }
            OpenOptions::new().read(true).write(true).create(true).truncate(truncate).open(&target).map_err(|_| ERROR_ACCESS_DENIED)?
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
        let rel = guest_relative(&self.cwd, path).map_err(|_| ERROR_PATH_NOT_FOUND)?;
        let exists = |root: &Path| resolve_ci(root, &rel).is_some_and(|p| p.is_dir());
        if !exists(&self.data_dir) && !exists(&self.save_dir) {
            return Err(ERROR_PATH_NOT_FOUND);
        }
        self.cwd = rel;
        Ok(())
    }

    /// The guest current directory as an absolute guest path.
    pub fn cwd_path(&self) -> String {
        if self.cwd.is_empty() { GAME_DIR.to_string() } else { format!("{GAME_DIR}\\{}", self.cwd) }
    }

    /// FindFirstFileA: entries of the game and save directories matching the
    /// pattern's file part, in NTFS order (case-insensitive). Returns a search handle.
    pub fn find_first(&mut self, pattern: &str) -> Result<(u32, FindEntry), u32> {
        let rel = guest_relative(&self.cwd, pattern).map_err(|_| ERROR_PATH_NOT_FOUND)?;
        let (dir, pat) = match rel.rfind('\\') {
            Some(i) => (&rel[..i], &rel[i + 1..]),
            None => ("", &rel[..]),
        };
        let mut found: Vec<FindEntry> = Vec::new();
        let mut any_dir = false;
        for root in [&self.save_dir, &self.data_dir] {
            let Some(d) = resolve_ci(root, dir).filter(|p| p.is_dir()) else { continue };
            any_dir = true;
            for e in std::fs::read_dir(d).map_err(|_| ERROR_PATH_NOT_FOUND)?.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if !wildcard_match(pat, &name) || found.iter().any(|f| f.name.eq_ignore_ascii_case(&name)) {
                    continue;
                }
                let md = e.metadata().map_err(|_| ERROR_ACCESS_DENIED)?;
                found.push(FindEntry { name, size: md.len(), dir: md.is_dir() });
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
            match f.read(&mut buf[n..]) {
                Ok(0) => break,
                Ok(k) => n += k,
                Err(_) => return None,
            }
        }
        Some(n)
    }

    pub fn write(&mut self, h: u32, data: &[u8]) -> Option<usize> {
        let f = &mut self.open.get_mut(&h)?.0;
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
        f.seek(pos).ok()
    }

    pub fn size(&mut self, h: u32) -> Option<u64> {
        Some(self.open.get(&h)?.0.metadata().ok()?.len())
    }

    pub fn close(&mut self, h: u32) -> bool {
        self.open.remove(&h).is_some()
    }
}
