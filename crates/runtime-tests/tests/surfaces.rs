//! DirectDraw surfaces in guest memory: creation, descriptors, locking, flipping and blits.

use bugatron_runtime::ddraw::pixel::PixelFormat;
use bugatron_runtime::testing::call_com;
use bugatron_runtime::{with_rt, Cpu};
use bugatron_runtime_tests::*;

// IDirectDraw4 slots
const SET_COOPERATIVE_LEVEL: u32 = 20;
const SET_DISPLAY_MODE: u32 = 21;
// IDirectDrawSurface4 slots
const QUERY_INTERFACE: u32 = 0;
const ADD_REF: u32 = 1;
const RELEASE: u32 = 2;
const ADD_ATTACHED: u32 = 3;
const BLT: u32 = 5;
const BLT_FAST: u32 = 7;
const FLIP: u32 = 11;
const GET_ATTACHED: u32 = 12;
const GET_CAPS: u32 = 14;
const GET_COLOR_KEY: u32 = 16;
const GET_PIXEL_FORMAT: u32 = 21;
const GET_SURFACE_DESC: u32 = 22;
const LOCK: u32 = 25;
const SET_COLOR_KEY: u32 = 29;
const UNLOCK: u32 = 32;

const DDERR_INVALIDRECT: u32 = 0x8876_0096;
const DDERR_NOTLOCKED: u32 = 0x8876_0248;
const DDERR_INVALIDMODE: u32 = 0x8876_0078;
const DDERR_NOCOLORKEY: u32 = 0x8876_00d7;
const DDERR_NOTFOUND: u32 = 0x8876_00ff;

/// Fullscreen setup as r3d does it: exclusive mode and SetDisplayMode(w, h, bpp).
fn fullscreen(cpu: &mut Cpu, w: u32, h: u32, bpp: u32) -> u32 {
    let dd4 = dd4(cpu);
    assert_eq!(call_com(cpu, dd4, SET_COOPERATIVE_LEVEL, &[0x0001_0010, 0x13]), 0);
    assert_eq!(call_com(cpu, dd4, SET_DISPLAY_MODE, &[w, h, bpp, 0, 0]), 0);
    dd4
}

/// The flip chain r3d creates: DDSD_CAPS|DDSD_BACKBUFFERCOUNT, PRIMARY|FLIP|COMPLEX|3DDEVICE, 1 back buffer.
fn flip_chain(cpu: &mut Cpu, dd4: u32) -> (u32, u32) {
    let d = surface_desc(0x1000);
    w32(d + 4, 0x21);
    w32(d + 0x14, 1);
    w32(d + 0x68, 0x2218);
    let (r, front) = create_surface(cpu, dd4, d);
    assert_eq!(r, 0);
    let caps = put_bytes(0x1100, &4u32.to_le_bytes()); // DDSCAPS_BACKBUFFER
    assert_eq!(call_com(cpu, front, GET_ATTACHED, &[caps, SCRATCH + 0x1200]), 0);
    (front, r32(SCRATCH + 0x1200))
}

/// An offscreen surface of the given size and format.
fn offscreen(cpu: &mut Cpu, dd4: u32, w: u32, h: u32, fmt: PixelFormat) -> u32 {
    let d = surface_desc(0x2000);
    w32(d + 4, 0x1007); // CAPS|HEIGHT|WIDTH|PIXELFORMAT
    w32(d + 8, h);
    w32(d + 0xc, w);
    with_rt(|rt| rt.mem.write(d + 0x48, &fmt.to_ddpf()));
    w32(d + 0x68, 0x40 | 0x800); // OFFSCREENPLAIN | SYSTEMMEMORY
    let (r, s) = create_surface(cpu, dd4, d);
    assert_eq!(r, 0);
    s
}

/// Lock (whole surface or `rect`) and return (lpSurface, pitch).
fn lock(cpu: &mut Cpu, s: u32, rect: u32) -> (u32, u32) {
    let d = surface_desc(0x3000);
    assert_eq!(call_com(cpu, s, LOCK, &[rect, d, 0x801, 0]), 0);
    (r32(d + 0x24), r32(d + 0x10))
}

fn rect(off: u32, l: u32, t: u32, r: u32, b: u32) -> u32 {
    for (i, v) in [l, t, r, b].into_iter().enumerate() {
        w32(SCRATCH + off + 4 * i as u32, v);
    }
    SCRATCH + off
}

