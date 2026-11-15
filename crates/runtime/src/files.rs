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
pub const ERROR_ACCESS_DENIED: u32 = 5;
pub const ERROR_FILE_EXISTS: u32 = 80;

const GENERIC_WRITE: u32 = 0x4000_0000;

#[derive(Default)]
pub struct Files {
    pub data_dir: PathBuf,
    pub save_dir: PathBuf,
    open: HashMap<u32, (File, String)>,
    next: u32,
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

/// Strip a drive or the virtual install directory, leaving a path relative to the game directory.
fn guest_relative(name: &str) -> Result<String, String> {
    let n = name.replace('/', "\\");
    let lower = n.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("c:\\bugatron\\") {
        return Ok(n[n.len() - rest.len()..].to_string());
    }
    if n.len() >= 2 && n.as_bytes()[1] == b':' || n.starts_with('\\') {
        return Err(format!("path outside the game directory: {name}"));
    }
    Ok(n)
}

impl Files {
    /// CreateFileA. Returns a handle or a Win32 error code.
    pub fn create(&mut self, name: &str, access: u32, disposition: u32) -> Result<u32, u32> {
        let rel = guest_relative(name).map_err(|e| {
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
