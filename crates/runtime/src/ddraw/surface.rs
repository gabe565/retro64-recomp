//! DirectDraw surfaces whose pixels live in guest memory.
//!
//! Every surface owns a guest-memory pixel buffer in its DirectDraw format, so
//! `Lock` hands game code a pointer with exactly the layout and pitch it
//! expects. Render targets may additionally be GPU-resident; the renderer
//! keeps the two coherent through [`super::Registry::sync_for_cpu`] and
//! [`Surface::generation`] (bumped on every CPU-side modification).

use super::pixel::PixelFormat;

/// `DDSCAPS_*` values used by the game.
pub mod caps {
    pub const BACKBUFFER: u32 = 0x4;
    pub const COMPLEX: u32 = 0x8;
    pub const FLIP: u32 = 0x10;
    pub const FRONTBUFFER: u32 = 0x20;
    pub const OFFSCREENPLAIN: u32 = 0x40;
    pub const PRIMARYSURFACE: u32 = 0x200;
    pub const SYSTEMMEMORY: u32 = 0x800;
    pub const TEXTURE: u32 = 0x1000;
    pub const DEVICE3D: u32 = 0x2000;
    pub const VIDEOMEMORY: u32 = 0x4000;
    pub const VISIBLE: u32 = 0x8000;
    pub const ZBUFFER: u32 = 0x20000;
    pub const MIPMAP: u32 = 0x40_0000;
    pub const LOCALVIDMEM: u32 = 0x1000_0000;
}

/// `DDSD_*` flags.
pub mod ddsd {
    pub const CAPS: u32 = 0x1;
    pub const HEIGHT: u32 = 0x2;
    pub const WIDTH: u32 = 0x4;
    pub const PITCH: u32 = 0x8;
    pub const BACKBUFFERCOUNT: u32 = 0x20;
    pub const ZBUFFERBITDEPTH: u32 = 0x40;
    pub const LPSURFACE: u32 = 0x800;
    pub const PIXELFORMAT: u32 = 0x1000;
    pub const CKSRCBLT: u32 = 0x10000;
    pub const MIPMAPCOUNT: u32 = 0x20000;
}

#[derive(Debug, Clone)]
pub struct Surface {
    pub w: u32,
    pub h: u32,
    pub pitch: u32,
    pub fmt: PixelFormat,
    pub caps: u32,
    pub caps2: u32,
    /// Guest address of the pixel buffer (`h * pitch` bytes).
    pub bits: u32,
    pub lock_count: u32,
    /// Surfaces attached to this one (back buffer, Z-buffer), as COM objects.
    pub attached: Vec<u32>,
    /// Source colour key (low, high), `DDCKEY_SRCBLT`.
    pub ck_src: Option<(u32, u32)>,
    pub ck_dest: Option<(u32, u32)>,
    pub clipper: u32,
    /// Bumped whenever guest memory changes outside the GPU (unlock, CPU blit, flip).
    pub generation: u64,
    /// Implicitly created as part of a flip chain (released with the primary).
    pub implicit: bool,
}

impl Surface {
    pub fn new(w: u32, h: u32, fmt: PixelFormat, caps: u32, caps2: u32, bits: u32) -> Surface {
        Surface {
            w, h, pitch: Surface::pitch_for(w, fmt), fmt, caps, caps2, bits,
            lock_count: 0, attached: Vec::new(), ck_src: None, ck_dest: None, clipper: 0, generation: 1, implicit: false,
        }
    }

    /// Row pitch: bytes per row rounded up to a DWORD, as DirectDraw guarantees.
    pub fn pitch_for(w: u32, fmt: PixelFormat) -> u32 {
        (w * fmt.bytes_per_pixel() + 3) & !3
    }

    pub fn size_bytes(&self) -> u32 {
        self.pitch * self.h
    }

