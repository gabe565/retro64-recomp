//! Guest memory that was never committed must fault with a diagnostic naming the
//! guest address (exit status 102), on every OS. The test re-runs itself as a
//! child process because the fault terminates the process.

use std::process::Command;

#[test]
fn uncommitted_guest_access_is_reported() {
    if std::env::var("BUGATRON_FAULT_CHILD").is_ok() {
        bugatron_x86_tests::install_runtime();
        bugatron_runtime::with_rt(|rt| {
            let v = rt.mem.r32(0x5000_0000); // never committed
            println!("unexpected read value {v}");
        });
        return;
    }
    let exe = std::env::current_exe().unwrap();
    let out = Command::new(exe)
        .args(["--exact", "uncommitted_guest_access_is_reported", "--nocapture"])
        .env("BUGATRON_FAULT_CHILD", "1")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(102), "child status {:?}, stderr: {stderr}", out.status);
    assert!(stderr.contains("guest memory fault at 0x50000000"), "stderr: {stderr}");
}
