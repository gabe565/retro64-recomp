//! `IDirectDraw`, `IDirectDraw4` and `IDirect3D3` methods.

use super::*;
use super::surface::{caps as sc, ddsd};
use super::pixel::PixelFormat;

/// Returns false if the method is not implemented.
pub(super) fn call(cpu: &mut Cpu, iface: u32, _this: u32, slot: u32) -> bool {
    match (iface, slot) {
        // IUnknown::QueryInterface(this, REFIID, void**)
        (_, 0) => {
            let (riid, out) = (arg(cpu, 1), arg(cpu, 2));
            let id = read_guid(riid);
            let obj = match (iface, id) {
                (iface::DIRECTDRAW | iface::DIRECTDRAW4, guid::IID_IDIRECTDRAW4) => Some(with_rt(|rt| rt.new_com(iface::DIRECTDRAW4, Kind::DirectDraw))),
                (iface::DIRECTDRAW | iface::DIRECTDRAW4, guid::IID_IDIRECT3D3) => Some(with_rt(|rt| rt.new_com(iface::DIRECT3D3, Kind::Direct3D))),
                _ => None,
            };
            qi_result(cpu, iface, id, out, obj);
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
            for (w, h, bpp) in with_rt(|rt| rt.com.modes()) {
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
            // (GUID, description, name, HAL caps, HEL caps)
            type DeviceInfo<'a> = (&'a [u8; 16], &'a str, &'a str, &'a [u8; 0xfc], &'a [u8; 0xfc]);
            let devices: [DeviceInfo; 2] = [
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
        (iface::DIRECT3D3, 4 | 5 | 6 | 8 | 11) => return super::d3d::call_d3d(cpu, _this, slot),
        // IDirect3D3::EnumZBufferFormats(this, REFCLSID device, callback, ctx)
        (iface::DIRECT3D3, 10) => {
            let (cb, ctx) = (arg(cpu, 2), arg(cpu, 3));
            // 16, 24 and 32-bit Z in the usual driver order. The game's callback
            // (0x425620) stops at the first 16-bit format: the Z16 it used in the VM.
            let formats = [
                PixelFormat::Z16,
                PixelFormat { bpp: 32, a: 0x00ff_ffff, ..PixelFormat::Z16 },
                PixelFormat { bpp: 32, a: 0xffff_ffff, ..PixelFormat::Z16 },
            ];
            let buf = with_rt(|rt| rt.heap_alloc(32, true));
            for f in formats {
                with_rt(|rt| rt.mem.write(buf, &f.to_ddpf()));
                if call_guest_stdcall(cpu, cb, &[buf, ctx]) == 0 {
                    break;
                }
            }
            with_rt(|rt| rt.heap.free(buf));
            ret(cpu, DD_OK, 4);
        }
        // IDirectDraw4::SetCooperativeLevel(this, hwnd, flags)
        (iface::DIRECTDRAW4, 20) => {
            let flags = arg(cpu, 2);
            with_rt(|rt| {
                rt.com.coop = flags;
                if flags & 0x11 == 0x11 {
                    // DDSCL_FULLSCREEN | DDSCL_EXCLUSIVE
                    crate::user::exclusive_fullscreen_rt(rt);
                }
            });
            ret(cpu, DD_OK, 3);
        }
        // IDirectDraw4::SetDisplayMode(this, w, h, bpp, refresh, flags)
        (iface::DIRECTDRAW4, 21) => {
            let (w, h, bpp) = (arg(cpu, 1), arg(cpu, 2), arg(cpu, 3));
            if !with_rt(|rt| rt.com.modes().contains(&(w, h, bpp))) {
                ret(cpu, DDERR_INVALIDMODE, 6);
                return true;
            }
            with_rt(|rt| {
                rt.com.mode = Some((w, h, bpp));
                rt.user.screen = (w as i32, h as i32);
            });
            ret(cpu, DD_OK, 6);
        }
        // IDirectDraw4::WaitForVerticalBlank(this, flags, event)
        (iface::DIRECTDRAW4, 22) => {
            let flags = arg(cpu, 1);
            if flags != 1 {
                fatal(&format!("WaitForVerticalBlank: flags {flags:#x} not emulated (the game uses DDWAITVB_BLOCKBEGIN)"));
            }
            with_rt(crate::clock::vblank_wait);
            ret(cpu, DD_OK, 3);
        }
        // IDirectDraw4::RestoreDisplayMode / FlipToGDISurface / RestoreAllSurfaces
        (iface::DIRECTDRAW4, 19) | (iface::DIRECTDRAW4, 10) | (iface::DIRECTDRAW4, 25) => ret(cpu, DD_OK, 1),
        // IDirectDraw4::CreateClipper(this, flags, LPDIRECTDRAWCLIPPER*, outer)
        (iface::DIRECTDRAW4, 4) => {
            let out = arg(cpu, 2);
            with_rt(|rt| {
                let c = rt.new_com(iface::CLIPPER, Kind::Clipper { hwnd: 0 });
                rt.mem.w32(out, c);
            });
            ret(cpu, DD_OK, 4);
        }
        // IDirectDraw4::CreateSurface(this, LPDDSURFACEDESC2, LPDIRECTDRAWSURFACE4*, outer)
        (iface::DIRECTDRAW4, 6) => {
            let (desc, out) = (arg(cpu, 1), arg(cpu, 2));
            let r = with_rt(|rt| create_surface(rt, desc));
            match r {
                Ok(s) => {
                    with_rt(|rt| rt.mem.w32(out, s));
                    ret(cpu, DD_OK, 4);
                }
                Err(e) => {
                    with_rt(|rt| rt.mem.w32(out, 0));
                    ret(cpu, e, 4);
                }
            }
        }
        _ => return false,
    }
    true
}

/// Allocate guest pixel memory for `s` and register it as a COM surface.
pub(crate) fn register_surface(rt: &mut Runtime, mut s: Surface) -> u32 {
    s.bits = rt.heap_alloc(s.size_bytes().max(4), true);
    let obj = rt.new_com(iface::SURFACE4, Kind::Surface);
    rt.com.surfaces.insert(obj, s);
    obj
}

/// `CreateSurface`: the primary flip chain, Z-buffers, textures and offscreen surfaces.
pub(crate) fn create_surface(rt: &mut Runtime, desc: u32) -> Result<u32, u32> {
    let d = |i: u32| rt.mem.r32(desc + 4 * i);
    let (flags, caps, caps2) = (d(1), d(26), d(27));
    if flags & ddsd::CAPS == 0 {
        return Err(DDERR_INVALIDPARAMS);
    }
    let pf = (flags & ddsd::PIXELFORMAT != 0).then(|| PixelFormat::from_ddpf(rt.mem.slice(desc + 0x48, 32)));
    let mem_caps = if caps & sc::SYSTEMMEMORY != 0 { 0 } else { sc::VIDEOMEMORY | sc::LOCALVIDMEM };
    if caps & sc::PRIMARYSURFACE != 0 {
        let (w, h, bpp) = rt.com.mode.unwrap_or(DESKTOP);
        let fmt = if bpp == 16 { PixelFormat::RGB565 } else { PixelFormat::XRGB8888 };
        let backs = if flags & ddsd::BACKBUFFERCOUNT != 0 { d(5) } else { 0 };
        let mut front_caps = caps | mem_caps | sc::VISIBLE;
        if backs > 0 {
            front_caps |= sc::FRONTBUFFER;
        }
        let front = register_surface(rt, Surface::new(w, h, fmt, front_caps, caps2, 0));
        let mut prev = front;
        for i in 0..backs {
            // Back buffers inherit the chain's capabilities minus primary/visible.
            let bcaps = (caps & !(sc::PRIMARYSURFACE | sc::VISIBLE)) | mem_caps | if i == 0 { sc::BACKBUFFER } else { 0 };
            let mut b = Surface::new(w, h, fmt, bcaps, caps2, 0);
            b.implicit = true;
            let bo = register_surface(rt, b);
            rt.com.surface_mut(prev).attached.push(bo);
            prev = bo;
        }
        if backs > 0 {
            rt.com.surface_mut(prev).attached.push(front); // the chain is circular, as in DirectDraw
        }
        rt.com.primary = Some(front);
        return Ok(front);
    }
    if flags & (ddsd::WIDTH | ddsd::HEIGHT) != (ddsd::WIDTH | ddsd::HEIGHT) {
        return Err(DDERR_INVALIDPARAMS);
    }
    let (w, h) = (d(3), d(2));
    if w == 0 || h == 0 || w > 4096 || h > 4096 {
        return Err(DDERR_INVALIDPARAMS);
    }
    let fmt = if caps & sc::ZBUFFER != 0 {
        match pf {
            Some(f) if f.is_depth() => f,
            _ if flags & ddsd::ZBUFFERBITDEPTH != 0 => PixelFormat { bpp: d(6), a: (1u64 << d(6)).wrapping_sub(1) as u32, ..PixelFormat::Z16 },
            _ => return Err(DDERR_INVALIDPARAMS),
        }
    } else {
        pf.unwrap_or_else(|| match rt.com.mode.unwrap_or(DESKTOP).2 {
            16 => PixelFormat::RGB565,
            _ => PixelFormat::XRGB8888,
        })
    };
    if flags & ddsd::MIPMAPCOUNT != 0 && d(6) > 1 {
        fatal("CreateSurface: mipmapped surfaces are not emulated (the game disables mipmaps)");
    }
    let mut s = Surface::new(w, h, fmt, caps | mem_caps, caps2, 0);
    if flags & ddsd::CKSRCBLT != 0 {
        s.ck_src = Some((d(16), d(17)));
    }
    Ok(register_surface(rt, s))
}
