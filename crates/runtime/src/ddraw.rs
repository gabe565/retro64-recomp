//! DirectDraw 6 / Direct3D 6 emulation at the COM boundary.
//!
//! The game's r3d wrapper (0x41e950–0x42a3ff) stays recompiled; this module
//! implements the DDRAW imports and the COM methods it calls
//! (docs/graphics/r3d-interface.md §5). Methods are implemented on evidence:
//! an unimplemented slot stops the run with its interface and method name.
//!
//! Methods that call back into guest code (the Enum* family) run without the
//! runtime borrowed, so dispatch here happens outside `with_rt`.

use crate::{call_guest_stdcall, fatal, with_rt, Cpu, Runtime};
use crate::ddraw_caps as caps;
use std::collections::HashMap;

/// Interface ids, encoded in COM thunk addresses (`COM_THUNK_BASE + id·0x1000 + slot·16`).
pub mod iface {
    pub const DIRECTDRAW4: u32 = 1;
    pub const DIRECTDRAW: u32 = 2;
    pub const DIRECT3D3: u32 = 3;
}

/// Interface and device GUIDs, in memory byte order.
mod guid {
    const fn g(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> [u8; 16] {
        let a = d1.to_le_bytes();
        let b = d2.to_le_bytes();
        let c = d3.to_le_bytes();
        [a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d4[0], d4[1], d4[2], d4[3], d4[4], d4[5], d4[6], d4[7]]
    }
    pub const IID_IDIRECTDRAW4: [u8; 16] = g(0x9c59509a, 0x39bd, 0x11d1, [0x8c, 0x4a, 0x00, 0xc0, 0x4f, 0xd9, 0x30, 0xc5]);
    pub const IID_IDIRECT3D3: [u8; 16] = g(0xbb223240, 0xe72b, 0x11d0, [0xa9, 0xb4, 0x00, 0xaa, 0x00, 0xc0, 0x99, 0x3e]);
    pub const IID_IDIRECT3DRGBDEVICE: [u8; 16] = g(0xa4665c60, 0x2673, 0x11cf, [0xa3, 0x1a, 0x00, 0xaa, 0x00, 0xb9, 0x33, 0x56]);
    pub const IID_IDIRECT3DHALDEVICE: [u8; 16] = g(0x84e63de0, 0x46aa, 0x11cf, [0x81, 0x6f, 0x00, 0x00, 0xc0, 0x20, 0x15, 0x6e]);
}

const DD_OK: u32 = 0;
const E_NOINTERFACE: u32 = 0x8000_4002;

/// Display modes offered to the game (width, height, bpp). Not the host's list:
/// a fixed, period-plausible set. 32-bit modes come first so the game's own
/// default rule (first enumerated 640-wide mode, WinMain 0x4047xx) selects
/// 640×480×32, the owner-decided default (r3d-interface.md §9.3).
const MODES: &[(u32, u32, u32)] = &[
    (640, 480, 32), (800, 600, 32), (1024, 768, 32), (1280, 1024, 32),
    (640, 480, 16), (800, 600, 16), (1024, 768, 16), (1280, 1024, 16),
];
/// Desktop mode reported by GetDisplayMode (32-bit, so windowed rendering is offered).
const DESKTOP: (u32, u32, u32) = (1024, 768, 32);

/// Vtable method names, for diagnostics.
fn method_name(iface: u32, slot: u32) -> &'static str {
    const UNKNOWN: [&str; 3] = ["QueryInterface", "AddRef", "Release"];
    const DD: &[&str] = &[
        "QueryInterface", "AddRef", "Release", "Compact", "CreateClipper", "CreatePalette", "CreateSurface",
        "DuplicateSurface", "EnumDisplayModes", "EnumSurfaces", "FlipToGDISurface", "GetCaps", "GetDisplayMode",
        "GetFourCCCodes", "GetGDISurface", "GetMonitorFrequency", "GetScanLine", "GetVerticalBlankStatus",
        "Initialize", "RestoreDisplayMode", "SetCooperativeLevel", "SetDisplayMode", "WaitForVerticalBlank",
        // IDirectDraw2+
        "GetAvailableVidMem",
        // IDirectDraw4
        "GetSurfaceFromDC", "RestoreAllSurfaces", "TestCooperativeLevel", "GetDeviceIdentifier",
    ];
    let table: &[&str] = match iface {
        iface::DIRECTDRAW4 => DD,
        iface::DIRECTDRAW => &DD[..23],
        iface::DIRECT3D3 => &[
            "QueryInterface", "AddRef", "Release", "EnumDevices", "CreateLight", "CreateMaterial", "CreateViewport",
            "FindDevice", "CreateDevice", "CreateVertexBuffer", "EnumZBufferFormats", "EvictManagedTextures",
        ],
        _ => &UNKNOWN,
    };
    table.get(slot as usize).copied().unwrap_or("<beyond vtable>")
}

