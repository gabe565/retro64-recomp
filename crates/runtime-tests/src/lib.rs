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
    rt.fmod.output = false; // never open a sound device in tests
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

/// Fullscreen 640×480 at `bpp` with r3d's flip chain and a Z16 buffer attached
/// to the back buffer. Returns (dd4, front, back, zbuffer).
pub fn display(cpu: &mut Cpu, bpp: u32) -> (u32, u32, u32, u32) {
    use bugatron_runtime::ddraw::pixel::PixelFormat;
    use bugatron_runtime::testing::call_com;
    let dd4 = dd4(cpu);
    assert_eq!(call_com(cpu, dd4, 20, &[0x0001_0010, 0x13]), 0);
    assert_eq!(call_com(cpu, dd4, 21, &[640, 480, bpp, 0, 0]), 0);
    let d = surface_desc(0x1000);
    w32(d + 4, 0x21);
    w32(d + 0x14, 1);
    w32(d + 0x68, 0x2218);
    let (r, front) = create_surface(cpu, dd4, d);
    assert_eq!(r, 0);
    w32(HELPER_OUT + 0x40, 4);
    assert_eq!(call_com(cpu, front, 12, &[HELPER_OUT + 0x40, HELPER_OUT + 0x44]), 0);
    let back = r32(HELPER_OUT + 0x44);
    let d = surface_desc(0x1000);
    w32(d + 4, 0x1007);
    w32(d + 8, 480);
    w32(d + 0xc, 640);
    with_rt(|rt| rt.mem.write(d + 0x48, &PixelFormat::Z16.to_ddpf()));
    w32(d + 0x68, 0x24000);
    let (r, z) = create_surface(cpu, dd4, d);
    assert_eq!(r, 0);
    assert_eq!(call_com(cpu, back, 3, &[z]), 0);
    (dd4, front, back, z)
}

/// `QueryInterface(IID_IDirect3D3)` on a DirectDraw4 object.
pub fn d3d3(cpu: &mut Cpu, dd4: u32) -> u32 {
    with_rt(|rt| rt.mem.write(HELPER_OUT + 0x10, &IID_IDIRECT3D3));
    assert_eq!(bugatron_runtime::testing::call_com(cpu, dd4, 0, &[HELPER_OUT + 0x10, HELPER_OUT]), 0);
    r32(HELPER_OUT)
}

/// Write `v` as consecutive f32 values at `a`.
pub fn put_f32s(a: u32, v: &[f32]) {
    with_rt(|rt| for (i, x) in v.iter().enumerate() { rt.mem.w32(a + 4 * i as u32, x.to_bits()) });
}

/// A RIFF WAVE file with PCM data (and an optional `smpl` loop).
pub fn wav(channels: u16, rate: u32, bits: u16, data: &[u8], smpl_loop: Option<(u32, u32)>) -> Vec<u8> {
    let mut chunks = Vec::new();
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(rate * channels as u32 * bits as u32 / 8).to_le_bytes());
    fmt.extend_from_slice(&(channels * bits / 8).to_le_bytes());
    fmt.extend_from_slice(&bits.to_le_bytes());
    for (id, body) in [(b"fmt ", fmt), (b"data", data.to_vec())] {
        chunks.extend_from_slice(id);
        chunks.extend_from_slice(&(body.len() as u32).to_le_bytes());
        chunks.extend_from_slice(&body);
        if body.len() % 2 == 1 {
            chunks.push(0);
        }
    }
    if let Some((s, e)) = smpl_loop {
        let mut b = vec![0u8; 36 + 24];
        b[28..32].copy_from_slice(&1u32.to_le_bytes());
        b[36 + 8..36 + 12].copy_from_slice(&s.to_le_bytes());
        b[36 + 12..36 + 16].copy_from_slice(&e.to_le_bytes());
        chunks.extend_from_slice(b"smpl");
        chunks.extend_from_slice(&(b.len() as u32).to_le_bytes());
        chunks.extend_from_slice(&b);
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(4 + chunks.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(&chunks);
    out
}

