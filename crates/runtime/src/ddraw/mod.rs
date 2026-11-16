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
use std::collections::HashMap;

pub mod caps;
pub mod d3d;
mod dd;
pub mod pixel;
pub mod surface;

use surface::Surface;

/// Interface ids, encoded in COM thunk addresses (`COM_THUNK_BASE + id·0x1000 + slot·16`).
pub mod iface {
    pub const DIRECTDRAW4: u32 = 1;
    pub const DIRECTDRAW: u32 = 2;
    pub const DIRECT3D3: u32 = 3;
    pub const SURFACE4: u32 = 4;
    pub const CLIPPER: u32 = 5;
    pub const DEVICE3: u32 = 6;
    pub const VIEWPORT3: u32 = 7;
    pub const LIGHT: u32 = 8;
    pub const MATERIAL3: u32 = 9;
    pub const TEXTURE2: u32 = 10;
}

/// Interface and device GUIDs, in memory byte order.
pub(crate) mod guid {
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
    pub const IID_IDIRECT3DTEXTURE2: [u8; 16] = g(0x93281502, 0x8cf8, 0x11d0, [0x89, 0xab, 0x00, 0xa0, 0xc9, 0x05, 0x41, 0x29]);
    pub const IID_IDIRECTDRAWSURFACE4: [u8; 16] = g(0x0b2b8630, 0xad35, 0x11d0, [0x8e, 0xa6, 0x00, 0x60, 0x97, 0x97, 0xea, 0x5b]);
}

pub const DD_OK: u32 = 0;
pub const E_NOINTERFACE: u32 = 0x8000_4002;
pub const DDERR_INVALIDPARAMS: u32 = 0x8007_0057;
pub const DDERR_INVALIDRECT: u32 = 0x8876_0096;
pub const DDERR_NOTFOUND: u32 = 0x8876_00ff;
pub const DDERR_NOTLOCKED: u32 = 0x8876_0248;
pub const DDERR_INVALIDMODE: u32 = 0x8876_0078;
pub const DDERR_NOCOLORKEY: u32 = 0x8876_00d7;

/// Display modes offered to the game (width, height, bpp): the set a typical
/// 2001 display driver offered, not the host's list. The original accepts any
/// enumerated mode, but its fullscreen projection assumes 4:3 (0x421fd0 fixes
/// the aspect at 0.75), so the list is 4:3 apart from the period-common 5:4
/// 1280×1024, which the original stretches as it did on real hardware.
/// 32-bit modes come first so the game's own default rule (first enumerated
/// 640-wide mode, WinMain 0x4047xx) selects 640×480×32, the owner-decided
/// default (r3d-interface.md §9.3).
pub const MODES: &[(u32, u32, u32)] = &[
    (640, 480, 32), (512, 384, 32), (800, 600, 32), (1024, 768, 32), (1152, 864, 32), (1280, 960, 32),
    (1280, 1024, 32), (1600, 1200, 32), (1920, 1440, 32), (2048, 1536, 32),
    (640, 480, 16), (512, 384, 16), (800, 600, 16), (1024, 768, 16), (1152, 864, 16), (1280, 960, 16),
    (1280, 1024, 16), (1600, 1200, 16), (1920, 1440, 16), (2048, 1536, 16),
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
        iface::SURFACE4 => &[
            "QueryInterface", "AddRef", "Release", "AddAttachedSurface", "AddOverlayDirtyRect", "Blt", "BltBatch",
            "BltFast", "DeleteAttachedSurface", "EnumAttachedSurfaces", "EnumOverlayZOrders", "Flip",
            "GetAttachedSurface", "GetBltStatus", "GetCaps", "GetClipper", "GetColorKey", "GetDC", "GetFlipStatus",
            "GetOverlayPosition", "GetPalette", "GetPixelFormat", "GetSurfaceDesc", "Initialize", "IsLost", "Lock",
            "ReleaseDC", "Restore", "SetClipper", "SetColorKey", "SetOverlayPosition", "SetPalette", "Unlock",
            "UpdateOverlay", "UpdateOverlayDisplay", "UpdateOverlayZOrder", "GetDDInterface", "PageLock",
            "PageUnlock", "SetSurfaceDesc", "SetPrivateData", "GetPrivateData", "FreePrivateData",
            "GetUniquenessValue", "ChangeUniquenessValue",
        ],
        iface::CLIPPER => &[
            "QueryInterface", "AddRef", "Release", "GetClipList", "GetHWnd", "Initialize", "IsClipListChanged",
            "SetClipList", "SetHWnd",
        ],
        iface::DEVICE3 => &[
            "QueryInterface", "AddRef", "Release", "GetCaps", "GetStats", "AddViewport", "DeleteViewport",
            "NextViewport", "EnumTextureFormats", "BeginScene", "EndScene", "GetDirect3D", "SetCurrentViewport",
            "GetCurrentViewport", "SetRenderTarget", "GetRenderTarget", "Begin", "BeginIndexed", "Vertex", "Index",
            "End", "GetRenderState", "SetRenderState", "GetLightState", "SetLightState", "SetTransform",
            "GetTransform", "MultiplyTransform", "DrawPrimitive", "DrawIndexedPrimitive", "SetClipStatus",
            "GetClipStatus", "DrawPrimitiveStrided", "DrawIndexedPrimitiveStrided", "DrawPrimitiveVB",
            "DrawIndexedPrimitiveVB", "ComputeSphereVisibility", "GetTexture", "SetTexture",
            "GetTextureStageState", "SetTextureStageState", "ValidateDevice",
        ],
        iface::VIEWPORT3 => &[
            "QueryInterface", "AddRef", "Release", "Initialize", "GetViewport", "SetViewport", "TransformVertices",
            "LightElements", "SetBackground", "GetBackground", "SetBackgroundDepth", "GetBackgroundDepth", "Clear",
            "AddLight", "DeleteLight", "NextLight", "GetViewport2", "SetViewport2", "SetBackgroundDepth2",
            "GetBackgroundDepth2", "Clear2",
        ],
        iface::LIGHT => &["QueryInterface", "AddRef", "Release", "Initialize", "SetLight", "GetLight"],
        iface::MATERIAL3 => &["QueryInterface", "AddRef", "Release", "SetMaterial", "GetMaterial", "GetHandle"],
        iface::TEXTURE2 => &["QueryInterface", "AddRef", "Release", "GetHandle", "PaletteChanged", "Load"],
        _ => &UNKNOWN,
    };
    table.get(slot as usize).copied().unwrap_or("<beyond vtable>")
}