fn iface_name(iface: u32) -> &'static str {
    match iface {
        iface::DIRECTDRAW4 => "IDirectDraw4",
        iface::DIRECTDRAW => "IDirectDraw",
        iface::DIRECT3D3 => "IDirect3D3",
        _ => "<unknown interface>",
    }
}

/// Number of vtable slots allocated per interface.
fn slot_count(iface: u32) -> u32 {
    match iface {
        iface::DIRECTDRAW4 => 28,
        iface::DIRECTDRAW => 23,
        iface::DIRECT3D3 => 12,
        _ => 3,
    }
}

#[derive(Debug)]
pub(crate) enum Kind {
    DirectDraw,
    Direct3D,
}

pub(crate) struct ComObj {
    pub iface: u32,
    pub refs: u32,
    #[allow(dead_code)] // read once surfaces and devices land
    pub kind: Kind,
}

#[derive(Default)]
pub(crate) struct ComRegistry {
    objs: HashMap<u32, ComObj>,
}

impl Runtime {
    /// Create a guest-visible COM object of `iface` and register it.
    pub(crate) fn new_com(&mut self, iface: u32, kind: Kind) -> u32 {
        let p = self.create_com_object(iface, slot_count(iface));
        self.com.objs.insert(p, ComObj { iface, refs: 1, kind });
        p
    }
}

impl ComRegistry {
    fn get(&self, this: u32) -> &ComObj {
        self.objs.get(&this).unwrap_or_else(|| fatal(&format!("COM call on unknown object {this:#x}")))
    }
}

// ---- guest structures ----------------------------------------------------------------

