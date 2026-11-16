//! Win32 file imports over the game and save directories.

use bugatron_runtime::testing::call_import;
use bugatron_runtime::files::{FindEntry, Files, ERROR_FILE_NOT_FOUND};
use bugatron_runtime::with_rt;
use bugatron_runtime_tests::*;
use std::path::PathBuf;

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const CREATE_NEW: u32 = 1;
const CREATE_ALWAYS: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const OPEN_ALWAYS: u32 = 4;
const INVALID: u32 = 0xffff_ffff;

/// Fresh game and save directories under the system temp dir.
fn dirs(tag: &str) -> (PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!("bugatron-files-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (data, save) = (base.join("game"), base.join("save"));
    std::fs::create_dir_all(data.join("Graphics")).unwrap();
    std::fs::write(data.join("Graphics").join("Title.BMP"), b"original-bytes").unwrap();
    std::fs::write(data.join("prefs"), [1u8, 0, 0, 0]).unwrap();
    with_rt(|rt| {
        rt.files.data_dir = data.clone();
        rt.files.save_dir = save.clone();
    });
    (data, save)
}

fn last_error(cpu: &mut bugatron_runtime::Cpu) -> u32 {
    call_import(cpu, "kernel32.dll", "GetLastError", &[])
}

#[test]
fn case_insensitive_read_and_seek() {
    let mut cpu = setup();
    dirs("read");
    let name = put_str(0, "graphics\\TITLE.bmp");
    let h = call_import(&mut cpu, "kernel32.dll", "CreateFileA", &[name, GENERIC_READ, 1, 0, OPEN_EXISTING, 0x80, 0]);
    assert_ne!(h, INVALID);
    assert_eq!(call_import(&mut cpu, "kernel32.dll", "GetFileSize", &[h, 0]), 14);
    let (buf, nread) = (SCRATCH + 0x100, SCRATCH + 0x200);
    assert_eq!(call_import(&mut cpu, "kernel32.dll", "SetFilePointer", &[h, 9, 0, 0]), 9);
    assert_eq!(call_import(&mut cpu, "kernel32.dll", "ReadFile", &[h, buf, 100, nread, 0]), 1);
    assert_eq!(r32(nread), 5);
    assert_eq!(read(buf, 5), b"bytes");
    // Reading at end of file succeeds with zero bytes, as on Windows.
    assert_eq!(call_import(&mut cpu, "kernel32.dll", "ReadFile", &[h, buf, 10, nread, 0]), 1);
    assert_eq!(r32(nread), 0);
    assert_eq!(call_import(&mut cpu, "kernel32.dll", "GetFileType", &[h]), 1); // FILE_TYPE_DISK
    assert_eq!(call_import(&mut cpu, "kernel32.dll", "CloseHandle", &[h]), 1);
    assert_eq!(call_import(&mut cpu, "kernel32.dll", "CloseHandle", &[h]), 0);
}

#[test]
fn missing_file_and_create_new_errors() {
    let mut cpu = setup();
    dirs("errors");
    let missing = put_str(0, "nothere.dat");
    assert_eq!(call_import(&mut cpu, "kernel32.dll", "CreateFileA", &[missing, GENERIC_READ, 0, 0, OPEN_EXISTING, 0, 0]), INVALID);
    assert_eq!(last_error(&mut cpu), 2); // ERROR_FILE_NOT_FOUND
    let existing = put_str(0x40, "PREFS");
    assert_eq!(call_import(&mut cpu, "kernel32.dll", "CreateFileA", &[existing, GENERIC_WRITE, 0, 0, CREATE_NEW, 0, 0]), INVALID);
    assert_eq!(last_error(&mut cpu), 80); // ERROR_FILE_EXISTS
    let outside = put_str(0x80, "D:\\elsewhere\\x.txt");
    assert_eq!(call_import(&mut cpu, "kernel32.dll", "CreateFileA", &[outside, GENERIC_READ, 0, 0, OPEN_EXISTING, 0, 0]), INVALID);
}

#[test]
fn writes_are_copy_on_write_into_the_save_directory() {
    let mut cpu = setup();
    let (data, save) = dirs("cow");
    let name = put_str(0, "C:\\Bugatron\\prefs");
    let h = call_import(&mut cpu, "kernel32.dll", "CreateFileA", &[name, GENERIC_READ | GENERIC_WRITE, 0, 0, OPEN_ALWAYS, 0, 0]);
    assert_ne!(h, INVALID);
    // Existing content was copied; overwrite the second dword.
    assert_eq!(call_import(&mut cpu, "kernel32.dll", "SetFilePointer", &[h, 4, 0, 0]), 4);
    w32(SCRATCH + 0x100, 0x2a);
    assert_eq!(call_import(&mut cpu, "kernel32.dll", "WriteFile", &[h, SCRATCH + 0x100, 4, SCRATCH + 0x200, 0]), 1);
    assert_eq!(r32(SCRATCH + 0x200), 4);
    call_import(&mut cpu, "kernel32.dll", "CloseHandle", &[h]);
    assert_eq!(std::fs::read(data.join("prefs")).unwrap(), [1, 0, 0, 0], "original game file must not change");
    assert_eq!(std::fs::read(save.join("prefs")).unwrap(), [1, 0, 0, 0, 0x2a, 0, 0, 0]);
    // Later reads see the saved copy.
    let h = call_import(&mut cpu, "kernel32.dll", "CreateFileA", &[name, GENERIC_READ, 0, 0, OPEN_EXISTING, 0, 0]);
    assert_eq!(call_import(&mut cpu, "kernel32.dll", "GetFileSize", &[h, 0]), 8);
    // CREATE_ALWAYS truncates (in the save directory only).
    let h2 = call_import(&mut cpu, "kernel32.dll", "CreateFileA", &[name, GENERIC_WRITE, 0, 0, CREATE_ALWAYS, 0, 0]);
    assert_eq!(call_import(&mut cpu, "kernel32.dll", "GetFileSize", &[h2, 0]), 0);
    assert_eq!(std::fs::read(data.join("prefs")).unwrap(), [1, 0, 0, 0]);
}

#[test]
fn current_directory_and_relative_paths() {
    let mut cpu = setup();
    dirs("cwd");
    let gfx = put_str(0, "GRAPHICS");
    assert_eq!(call_import(&mut cpu, "kernel32.dll", "SetCurrentDirectoryA", &[gfx]), 1);
    let buf = SCRATCH + 0x100;
    let n = call_import(&mut cpu, "kernel32.dll", "GetCurrentDirectoryA", &[64, buf]);
    assert_eq!(String::from_utf8(read(buf, n as usize)).unwrap(), "C:\\Bugatron\\GRAPHICS");
    let name = put_str(0x40, "title.bmp");
    let h = call_import(&mut cpu, "kernel32.dll", "CreateFileA", &[name, GENERIC_READ, 0, 0, OPEN_EXISTING, 0, 0]);
    assert_ne!(h, INVALID, "relative to the current directory");
    let up = put_str(0x80, "..\\prefs");
    assert_ne!(call_import(&mut cpu, "kernel32.dll", "CreateFileA", &[up, GENERIC_READ, 0, 0, OPEN_EXISTING, 0, 0]), INVALID);
    let root = put_str(0xc0, "..");
    assert_eq!(call_import(&mut cpu, "kernel32.dll", "SetCurrentDirectoryA", &[root]), 1);
    assert_eq!(call_import(&mut cpu, "kernel32.dll", "SetCurrentDirectoryA", &[root]), 0, "cannot leave the game directory");
    let missing = put_str(0x100, "Nowhere");
    assert_eq!(call_import(&mut cpu, "kernel32.dll", "SetCurrentDirectoryA", &[missing]), 0);
    assert_eq!(last_error(&mut cpu), 3); // ERROR_PATH_NOT_FOUND
}

#[test]
fn find_files_merges_directories_in_ntfs_order() {
    let mut cpu = setup();
    let (data, save) = dirs("find");
    for n in ["pack2.XML.lev", "Pack1.xml.lev", "BUG1.XML.lev", "notes.txt"] {
        std::fs::write(data.join(n), b"x").unwrap();
    }
    std::fs::create_dir_all(&save).unwrap();
    std::fs::write(save.join("pack3.xml.lev"), b"yy").unwrap();
    std::fs::write(save.join("PACK1.XML.LEV"), b"zzz").unwrap();
    let pat = put_str(0, "*.xml.lev");
    let fd = SCRATCH + 0x400;
    let h = call_import(&mut cpu, "kernel32.dll", "FindFirstFileA", &[pat, fd]);
    assert_ne!(h, INVALID);
    let name = |fd: u32| {
        let b = read(fd + 0x2c, 260);
        String::from_utf8(b[..b.iter().position(|&c| c == 0).unwrap()].to_vec()).unwrap()
    };
    let mut names = vec![(name(fd), r32(fd + 0x20))];
    while call_import(&mut cpu, "kernel32.dll", "FindNextFileA", &[h, fd]) != 0 {
        names.push((name(fd), r32(fd + 0x20)));
    }
    assert_eq!(last_error(&mut cpu), 18); // ERROR_NO_MORE_FILES
    assert_eq!(call_import(&mut cpu, "kernel32.dll", "FindClose", &[h]), 1);
    assert_eq!(names, [("BUG1.XML.lev".into(), 1), ("PACK1.XML.LEV".into(), 3), ("pack2.XML.lev".into(), 1), ("pack3.xml.lev".into(), 2)],
        "case-insensitive order; the saved copy shadows the original");
    let none = put_str(0x40, "*.zzz");
    assert_eq!(call_import(&mut cpu, "kernel32.dll", "FindFirstFileA", &[none, fd]), INVALID);
    assert_eq!(last_error(&mut cpu), 2);
}

#[test]
fn wildcard_matching() {
    use bugatron_runtime::files::wildcard_match as m;
    assert!(m("*.*", "prefs"));
    assert!(m("*.lev", "BUG1.XML.LEV"));
    assert!(m("bug?.xml.lev", "BUG7.XML.lev"));
    assert!(!m("bug?.xml.lev", "bug17.xml.lev"));
    assert!(m("pack*", "PACK1.hs"));
    assert!(!m("*.hs", "pack1.hsx"));
}

/// Game files built into the program (`embed-game`) behave like a data folder:
/// case-insensitive reads, folder listings, the current directory, and
/// copy-on-write into the save folder.
#[test]
fn embedded_pack_serves_as_the_game_folder() {
    static PACK: &[(&str, &[u8])] = &[
        ("Bug3D.exe", b"MZ"),
        ("prefs", b"\x01\x02"),
        ("Audio/boing.wav", b"RIFF"),
        ("Audio/explode.wav", b"RIFF!"),
        ("Graphics/back0.bmp", b"BM"),
    ];
    let save = std::env::temp_dir().join(format!("bugatron-pack-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&save);
    let mut f = Files::default();
    f.pack = Some(PACK);
    f.save_dir = save.clone();

    let h = f.create("AUDIO\\Explode.WAV", 0x8000_0000, 3).unwrap();
    assert_eq!(f.size(h), Some(5));
    let mut buf = [0u8; 8];
    assert_eq!(f.read(h, &mut buf), Some(5));
    assert_eq!(&buf[..5], b"RIFF!");
    assert_eq!(f.seek(h, 1, 0), Some(1));
    f.close(h);
    assert_eq!(f.create("missing.txt", 0x8000_0000, 3), Err(ERROR_FILE_NOT_FOUND));

    f.set_cwd("audio").unwrap();
    let (h, first) = f.find_first("*.wav").unwrap();
    assert_eq!(first.name, "boing.wav");
    assert_eq!(f.find_next(h), Some(Some(FindEntry { name: "explode.wav".into(), size: 5, dir: false })));
    assert_eq!(f.find_next(h), Some(None));
    f.set_cwd("..").unwrap();
    let (_, first) = f.find_first("*.*").unwrap();
    assert_eq!(first, FindEntry { name: "Audio".into(), size: 0, dir: true });
    assert_eq!(f.set_cwd("Fonts"), Err(3));

    // Writing an original file copies it out of the pack first.
    let h = f.create("PREFS", 0xC000_0000, 3).unwrap();
    assert_eq!(f.write(h, b"\x09"), Some(1));
    f.close(h);
    assert_eq!(std::fs::read(save.join("prefs")).unwrap(), b"\x09\x02");
    assert_eq!(names(&save), ["prefs"], "the copy keeps the original's spelling, as on Windows");
    // A new file in an existing folder goes into that folder's spelling.
    let h = f.create("AUDIO\\new.wav", 0xC000_0000, 1).unwrap();
    f.close(h);
    assert_eq!(names(&save.join("Audio")), ["new.wav"]);
    let h = f.create("prefs", 0x8000_0000, 3).unwrap();
    assert_eq!(f.read(h, &mut buf), Some(2));
    assert_eq!(&buf[..2], b"\x09\x02", "the save folder wins over the pack");
    let _ = std::fs::remove_dir_all(&save);
}

/// File names in `dir` exactly as stored (case-sensitive), sorted.
fn names(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

/// The same for an on-disk game folder: writing `GRAPHICS\TITLE.bmp` copies
/// `Graphics/Title.BMP` under its own name.
#[test]
fn copy_on_write_keeps_the_original_spelling() {
    let mut cpu = setup();
    let (_data, save) = dirs("spelling");
    let name = put_str(0, "GRAPHICS\\title.bmp");
    let h = call_import(&mut cpu, "kernel32.dll", "CreateFileA", &[name, GENERIC_READ | GENERIC_WRITE, 0, 0, OPEN_EXISTING, 0x80, 0]);
    assert_ne!(h, INVALID);
    assert_eq!(call_import(&mut cpu, "kernel32.dll", "CloseHandle", &[h]), 1);
    assert_eq!(names(&save), ["Graphics"]);
    assert_eq!(names(&save.join("Graphics")), ["Title.BMP"]);
}