    /// `DDSURFACEDESC2` (0x7c bytes) describing this surface; `lp_surface` is
    /// included (with `DDSD_LPSURFACE`) for Lock and omitted for GetSurfaceDesc.
    pub fn desc(&self, lp_surface: Option<u32>) -> [u8; 0x7c] {
        let mut d = [0u32; 31];
        d[0] = 0x7c;
        d[1] = ddsd::CAPS | ddsd::HEIGHT | ddsd::WIDTH | ddsd::PITCH | ddsd::PIXELFORMAT;
        d[2] = self.h;
        d[3] = self.w;
        d[4] = self.pitch;
        if let Some(p) = lp_surface {
            d[1] |= ddsd::LPSURFACE;
            d[9] = p;
        }
        if let Some((lo, hi)) = self.ck_src {
            d[1] |= ddsd::CKSRCBLT;
            d[16] = lo;
            d[17] = hi;
        }
        if let Some((lo, hi)) = self.ck_dest {
            d[12] = lo;
            d[13] = hi;
        }
        let pf = self.fmt.to_ddpf();
        for i in 0..8 {
            d[18 + i] = u32::from_le_bytes(pf[4 * i..4 * i + 4].try_into().unwrap());
        }
        d[26] = self.caps;
        d[27] = self.caps2;
        let mut out = [0u8; 0x7c];
        for (i, v) in d.iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
        }
        out
    }
}

/// A `RECT` (left, top, right, bottom), clipped to a surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub l: i32,
    pub t: i32,
    pub r: i32,
    pub b: i32,
}

impl Rect {
    pub fn full(s: &Surface) -> Rect {
        Rect { l: 0, t: 0, r: s.w as i32, b: s.h as i32 }
    }

    pub fn w(&self) -> i32 {
        self.r - self.l
    }

    pub fn h(&self) -> i32 {
        self.b - self.t
    }

    /// Valid and inside `s` (DirectDraw rejects out-of-bounds rectangles without a clipper).
    pub fn inside(&self, s: &Surface) -> bool {
        self.l >= 0 && self.t >= 0 && self.l < self.r && self.t < self.b && self.r <= s.w as i32 && self.b <= s.h as i32
    }
}

/// CPU blits on guest pixel buffers. `dst`/`src` are the surfaces' pixel bytes.
pub mod blit {
    use super::{Rect, Surface};

    pub fn fill(dst: &Surface, mem: &mut [u8], r: Rect, value: u32) {
        let bpp = dst.fmt.bytes_per_pixel() as usize;
        for y in r.t..r.b {
            let row = y as usize * dst.pitch as usize;
            for x in r.l..r.r {
                dst.fmt.store(&mut mem[row + x as usize * bpp..], value);
            }
        }
    }

    /// Copy (nearest-neighbour stretch when sizes differ) from `src` to `dst`,
    /// skipping source pixels within the colour key range `key` if given.
    /// Pixel formats must match (DirectDraw blits do not convert formats).
    #[allow(clippy::too_many_arguments)]
    pub fn copy(dst: &Surface, dmem: &mut [u8], dr: Rect, src: &Surface, smem: &[u8], sr: Rect, key: Option<(u32, u32)>) {
        let bpp = dst.fmt.bytes_per_pixel() as usize;
        for y in 0..dr.h() {
            let sy = sr.t + (y as i64 * sr.h() as i64 / dr.h() as i64) as i32;
            let drow = (dr.t + y) as usize * dst.pitch as usize;
            let srow = sy as usize * src.pitch as usize;
            for x in 0..dr.w() {
                let sx = sr.l + (x as i64 * sr.w() as i64 / dr.w() as i64) as i32;
                let v = src.fmt.load(&smem[srow + sx as usize * bpp..]);
                if let Some((lo, hi)) = key {
                    if v >= lo && v <= hi {
                        continue;
                    }
                }
                dst.fmt.store(&mut dmem[drow + (dr.l + x) as usize * bpp..], v);
            }
        }
    }
}

