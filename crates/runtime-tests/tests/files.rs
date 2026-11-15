//! Win32 file imports over the game and save directories.

use bugatron_runtime::testing::call_import;
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
