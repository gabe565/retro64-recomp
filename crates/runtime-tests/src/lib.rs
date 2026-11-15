//! Shared fixtures for runtime regression tests.
//!
//! Each test installs a bare runtime (no executable image) with headless
//! windowing, and drives imports and COM methods through
//! `bugatron_runtime::testing`, the dispatch path recompiled code uses.
//!
//! The guest address space is process-wide (generated code uses one base
//! pointer), so tests hold a global lock for their whole duration and the
//! fixture uninstalls its runtime before releasing it.

use bugatron_runtime::{with_rt, Cpu, Runtime};
use std::sync::{Mutex, MutexGuard};

static GUEST_SPACE: Mutex<()> = Mutex::new(());

/// A CPU bound to an installed runtime; dereferences to [`Cpu`].
pub struct Fixture {
    cpu: Cpu,
    _lock: MutexGuard<'static, ()>,
}

impl std::ops::Deref for Fixture {
    type Target = Cpu;
    fn deref(&self) -> &Cpu {
        &self.cpu
    }
}

impl std::ops::DerefMut for Fixture {
    fn deref_mut(&mut self) -> &mut Cpu {
        &mut self.cpu
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        drop(Runtime::uninstall()); // release the guest space while still holding the lock
    }
}

/// Recording window procedure (logs msg, wParam, lParam; returns 1 for WM_NCCREATE).
pub const HELPER_WNDPROC: u32 = 0x0040_1000;
/// stdcall callback with 2 arguments that logs them and returns 1 (continue).
pub const HELPER_REC2_CONTINUE: u32 = 0x0040_1100;
/// stdcall callback with 2 arguments that logs them and returns 0 (cancel).
pub const HELPER_REC2_CANCEL: u32 = 0x0040_1200;
/// stdcall callback with 6 arguments that logs them and returns 1.
pub const HELPER_REC6_CONTINUE: u32 = 0x0040_1300;
/// EnumDisplayModes callback that copies each 0x7c-byte descriptor into the log.
pub const HELPER_MODE_COPIER: u32 = 0x0040_1400;
/// stdcall callback with 4 arguments that logs them and returns 1.
pub const HELPER_REC4_CONTINUE: u32 = 0x0040_1500;

const LOG_PTR: u32 = 0x0061_0000;
const LOG_START: u32 = LOG_PTR + 0x10;
const LOG_SIZE: usize = 0x10_0000;
/// Scratch guest memory for test structures.
pub const SCRATCH: u32 = 0x0080_0000;
pub const SCRATCH_SIZE: usize = 0x10_0000;

/// Install a bare, headless runtime and return a CPU ready for calls.
pub fn setup() -> Fixture {
    let lock = GUEST_SPACE.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = Runtime::bare().expect("bare runtime");
    rt.mem.commit(LOG_PTR, LOG_SIZE).expect("commit log");
    rt.mem.commit(SCRATCH, SCRATCH_SIZE).expect("commit scratch");
    rt.mem.w32(LOG_PTR, LOG_START);
    rt.user.headless = true;
    rt.install();
    Fixture { cpu: with_rt(|rt| rt.initial_cpu()), _lock: lock }
}

/// Logged dwords since setup (or the last `clear_log`).
pub fn log() -> Vec<u32> {
    with_rt(|rt| {
        let end = rt.mem.r32(LOG_PTR);
        (LOG_START..end).step_by(4).map(|a| rt.mem.r32(a)).collect()
    })
}

pub fn log_bytes() -> Vec<u8> {
    with_rt(|rt| {
        let end = rt.mem.r32(LOG_PTR);
        rt.mem.slice(LOG_START, (end - LOG_START) as usize).to_vec()
    })
}

pub fn clear_log() {
    with_rt(|rt| rt.mem.w32(LOG_PTR, LOG_START));
}

/// Write a NUL-terminated string into scratch memory at `off` and return its address.
pub fn put_str(off: u32, s: &str) -> u32 {
    with_rt(|rt| {
        rt.mem.write(SCRATCH + off, s.as_bytes());
        rt.mem.w8(SCRATCH + off + s.len() as u32, 0);
    });
    SCRATCH + off
}

pub fn r32(a: u32) -> u32 {
    with_rt(|rt| rt.mem.r32(a))
}

pub fn w32(a: u32, v: u32) {
    with_rt(|rt| rt.mem.w32(a, v))
}

pub fn read(a: u32, n: usize) -> Vec<u8> {
    with_rt(|rt| rt.mem.slice(a, n).to_vec())
}

/// Little-endian dword `i` of a byte slice.
pub fn dw(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap())
}