// ---- COM methods -------------------------------------------------------------------

use super::{arg, fill_sized, guid, iface, qi_result, read_guid, ret, Kind, DDERR_INVALIDPARAMS, DDERR_INVALIDRECT, DDERR_NOCOLORKEY, DDERR_NOTFOUND, DDERR_NOTLOCKED, DD_OK};
use crate::{fatal, with_rt, Cpu, Runtime};

const DDBLT_COLORFILL: u32 = 0x400;
const DDBLT_KEYSRC: u32 = 0x8000;
const DDBLT_KEYSRCOVERRIDE: u32 = 0x10000;
const DDBLT_WAIT: u32 = 0x100_0000;
const DDBLT_DEPTHFILL: u32 = 0x200_0000;
const DDBLTFAST_SRCCOLORKEY: u32 = 0x1;
const DDBLTFAST_WAIT: u32 = 0x10;
const DDCKEY_DESTBLT: u32 = 0x2;
const DDCKEY_SRCBLT: u32 = 0x8;

/// Read a guest `RECT`, or the full surface for NULL.
fn read_rect(rt: &Runtime, p: u32, s: &Surface) -> Rect {
    if p == 0 {
        return Rect::full(s);
    }
    Rect { l: rt.mem.r32(p) as i32, t: rt.mem.r32(p + 4) as i32, r: rt.mem.r32(p + 8) as i32, b: rt.mem.r32(p + 12) as i32 }
}

/// Make guest memory current before the CPU reads or writes `obj` (render
/// targets may hold newer pixels on the GPU).
pub(crate) fn sync_for_cpu(rt: &mut Runtime, obj: u32) {
    crate::render::sync_for_cpu(rt, obj);
}

/// Record that the CPU changed `obj`'s guest pixels.
pub(crate) fn cpu_modified(rt: &mut Runtime, obj: u32) {
    rt.com.surface_mut(obj).generation += 1;
}

/// Free a surface's pixels when its last reference goes; implicit chain members go with the primary.
pub(crate) fn destroy(rt: &mut Runtime, obj: u32) {
    if !rt.com.surfaces.contains_key(&obj) {
        return;
    }
    crate::render::before_surface_destroyed(rt, obj);
    let Some(s) = rt.com.surfaces.remove(&obj) else { return };
    crate::render::surface_destroyed(rt, obj, s.bits);
    rt.heap.free(s.bits);
    if rt.com.primary == Some(obj) {
        rt.com.primary = None;
        for a in s.attached {
            if rt.com.surfaces.get(&a).is_some_and(|x| x.implicit) {
                rt.com.objs.remove(&a);
                destroy(rt, a);
            }
        }
    }
}

/// Swap the pixel storage of the front buffer and its first back buffer, as
/// `IDirectDrawSurface::Flip` does; the surface objects keep their identities.
pub(crate) fn flip(rt: &mut Runtime, front: u32) -> Result<u32, u32> {
    let back = *rt.com.surface(front).attached.first().ok_or(DDERR_NOTFOUND)?;
    crate::render::before_flip(rt, front, back);
    let (fb, fg) = { let f = rt.com.surface(front); (f.bits, f.generation) };
    let (bb, bg) = { let b = rt.com.surface(back); (b.bits, b.generation) };
    { let f = rt.com.surface_mut(front); f.bits = bb; f.generation = bg; }
    { let b = rt.com.surface_mut(back); b.bits = fb; b.generation = fg; }
    rt.com.flips += 1;
    crate::render::after_flip(rt, front);
    Ok(back)
}