#[test]
fn flip_chain_has_the_documented_caps_and_attachments() {
    let mut cpu = setup();
    let dd4 = fullscreen(&mut cpu, 640, 480, 32);
    let (front, back) = flip_chain(&mut cpu, dd4);
    let caps = SCRATCH + 0x1300;
    call_com(&mut cpu, front, GET_CAPS, &[caps]);
    let fc = r32(caps);
    for bit in [0x200, 0x20, 0x8000, 0x10, 0x8, 0x2000, 0x4000] {
        assert_ne!(fc & bit, 0, "front buffer cap {bit:#x}");
    }
    call_com(&mut cpu, back, GET_CAPS, &[caps]);
    let bc = r32(caps);
    assert_ne!(bc & 0x4, 0, "BACKBUFFER");
    assert_eq!(bc & (0x200 | 0x8000), 0, "back buffer is neither primary nor visible");
    // GetAttachedSurface AddRef'd the back buffer.
    assert_eq!(call_com(&mut cpu, back, ADD_REF, &[]), 3);
    call_com(&mut cpu, back, RELEASE, &[]);
    // The chain is circular: the back buffer's attachment is the front buffer (FRONTBUFFER cap).
    let want = put_bytes(0x1400, &0x20u32.to_le_bytes());
    assert_eq!(call_com(&mut cpu, back, GET_ATTACHED, &[want, SCRATCH + 0x1500]), 0);
    assert_eq!(r32(SCRATCH + 0x1500), front);
    let want = put_bytes(0x1400, &0x20000u32.to_le_bytes());
    assert_eq!(call_com(&mut cpu, front, GET_ATTACHED, &[want, SCRATCH + 0x1500]), DDERR_NOTFOUND);
}

#[test]
fn surface_descriptors_report_mode_format_and_pitch() {
    for (bpp, fmt, pitch) in [(32, PixelFormat::XRGB8888, 2560), (16, PixelFormat::RGB565, 1280)] {
        let mut cpu = setup();
        let dd4 = fullscreen(&mut cpu, 640, 480, bpp);
        let (_, back) = flip_chain(&mut cpu, dd4);
        let d = surface_desc(0x3000);
        assert_eq!(call_com(&mut cpu, back, GET_SURFACE_DESC, &[d]), 0);
        let b = read(d, 0x7c);
        assert_eq!((dw(&b, 3), dw(&b, 2), dw(&b, 4)), (640, 480, pitch));
        assert_eq!(dw(&b, 1) & 0x800, 0, "GetSurfaceDesc has no lpSurface");
        assert_eq!(PixelFormat::from_ddpf(&b[0x48..0x68]), fmt);
        let pf = SCRATCH + 0x3100;
        call_com(&mut cpu, back, GET_PIXEL_FORMAT, &[pf]);
        assert_eq!(PixelFormat::from_ddpf(&read(pf, 32)), fmt);
    }
}

#[test]
fn unsupported_display_mode_is_rejected() {
    let mut cpu = setup();
    let dd4 = dd4(&mut cpu);
    assert_eq!(call_com(&mut cpu, dd4, SET_DISPLAY_MODE, &[641, 480, 32, 0, 0]), DDERR_INVALIDMODE);
    assert_eq!(call_com(&mut cpu, dd4, SET_DISPLAY_MODE, &[800, 600, 16, 0, 0]), 0);
    assert_eq!(with_rt(|rt| rt.user.screen), (800, 600), "the guest screen follows the display mode");
}

#[test]
fn lock_exposes_guest_memory_with_rectangles_and_errors() {
    let mut cpu = setup();
    let dd4 = fullscreen(&mut cpu, 640, 480, 32);
    let (_, back) = flip_chain(&mut cpu, dd4);
    let (p, pitch) = lock(&mut cpu, back, 0);
    assert_eq!(pitch, 2560);
    w32(p + 10 * pitch + 20 * 4, 0x00ab_cdef);
    let gen0 = with_rt(|rt| rt.com.surface(back).generation);
    assert_eq!(call_com(&mut cpu, back, UNLOCK, &[0]), 0);
    assert!(with_rt(|rt| rt.com.surface(back).generation) > gen0, "unlock marks CPU changes");
    // A rectangle lock points at the rectangle's first pixel.
    let (q, _) = lock(&mut cpu, back, rect(0x4000, 20, 10, 30, 20));
    assert_eq!(q, p + 10 * pitch + 20 * 4);
    assert_eq!(r32(q), 0x00ab_cdef);
    call_com(&mut cpu, back, UNLOCK, &[0]);
    assert_eq!(call_com(&mut cpu, back, UNLOCK, &[0]), DDERR_NOTLOCKED);
    let d = surface_desc(0x3000);
    assert_eq!(call_com(&mut cpu, back, LOCK, &[rect(0x4000, 600, 0, 700, 10), d, 1, 0]), DDERR_INVALIDRECT);
    // A smaller caller dwSize (DDSURFACEDESC, 0x6c) is honoured.
    w32(d, 0x6c);
    w32(d + 0x6c, 0x1357_9bdf);
    assert_eq!(call_com(&mut cpu, back, LOCK, &[0, d, 1, 0]), 0);
    assert_eq!(r32(d + 0x24), p);
    assert_eq!(r32(d + 0x6c), 0x1357_9bdf);
    call_com(&mut cpu, back, UNLOCK, &[0]);
}

