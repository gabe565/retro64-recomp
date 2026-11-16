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

pub const IID_IDIRECTDRAW4: [u8; 16] = [0x9a, 0x50, 0x59, 0x9c, 0xbd, 0x39, 0xd1, 0x11, 0x8c, 0x4a, 0x00, 0xc0, 0x4f, 0xd9, 0x30, 0xc5];
pub const IID_IDIRECT3D3: [u8; 16] = [0x40, 0x32, 0x22, 0xbb, 0x2b, 0xe7, 0xd0, 0x11, 0xa9, 0xb4, 0x00, 0xaa, 0x00, 0xc0, 0x99, 0x3e];

/// Scratch area reserved for helper outputs (well clear of test data).
const HELPER_OUT: u32 = SCRATCH + 0xf_0000;

pub fn put_bytes(off: u32, b: &[u8]) -> u32 {
    with_rt(|rt| rt.mem.write(SCRATCH + off, b));
    SCRATCH + off
}

/// `DirectDrawCreate` + `QueryInterface(IID_IDirectDraw4)`.
pub fn dd4(cpu: &mut Cpu) -> u32 {
    use bugatron_runtime::testing::{call_com, call_import};
    assert_eq!(call_import(cpu, "ddraw.dll", "DirectDrawCreate", &[0, HELPER_OUT, 0]), 0);
    let dd = r32(HELPER_OUT);
    with_rt(|rt| rt.mem.write(HELPER_OUT + 0x10, &IID_IDIRECTDRAW4));
    assert_eq!(call_com(cpu, dd, 0, &[HELPER_OUT + 0x10, HELPER_OUT]), 0);
    let dd4 = r32(HELPER_OUT);
    call_com(cpu, dd, 2, &[]);
    dd4
}

/// A zeroed `DDSURFACEDESC2` in scratch memory at `off` with dwSize set.
pub fn surface_desc(off: u32) -> u32 {
    let p = SCRATCH + off;
    with_rt(|rt| {
        rt.mem.slice_mut(p, 0x7c).fill(0);
        rt.mem.w32(p, 0x7c);
    });
    p
}

/// `CreateSurface` with the given desc; returns (HRESULT, surface).
pub fn create_surface(cpu: &mut Cpu, dd4: u32, desc: u32) -> (u32, u32) {
    let r = bugatron_runtime::testing::call_com(cpu, dd4, 6, &[desc, HELPER_OUT, 0]);
    (r, r32(HELPER_OUT))
}