/// Returns false if the method is not implemented.
pub(super) fn call(cpu: &mut Cpu, iface_id: u32, this: u32, slot: u32) -> bool {
    if iface_id == iface::CLIPPER {
        match slot {
            // SetHWnd(this, flags, hwnd)
            8 => {
                let hwnd = arg(cpu, 2);
                with_rt(|rt| rt.com.objs.get_mut(&this).unwrap().kind = Kind::Clipper { hwnd });
                ret(cpu, DD_OK, 3);
            }
            // GetHWnd(this, HWND*)
            4 => {
                let out = arg(cpu, 1);
                with_rt(|rt| {
                    let Kind::Clipper { hwnd } = rt.com.get(this).kind else { unreachable!() };
                    rt.mem.w32(out, hwnd);
                });
                ret(cpu, DD_OK, 2);
            }
            _ => return false,
        }
        return true;
    }
    match slot {
        // QueryInterface(this, riid, out)
        0 => {
            let (riid, out) = (arg(cpu, 1), arg(cpu, 2));
            let id = read_guid(riid);
            let obj = if id == guid::IID_IDIRECTDRAWSURFACE4 {
                with_rt(|rt| rt.com.objs.get_mut(&this).unwrap().refs += 1);
                Some(this)
            } else if id == guid::IID_IDIRECT3DTEXTURE2 {
                Some(with_rt(|rt| super::d3d::texture_for_surface(rt, this)))
            } else {
                None
            };
            qi_result(cpu, iface_id, id, out, obj);
        }
        // AddAttachedSurface(this, surface)
        3 => {
            let other = arg(cpu, 1);
            with_rt(|rt| {
                rt.com.surface(other);
                rt.com.surface_mut(this).attached.push(other);
                rt.com.objs.get_mut(&other).unwrap().refs += 1;
            });
            ret(cpu, DD_OK, 2);
        }
        // Blt(this, dstRect, src, srcRect, flags, LPDDBLTFX)
        5 => {
            let a: Vec<u32> = (1..6).map(|i| arg(cpu, i)).collect();
            let r = with_rt(|rt| blt(rt, this, a[0], a[1], a[2], a[3], a[4]));
            ret(cpu, r, 6);
        }
        // BltFast(this, x, y, src, srcRect, trans)
        7 => {
            let (x, y, src, srect, trans) = (arg(cpu, 1), arg(cpu, 2), arg(cpu, 3), arg(cpu, 4), arg(cpu, 5));
            let r = with_rt(|rt| {
                let sr = read_rect(rt, srect, rt.com.surface(src));
                let dr = Rect { l: x as i32, t: y as i32, r: x as i32 + sr.w(), b: y as i32 + sr.h() };
                let key = if trans & DDBLTFAST_SRCCOLORKEY != 0 { rt.com.surface(src).ck_src } else { None };
                if trans & !(DDBLTFAST_SRCCOLORKEY | DDBLTFAST_WAIT) != 0 {
                    fatal(&format!("BltFast: flags {trans:#x} not emulated"));
                }
                blt_rects(rt, this, dr, src, sr, key)
            });
            ret(cpu, r, 6);
        }
        // DeleteAttachedSurface(this, flags, surface)
        8 => {
            let other = arg(cpu, 2);
            let found = with_rt(|rt| {
                let s = rt.com.surface_mut(this);
                let before = s.attached.len();
                s.attached.retain(|&a| a != other);
                before != s.attached.len()
            });
            if found {
                add_ref_release_internal(other);
            }
            ret(cpu, if found { DD_OK } else { DDERR_NOTFOUND }, 3);
        }
        // Flip(this, override, flags)
        11 => {
            let r = with_rt(|rt| {
                let r = flip(rt, this).map(|_| DD_OK).unwrap_or_else(|e| e);
                // DDFLIP_WAIT: the flip happens at the next vertical blank.
                crate::clock::vblank_wait(rt);
                r
            });
            ret(cpu, r, 3);
        }
        // GetAttachedSurface(this, LPDDSCAPS2, LPDIRECTDRAWSURFACE4*)
        12 => {
            let (pcaps, out) = (arg(cpu, 1), arg(cpu, 2));
            let r = with_rt(|rt| {
                let want = rt.mem.r32(pcaps);
                let found = rt.com.surface(this).attached.iter().copied().find(|a| rt.com.surface(*a).caps & want == want);
                match found {
                    Some(a) => {
                        rt.com.objs.get_mut(&a).unwrap().refs += 1;
                        rt.mem.w32(out, a);
                        DD_OK
                    }
                    None => {
                        rt.mem.w32(out, 0);
                        DDERR_NOTFOUND
                    }
                }
            });
            ret(cpu, r, 3);
        }
        // GetCaps(this, LPDDSCAPS2)
        14 => {
            let p = arg(cpu, 1);
            with_rt(|rt| {
                let s = rt.com.surface(this);
                for (i, v) in [s.caps, s.caps2, 0, 0].into_iter().enumerate() {
                    rt.mem.w32(p + 4 * i as u32, v);
                }
            });
            ret(cpu, DD_OK, 2);
        }
        // GetColorKey(this, flags, LPDDCOLORKEY)
        16 => {
            let (flags, p) = (arg(cpu, 1), arg(cpu, 2));
            let r = with_rt(|rt| {
                let s = rt.com.surface(this);
                let k = if flags & DDCKEY_SRCBLT != 0 { s.ck_src } else if flags & DDCKEY_DESTBLT != 0 { s.ck_dest } else { None };
                match k {
                    Some((lo, hi)) => {
                        rt.mem.w32(p, lo);
                        rt.mem.w32(p + 4, hi);
                        DD_OK
                    }
                    None => DDERR_NOCOLORKEY,
                }
            });
            ret(cpu, r, 3);
        }
        // GetPixelFormat(this, LPDDPIXELFORMAT)
        21 => {
            let p = arg(cpu, 1);
            with_rt(|rt| {
                let pf = rt.com.surface(this).fmt.to_ddpf();
                rt.mem.write(p, &pf);
            });
            ret(cpu, DD_OK, 2);
        }
        // GetSurfaceDesc(this, LPDDSURFACEDESC2)
        22 => {
            let p = arg(cpu, 1);
            let d = with_rt(|rt| rt.com.surface(this).desc(None));
            fill_sized(p, &d);
            ret(cpu, DD_OK, 2);
        }
        // IsLost / Restore: the emulated display never loses surfaces.
        24 | 27 => ret(cpu, DD_OK, 1),
        // Lock(this, LPRECT, LPDDSURFACEDESC2, flags, event)
        25 => {
            let (prect, pdesc) = (arg(cpu, 1), arg(cpu, 2));
            let r = with_rt(|rt| {
                sync_for_cpu(rt, this);
                let s = rt.com.surface(this).clone();
                let rect = read_rect(rt, prect, &s);
                if !rect.inside(&s) {
                    return Err(DDERR_INVALIDRECT);
                }
                let p = s.bits + rect.t as u32 * s.pitch + rect.l as u32 * s.fmt.bytes_per_pixel();
                rt.com.surface_mut(this).lock_count += 1;
                Ok(s.desc(Some(p)))
            });
            match r {
                Ok(d) => {
                    fill_sized(pdesc, &d);
                    ret(cpu, DD_OK, 5);
                }
                Err(e) => ret(cpu, e, 5),
            }
        }
        // SetClipper(this, clipper)
        28 => {
            let c = arg(cpu, 1);
            with_rt(|rt| rt.com.surface_mut(this).clipper = c);
            ret(cpu, DD_OK, 2);
        }
        // SetColorKey(this, flags, LPDDCOLORKEY or NULL)
        29 => {
            let (flags, p) = (arg(cpu, 1), arg(cpu, 2));
            let r = with_rt(|rt| {
                let k = (p != 0).then(|| (rt.mem.r32(p), rt.mem.r32(p + 4)));
                let s = rt.com.surface_mut(this);
                if flags & DDCKEY_SRCBLT != 0 {
                    s.ck_src = k;
                } else if flags & DDCKEY_DESTBLT != 0 {
                    s.ck_dest = k;
                } else {
                    return DDERR_INVALIDPARAMS;
                }
                s.generation += 1; // colour-keyed textures depend on the key
                DD_OK
            });
            ret(cpu, r, 3);
        }
        // Unlock(this, LPRECT)
        32 => {
            let r = with_rt(|rt| {
                let s = rt.com.surface_mut(this);
                if s.lock_count == 0 {
                    return DDERR_NOTLOCKED;
                }
                s.lock_count -= 1;
                cpu_modified(rt, this);
                DD_OK
            });
            ret(cpu, r, 2);
        }
        _ => return false,
    }
    true
}