#[test]
fn flip_exchanges_front_and_back_pixels() {
    let mut cpu = setup();
    let dd4 = fullscreen(&mut cpu, 640, 480, 16);
    let (front, back) = flip_chain(&mut cpu, dd4);
    let (bp, _) = lock(&mut cpu, back, 0);
    w32(bp, 0xbeef_f00d);
    call_com(&mut cpu, back, UNLOCK, &[0]);
    let (fp, _) = lock(&mut cpu, front, 0);
    w32(fp, 0x1111_2222);
    call_com(&mut cpu, front, UNLOCK, &[0]);
    assert_eq!(call_com(&mut cpu, front, FLIP, &[0, 1]), 0);
    let (fp2, _) = lock(&mut cpu, front, 0);
    assert_eq!(r32(fp2), 0xbeef_f00d, "what was drawn to the back buffer is now displayed");
    call_com(&mut cpu, front, UNLOCK, &[0]);
    let (bp2, _) = lock(&mut cpu, back, 0);
    assert_eq!(r32(bp2), 0x1111_2222, "the back buffer now holds the old front buffer");
    call_com(&mut cpu, back, UNLOCK, &[0]);
    assert_eq!(with_rt(|rt| rt.com.flips), 1);
}

#[test]
fn zbuffer_and_texture_surfaces() {
    let mut cpu = setup();
    let dd4 = fullscreen(&mut cpu, 640, 480, 32);
    let (_, back) = flip_chain(&mut cpu, dd4);
    // Z-buffer as r3d creates it: CAPS|WIDTH|HEIGHT|PIXELFORMAT, ZBUFFER|VIDEOMEMORY.
    let d = surface_desc(0x1000);
    w32(d + 4, 0x1007);
    w32(d + 8, 480);
    w32(d + 0xc, 640);
    with_rt(|rt| rt.mem.write(d + 0x48, &PixelFormat::Z16.to_ddpf()));
    w32(d + 0x68, 0x24000);
    let (r, z) = create_surface(&mut cpu, dd4, d);
    assert_eq!(r, 0);
    assert_eq!(call_com(&mut cpu, back, ADD_ATTACHED, &[z]), 0);
    let want = put_bytes(0x1400, &0x20000u32.to_le_bytes());
    assert_eq!(call_com(&mut cpu, back, GET_ATTACHED, &[want, SCRATCH + 0x1500]), 0);
    assert_eq!(r32(SCRATCH + 0x1500), z);
    let zd = surface_desc(0x3000);
    call_com(&mut cpu, z, GET_SURFACE_DESC, &[zd]);
    assert_eq!(r32(zd + 0x10), 1280);
    // A 4444 texture: 2 bytes per pixel, pitch rounded to a DWORD.
    let t = offscreen(&mut cpu, dd4, 33, 8, PixelFormat::ARGB4444);
    let (_, pitch) = lock(&mut cpu, t, 0);
    assert_eq!(pitch, 68);
    call_com(&mut cpu, t, UNLOCK, &[0]);
    // QueryInterface for IDirectDrawSurface4 returns the same object, AddRef'd.
    let iid = put_bytes(0x1600, &[0x30, 0x86, 0x2b, 0x0b, 0x35, 0xad, 0xd0, 0x11, 0x8e, 0xa6, 0x00, 0x60, 0x97, 0x97, 0xea, 0x5b]);
    assert_eq!(call_com(&mut cpu, t, QUERY_INTERFACE, &[iid, SCRATCH + 0x1700]), 0);
    assert_eq!(r32(SCRATCH + 0x1700), t);
    assert_eq!(call_com(&mut cpu, t, RELEASE, &[]), 1);
}