/// `DDSURFACEDESC2` (0x7c bytes; the first 0x6c are `DDSURFACEDESC`) for a display
/// mode, laid out as DirectDraw reports it (captured in the VM fixture).
fn mode_desc(w: u32, h: u32, bpp: u32) -> [u8; 0x7c] {
    let mut d = [0u32; 31];
    d[0] = 0x7c;
    d[1] = 0x100f; // CAPS | HEIGHT | WIDTH | PITCH | PIXELFORMAT
    d[2] = h;
    d[3] = w;
    d[4] = w * bpp / 8;
    // DDPIXELFORMAT at +0x48
    d[18] = 0x20;
    d[19] = 0x40; // DDPF_RGB
    d[21] = bpp;
    let (r, g, b) = if bpp == 16 { (0xf800, 0x7e0, 0x1f) } else { (0xff_0000, 0xff00, 0xff) };
    d[22] = r;
    d[23] = g;
    d[24] = b;
    d[26] = 0x1000_e27c; // ddsCaps as reported by the reference driver
    let mut out = [0u8; 0x7c];
    for (i, v) in d.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

fn read_guid(p: u32) -> [u8; 16] {
    with_rt(|rt| rt.mem.slice(p, 16).try_into().unwrap())
}

/// Copy `src` into a caller structure whose first dword is its size, honouring
/// the caller's size (DirectX structures are versioned by dwSize).
fn fill_sized(p: u32, src: &[u8]) {
    if p == 0 {
        return;
    }
    with_rt(|rt| {
        let size = (rt.mem.r32(p) as usize).min(src.len());
        rt.mem.write(p, &src[..size]);
        rt.mem.w32(p, size as u32);
    });
}

// ---- helpers ---------------------------------------------------------------------

fn arg(cpu: &Cpu, i: u32) -> u32 {
    with_rt(|rt| rt.mem.r32(cpu.esp + 4 + 4 * i))
}

fn ret(cpu: &mut Cpu, value: u32, nargs: u32) {
    cpu.eax = value;
    cpu.esp += 4 + 4 * nargs;
}

/// Allocate a NUL-terminated string in guest memory (kept for the run).
fn guest_str(s: &str) -> u32 {
    with_rt(|rt| rt.alloc_cstr(s))
}

// ---- imports ---------------------------------------------------------------------

/// DDRAW.DLL imports. Returns false if `name` is not handled here.
pub(crate) fn import(cpu: &mut Cpu, name: &str) -> bool {
    match name {
        "DirectDrawEnumerateA" => {
            // (LPDDENUMCALLBACKA cb, LPVOID ctx). One driver: the primary display,
            // reported exactly as DirectDraw does (NULL GUID, "Primary Display Driver", "display").
            let (cb, ctx) = (arg(cpu, 0), arg(cpu, 1));
            let (desc, drv) = (guest_str("Primary Display Driver"), guest_str("display"));
            call_guest_stdcall(cpu, cb, &[0, desc, drv, ctx]);
            ret(cpu, DD_OK, 2);
        }
        "DirectDrawCreate" => {
            // (GUID* driver, LPDIRECTDRAW* out, IUnknown* outer). Only the primary driver exists.
            let (drv, out) = (arg(cpu, 0), arg(cpu, 1));
            if drv != 0 {
                fatal("DirectDrawCreate: only the primary display driver (NULL GUID) is emulated");
            }
            with_rt(|rt| {
                let dd = rt.new_com(iface::DIRECTDRAW, Kind::DirectDraw);
                rt.mem.w32(out, dd);
            });
            ret(cpu, DD_OK, 3);
        }
        _ => return false,
    }
    true
}

// ---- COM dispatch ------------------------------------------------------------------

pub(crate) fn dispatch(cpu: &mut Cpu, iface: u32, slot: u32) {
    with_rt(|rt| *rt.diag.com_calls.entry(format!("{}::{}", iface_name(iface), method_name(iface, slot))).or_default() += 1);
    let this = arg(cpu, 0);
    match (iface, slot) {
        // IUnknown::QueryInterface(this, REFIID, void**)
        (_, 0) => {
            let (riid, out) = (arg(cpu, 1), arg(cpu, 2));
            let id = read_guid(riid);
            let new_iface = match (with_rt(|rt| rt.com.get(this).iface), id) {
                (iface::DIRECTDRAW | iface::DIRECTDRAW4, guid::IID_IDIRECTDRAW4) => Some((iface::DIRECTDRAW4, Kind::DirectDraw)),
                (iface::DIRECTDRAW | iface::DIRECTDRAW4, guid::IID_IDIRECT3D3) => Some((iface::DIRECT3D3, Kind::Direct3D)),
                _ => None,
            };
            match new_iface {
                Some((i, k)) => {
                    with_rt(|rt| {
                        let o = rt.new_com(i, k);
                        rt.mem.w32(out, o);
                    });
                    ret(cpu, DD_OK, 3);
                }
                None => {
                    with_rt(|rt| rt.mem.w32(out, 0));
                    eprintln!("[ddraw] {}::QueryInterface for unsupported {:02x?}", iface_name(iface), id);
                    ret(cpu, E_NOINTERFACE, 3);
                }
            }
        }
        // IUnknown::AddRef / Release
        (_, 1) | (_, 2) => {
            let n = with_rt(|rt| {
                let o = rt.com.objs.get_mut(&this).unwrap_or_else(|| fatal(&format!("AddRef/Release on unknown object {this:#x}")));
                if slot == 1 { o.refs += 1 } else { o.refs -= 1 }
                let n = o.refs;
                if n == 0 {
                    rt.com.objs.remove(&this); // the guest block is not reused
                }
                n
            });
            ret(cpu, n, 1);
        }
        // IDirectDraw4::GetCaps(this, LPDDCAPS hal, LPDDCAPS hel)
        (iface::DIRECTDRAW | iface::DIRECTDRAW4, 11) => {
            fill_sized(arg(cpu, 1), &caps::HAL_DDCAPS);
            fill_sized(arg(cpu, 2), &caps::HEL_DDCAPS);
            ret(cpu, DD_OK, 3);
        }
        // IDirectDraw[4]::GetDisplayMode(this, LPDDSURFACEDESC[2])
        (iface::DIRECTDRAW | iface::DIRECTDRAW4, 12) => {
            let (w, h, bpp) = DESKTOP;
            fill_sized(arg(cpu, 1), &mode_desc(w, h, bpp));
            ret(cpu, DD_OK, 2);
        }
        // IDirectDraw4::EnumDisplayModes(this, flags, LPDDSURFACEDESC2 filter, ctx, callback)
        (iface::DIRECTDRAW4, 8) => {
            let (flags, filter, ctx, cb) = (arg(cpu, 1), arg(cpu, 2), arg(cpu, 3), arg(cpu, 4));
            if flags != 0 || filter != 0 {
                fatal("EnumDisplayModes: flags/filter not emulated (the game passes none)");
            }
            let buf = with_rt(|rt| rt.heap_alloc(0x7c, true));
            for &(w, h, bpp) in MODES {
                with_rt(|rt| rt.mem.write(buf, &mode_desc(w, h, bpp)));
                if call_guest_stdcall(cpu, cb, &[buf, ctx]) == 0 {
                    break; // DDENUMRET_CANCEL
                }
            }
            with_rt(|rt| rt.heap.free(buf));
            ret(cpu, DD_OK, 5);
        }
        // IDirect3D3::EnumDevices(this, callback, ctx)
        (iface::DIRECT3D3, 3) => {
            let (cb, ctx) = (arg(cpu, 1), arg(cpu, 2));
            // Both devices of the reference driver, in its order (software first).
            let none = [0u8; 0xfc];
            let devices: [(&[u8; 16], &str, &str, &[u8; 0xfc], &[u8; 0xfc]); 2] = [
                (&guid::IID_IDIRECT3DRGBDEVICE, "Microsoft Direct3D RGB Software Emulation", "RGB Emulation", &none, &caps::RGB_DEVICE_DESC),
                (&guid::IID_IDIRECT3DHALDEVICE, "Microsoft Direct3D Hardware acceleration through Direct3D HAL", "Direct3D HAL", &caps::HAL_DEVICE_DESC, &none),
            ];
            for (g, desc, name, hal, hel) in devices {
                let ptrs = with_rt(|rt| {
                    let gp = rt.heap_alloc(16, false);
                    rt.mem.write(gp, g);
                    let (dp, np) = (rt.alloc_cstr(desc), rt.alloc_cstr(name));
                    let hp = rt.heap_alloc(0xfc, false);
                    let mut h = *hal;
                    h[..4].copy_from_slice(&0xfcu32.to_le_bytes());
                    rt.mem.write(hp, &h);
                    let ep = rt.heap_alloc(0xfc, false);
                    let mut e = *hel;
                    e[..4].copy_from_slice(&0xfcu32.to_le_bytes());
                    rt.mem.write(ep, &e);
                    [gp, dp, np, hp, ep, ctx]
                });
                if call_guest_stdcall(cpu, cb, &ptrs) == 0 {
                    break;
                }
            }
            ret(cpu, DD_OK, 3);
        }
        // IDirectDraw4::GetMonitorFrequency(this, LPDWORD)
        (iface::DIRECTDRAW4, 15) => {
            let p = arg(cpu, 1);
            with_rt(|rt| rt.mem.w32(p, rt.clock.monitor_frequency));
            ret(cpu, 0, 2);
        }
        _ => fatal(&format!(
            "unimplemented COM method {}::{} (slot {slot}, vtable offset {:#x}, this={:#x})",
            iface_name(iface), method_name(iface, slot), slot * 4, arg(cpu, 0)
        )),
    }
}