/// "Interface::Method" for diagnostics.
pub fn method_label(iface: u32, slot: u32) -> String {
    format!("{}::{}", iface_name(iface), method_name(iface, slot))
}

fn iface_name(iface: u32) -> &'static str {
    match iface {
        iface::DIRECTDRAW4 => "IDirectDraw4",
        iface::DIRECTDRAW => "IDirectDraw",
        iface::DIRECT3D3 => "IDirect3D3",
        iface::SURFACE4 => "IDirectDrawSurface4",
        iface::CLIPPER => "IDirectDrawClipper",
        iface::DEVICE3 => "IDirect3DDevice3",
        iface::VIEWPORT3 => "IDirect3DViewport3",
        iface::LIGHT => "IDirect3DLight",
        iface::MATERIAL3 => "IDirect3DMaterial3",
        iface::TEXTURE2 => "IDirect3DTexture2",
        _ => "<unknown interface>",
    }
}

/// Number of vtable slots allocated per interface.
fn slot_count(iface: u32) -> u32 {
    match iface {
        iface::DIRECTDRAW4 => 28,
        iface::DIRECTDRAW => 23,
        iface::DIRECT3D3 => 12,
        iface::SURFACE4 => 45,
        iface::CLIPPER => 9,
        iface::DEVICE3 => 42,
        iface::VIEWPORT3 => 21,
        iface::LIGHT => 6,
        iface::MATERIAL3 => 6,
        iface::TEXTURE2 => 6,
        _ => 3,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    DirectDraw,
    Direct3D,
    /// Pixels and state in [`Registry::surfaces`] under the same object address.
    Surface,
    Clipper { hwnd: u32 },
    Device,
    Viewport,
    Light,
    Material,
    Texture,
}

#[derive(Debug)]
pub struct ComObj {
    pub iface: u32,
    pub refs: u32,
    pub kind: Kind,
}

/// Emulated DirectDraw/Direct3D state: COM objects, surfaces, and the display.
#[derive(Default)]
pub struct Registry {
    pub objs: HashMap<u32, ComObj>,
    pub surfaces: HashMap<u32, Surface>,
    /// `SetCooperativeLevel` flags (`DDSCL_*`).
    pub coop: u32,
    /// Current guest display mode set with `SetDisplayMode` (width, height, bpp).
    pub mode: Option<(u32, u32, u32)>,
    /// The primary surface, if created.
    pub primary: Option<u32>,
    /// Number of `Flip` calls (frame boundaries of the original game loop).
    pub flips: u64,
    /// Direct3D objects and recorded rendering commands.
    pub d3d: d3d::State,
}

impl Runtime {
    /// Create a guest-visible COM object of `iface` and register it.
    pub(crate) fn new_com(&mut self, iface: u32, kind: Kind) -> u32 {
        let p = self.create_com_object(iface, slot_count(iface));
        self.com.objs.insert(p, ComObj { iface, refs: 1, kind });
        p
    }
}

impl Registry {
    pub fn get(&self, this: u32) -> &ComObj {
        self.objs.get(&this).unwrap_or_else(|| fatal(&format!("COM call on unknown object {this:#x}")))
    }

    pub fn surface(&self, this: u32) -> &Surface {
        self.surfaces.get(&this).unwrap_or_else(|| fatal(&format!("not a surface: {this:#x}")))
    }

    pub fn surface_mut(&mut self, this: u32) -> &mut Surface {
        self.surfaces.get_mut(&this).unwrap_or_else(|| fatal(&format!("not a surface: {this:#x}")))
    }
}

// ---- guest structures ----------------------------------------------------------------

/// `DDSURFACEDESC2` (0x7c bytes; the first 0x6c are `DDSURFACEDESC`) for a display
/// mode, laid out as DirectDraw reports it (captured in the VM fixture).
pub(crate) fn mode_desc(w: u32, h: u32, bpp: u32) -> [u8; 0x7c] {
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

pub(crate) fn read_guid(p: u32) -> [u8; 16] {
    with_rt(|rt| rt.mem.slice(p, 16).try_into().unwrap())
}

/// Copy `src` into a caller structure whose first dword is its size, honouring
/// the caller's size (DirectX structures are versioned by dwSize).
pub(crate) fn fill_sized(p: u32, src: &[u8]) {
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

pub(crate) fn arg(cpu: &Cpu, i: u32) -> u32 {
    with_rt(|rt| rt.mem.r32(cpu.esp + 4 + 4 * i))
}

pub(crate) fn ret(cpu: &mut Cpu, value: u32, nargs: u32) {
    cpu.eax = value;
    cpu.esp += 4 + 4 * nargs;
}

/// Allocate a NUL-terminated string in guest memory (kept for the run).
pub(crate) fn guest_str(s: &str) -> u32 {
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
    with_rt(|rt| *rt.diag.com_counts.entry((iface, slot)).or_default() += 1);
    let this = arg(cpu, 0);
    let handled = match (iface, slot) {
        (_, 1) | (_, 2) => {
            add_ref_release(cpu, this, slot == 1);
            true
        }
        (iface::DIRECTDRAW | iface::DIRECTDRAW4 | iface::DIRECT3D3, _) => dd::call(cpu, iface, this, slot),
        (iface::SURFACE4 | iface::CLIPPER, _) => surface::call(cpu, iface, this, slot),
        (iface::DEVICE3 | iface::VIEWPORT3 | iface::LIGHT | iface::MATERIAL3 | iface::TEXTURE2, _) => d3d::call(cpu, iface, this, slot),
        _ => false,
    };
    if !handled {
        fatal(&format!(
            "unimplemented COM method {}::{} (slot {slot}, vtable offset {:#x}, this={:#x})",
            iface_name(iface), method_name(iface, slot), slot * 4, this
        ));
    }
}

fn add_ref_release(cpu: &mut Cpu, this: u32, add: bool) {
    let n = with_rt(|rt| {
        let Some(o) = rt.com.objs.get_mut(&this) else {
            fatal(&format!("AddRef/Release on unknown object {this:#x}"))
        };
        if add { o.refs += 1 } else { o.refs -= 1 }
        let n = o.refs;
        if n == 0 {
            if let Some(o) = rt.com.objs.remove(&this) {
                destroy_object(rt, this, o.kind);
            }
        }
        n
    });
    ret(cpu, n, 1);
}

/// Free the emulation state of an object whose last reference went away.
pub(crate) fn destroy_object(rt: &mut Runtime, this: u32, kind: Kind) {
    match kind {
        Kind::Surface => surface::destroy(rt, this),
        Kind::Device => {
            rt.com.d3d.devices.remove(&this);
        }
        Kind::Viewport => {
            rt.com.d3d.viewports.remove(&this);
        }
        Kind::Light => {
            rt.com.d3d.lights.remove(&this);
        }
        Kind::Material => {
            rt.com.d3d.materials.remove(&this);
            rt.com.d3d.material_handles.retain(|_, &mut o| o != this);
        }
        Kind::Texture => {
            rt.com.d3d.texture_handles.retain(|_, &mut o| o != this);
            if let Some(s) = rt.com.d3d.textures.remove(&this) {
                if let Some(o) = rt.com.objs.get_mut(&s) {
                    o.refs -= 1;
                    if o.refs == 0 {
                        rt.com.objs.remove(&s);
                        surface::destroy(rt, s);
                    }
                }
            }
        }
        Kind::DirectDraw | Kind::Direct3D | Kind::Clipper { .. } => {}
    }
}

/// Write `obj` to `out` and return DD_OK, or clear `out` and log for unsupported interfaces.
pub(crate) fn qi_result(cpu: &mut Cpu, iface_from: u32, id: [u8; 16], out: u32, obj: Option<u32>) {
    match obj {
        Some(o) => {
            with_rt(|rt| rt.mem.w32(out, o));
            ret(cpu, DD_OK, 3);
        }
        None => {
            with_rt(|rt| rt.mem.w32(out, 0));
            eprintln!("[ddraw] {}::QueryInterface for unsupported {:02x?}", iface_name(iface_from), id);
            ret(cpu, E_NOINTERFACE, 3);
        }
    }
}