#[test]
fn blits_fill_copy_stretch_and_colour_key() {
    let mut cpu = setup();
    let dd4 = fullscreen(&mut cpu, 640, 480, 16);
    let src = offscreen(&mut cpu, dd4, 4, 2, PixelFormat::RGB565);
    let dst = offscreen(&mut cpu, dd4, 8, 4, PixelFormat::RGB565);
    // Colour fill the destination (DDBLT_COLORFILL | DDBLT_WAIT, DDBLTFX.dwFillColor at +0x50).
    let fx = SCRATCH + 0x5000;
    with_rt(|rt| rt.mem.slice_mut(fx, 0x64).fill(0));
    w32(fx, 0x64);
    w32(fx + 0x50, 0x1234);
    assert_eq!(call_com(&mut cpu, dst, BLT, &[0, 0, 0, 0x0100_0400, fx]), 0);
    // Source row 0: 1 2 3 4, row 1: 5 6 7 8 (16-bit values).
    let (sp, spitch) = lock(&mut cpu, src, 0);
    with_rt(|rt| {
        for (i, v) in [1u16, 2, 3, 4].iter().enumerate() { rt.mem.w16(sp + 2 * i as u32, *v); }
        for (i, v) in [5u16, 6, 7, 8].iter().enumerate() { rt.mem.w16(sp + spitch + 2 * i as u32, *v); }
    });
    call_com(&mut cpu, src, UNLOCK, &[0]);
    // 2× stretch of the whole source into the whole destination.
    assert_eq!(call_com(&mut cpu, dst, BLT, &[0, src, 0, 0x0100_0000, 0]), 0);
    let row = |cpu: &mut Cpu, y: u32| -> Vec<u16> {
        let (p, pitch) = lock(cpu, dst, 0);
        let v = (0..8).map(|x| with_rt(|rt| rt.mem.r16(p + y * pitch + 2 * x))).collect();
        call_com(cpu, dst, UNLOCK, &[0]);
        v
    };
    assert_eq!(row(&mut cpu, 0), [1, 1, 2, 2, 3, 3, 4, 4]);
    assert_eq!(row(&mut cpu, 3), [5, 5, 6, 6, 7, 7, 8, 8]);
    // Source colour key 2..3 is skipped by BltFast(SRCCOLORKEY).
    let ck = put_bytes(0x5100, &[2, 0, 0, 0, 3, 0, 0, 0]);
    assert_eq!(call_com(&mut cpu, src, SET_COLOR_KEY, &[8, ck]), 0);
    let got = SCRATCH + 0x5200;
    assert_eq!(call_com(&mut cpu, src, GET_COLOR_KEY, &[8, got]), 0);
    assert_eq!((r32(got), r32(got + 4)), (2, 3));
    assert_eq!(call_com(&mut cpu, src, GET_COLOR_KEY, &[2, got]), DDERR_NOCOLORKEY);
    call_com(&mut cpu, dst, BLT, &[0, 0, 0, 0x0100_0400, fx]);
    assert_eq!(call_com(&mut cpu, dst, BLT_FAST, &[1, 0, src, rect(0x5300, 0, 0, 4, 1), 0x11]), 0);
    assert_eq!(row(&mut cpu, 0), [0x1234, 1, 0x1234, 0x1234, 4, 0x1234, 0x1234, 0x1234]);
    // Out-of-bounds destination rectangles are rejected.
    assert_eq!(call_com(&mut cpu, dst, BLT_FAST, &[6, 0, src, 0, 0x10]), DDERR_INVALIDRECT);
}

#[test]
fn overlapping_blit_within_one_surface() {
    let mut cpu = setup();
    let dd4 = fullscreen(&mut cpu, 640, 480, 16);
    let s = offscreen(&mut cpu, dd4, 4, 1, PixelFormat::RGB565);
    let (p, _) = lock(&mut cpu, s, 0);
    with_rt(|rt| for (i, v) in [1u16, 2, 3, 4].iter().enumerate() { rt.mem.w16(p + 2 * i as u32, *v) });
    call_com(&mut cpu, s, UNLOCK, &[0]);
    // Shift right by one: [1,2,3] → x=1..4.
    assert_eq!(call_com(&mut cpu, s, BLT, &[rect(0x6000, 1, 0, 4, 1), s, rect(0x6010, 0, 0, 3, 1), 0x0100_0000, 0]), 0);
    let (p, _) = lock(&mut cpu, s, 0);
    let v: Vec<u16> = (0..4).map(|i| with_rt(|rt| rt.mem.r16(p + 2 * i))).collect();
    assert_eq!(v, [1, 1, 2, 3]);
}