fn add_ref_release_internal(obj: u32) {
    with_rt(|rt| {
        let Some(o) = rt.com.objs.get_mut(&obj) else { return };
        o.refs -= 1;
        if o.refs == 0 {
            let o = rt.com.objs.remove(&obj).unwrap();
            super::destroy_object(rt, obj, o.kind);
        }
    });
}

/// `Blt` with colour fill, depth fill, or a (stretched, optionally keyed) copy.
fn blt(rt: &mut Runtime, dst: u32, pdr: u32, src: u32, psr: u32, flags: u32, fx: u32) -> u32 {
    let known = DDBLT_COLORFILL | DDBLT_DEPTHFILL | DDBLT_KEYSRC | DDBLT_KEYSRCOVERRIDE | DDBLT_WAIT | 0x0100_0000;
    if flags & !known != 0 {
        fatal(&format!("Blt: flags {flags:#x} not emulated"));
    }
    let d = rt.com.surface(dst).clone();
    let dr = read_rect(rt, pdr, &d);
    if flags & (DDBLT_COLORFILL | DDBLT_DEPTHFILL) != 0 {
        if !dr.inside(&d) || fx == 0 {
            return DDERR_INVALIDPARAMS;
        }
        // DDBLTFX.dwFillColor / dwFillDepth (a union at +0x50)
        let value = rt.mem.r32(fx + 0x50);
        sync_for_cpu(rt, dst);
        let mem = rt.mem.slice_mut(d.bits, d.size_bytes() as usize);
        blit::fill(&d, mem, dr, value);
        cpu_modified(rt, dst);
        return DD_OK;
    }
    if src == 0 {
        return DDERR_INVALIDPARAMS;
    }
    let s = rt.com.surface(src).clone();
    let sr = read_rect(rt, psr, &s);
    let key = if flags & DDBLT_KEYSRCOVERRIDE != 0 {
        Some((rt.mem.r32(fx + 0x5c), rt.mem.r32(fx + 0x60)))
    } else if flags & DDBLT_KEYSRC != 0 {
        s.ck_src
    } else {
        None
    };
    blt_rects(rt, dst, dr, src, sr, key)
}

fn blt_rects(rt: &mut Runtime, dst: u32, dr: Rect, src: u32, sr: Rect, key: Option<(u32, u32)>) -> u32 {
    let d = rt.com.surface(dst).clone();
    let s = rt.com.surface(src).clone();
    if !dr.inside(&d) || !sr.inside(&s) {
        return DDERR_INVALIDRECT;
    }
    if d.fmt != s.fmt {
        fatal(&format!("Blt between different pixel formats ({:?} → {:?}) is not emulated", s.fmt, d.fmt));
    }
    sync_for_cpu(rt, src);
    sync_for_cpu(rt, dst);
    // Copy the source first so overlapping blits on one surface behave like DirectDraw's.
    let smem = rt.mem.slice(s.bits, s.size_bytes() as usize).to_vec();
    let dmem = rt.mem.slice_mut(d.bits, d.size_bytes() as usize);
    blit::copy(&d, dmem, dr, &s, &smem, sr, key);
    cpu_modified(rt, dst);
    DD_OK
}
