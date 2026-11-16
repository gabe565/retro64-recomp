//! Imports reachable only on rare paths (C runtime errors, teardown, the unused
//! LINK_BUY menu action), from an audit of every import Bug3D.exe declares.

use bugatron_runtime::testing::call_import;
use bugatron_runtime::with_rt;
use bugatron_runtime_tests::*;

#[test]
fn pointer_checks_follow_committed_memory_and_translated_code() {
    let mut cpu = setup();
    let k = "kernel32.dll";
    // SCRATCH is committed guest memory; 0x7000_0000 is reserved but never committed.
    assert_eq!(call_import(&mut cpu, k, "IsBadReadPtr", &[SCRATCH, 16]), 0);
    assert_eq!(call_import(&mut cpu, k, "IsBadWritePtr", &[SCRATCH, 16]), 0);
    assert_eq!(call_import(&mut cpu, k, "IsBadReadPtr", &[0x7000_0000, 4]), 1);
    assert_eq!(call_import(&mut cpu, k, "IsBadReadPtr", &[0x7000_0000, 0]), 0, "empty range");
    assert_eq!(call_import(&mut cpu, k, "IsBadCodePtr", &[HELPER_WNDPROC]), 0, "a translated function");
    assert_eq!(call_import(&mut cpu, k, "IsBadCodePtr", &[SCRATCH]), 1);
}

#[test]
fn runtime_error_and_teardown_helpers() {
    let mut cpu = setup();
    let k = "kernel32.dll";
    let user32 = put_str(0, "user32.dll");
    assert_eq!(call_import(&mut cpu, k, "LoadLibraryA", &[user32]), 0);
    assert_eq!(call_import(&mut cpu, k, "GetLastError", &[]), 126, "ERROR_MOD_NOT_FOUND");
    assert_eq!(call_import(&mut cpu, k, "SetStdHandle", &[0xffff_fff5, 0]), 1);
    let (name, value) = (put_str(0x40, "=C:"), put_str(0x80, "C:\\Bugatron"));
    assert_eq!(call_import(&mut cpu, k, "SetEnvironmentVariableA", &[name, value]), 1);
    assert_eq!(call_import(&mut cpu, k, "TerminateProcess", &[0x1234, 0]), 0, "not the current process");
}

#[test]
fn flush_and_set_end_of_file() {
    let mut cpu = setup();
    let base = std::env::temp_dir().join(format!("bugatron-imports-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    with_rt(|rt| {
        rt.files.data_dir = base.join("game");
        rt.files.save_dir = base.join("save");
    });
    std::fs::create_dir_all(base.join("game")).unwrap();
    let k = "kernel32.dll";
    let name = put_str(0, "trunc.bin");
    let h = call_import(&mut cpu, k, "CreateFileA", &[name, 0xC000_0000, 0, 0, 2, 0x80, 0]);
    let data = put_bytes(0x100, b"0123456789");
    let written = SCRATCH + 0x200;
    assert_eq!(call_import(&mut cpu, k, "WriteFile", &[h, data, 10, written, 0]), 1);
    assert_eq!(call_import(&mut cpu, k, "SetFilePointer", &[h, 4, 0, 0]), 4);
    assert_eq!(call_import(&mut cpu, k, "SetEndOfFile", &[h]), 1);
    assert_eq!(call_import(&mut cpu, k, "FlushFileBuffers", &[h]), 1);
    assert_eq!(call_import(&mut cpu, k, "CloseHandle", &[h]), 1);
    assert_eq!(std::fs::read(base.join("save/trunc.bin")).unwrap(), b"0123");
    assert_eq!(call_import(&mut cpu, k, "SetEndOfFile", &[0x9999]), 0, "bad handle");
    let _ = std::fs::remove_dir_all(&base);
}

/// LINK_BUY passes "URL=http://…" (website3.url's second token): not a file or
/// URL, so it fails as on Windows instead of stopping the game.
#[test]
fn shell_execute_of_a_non_url_fails_like_windows() {
    let mut cpu = setup();
    let (verb, file) = (put_str(0, "open"), put_str(0x40, "URL=http://www.bugatron.com/purchase.html"));
    assert_eq!(call_import(&mut cpu, "shell32.dll", "ShellExecuteA", &[0, verb, file, 0, 0, 1]), 2, "SE_ERR_FNF");
}
