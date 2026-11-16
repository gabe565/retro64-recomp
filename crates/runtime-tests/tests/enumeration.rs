//! DirectDraw/Direct3D enumeration and capability reporting, as seen by guest callbacks.

use bugatron_runtime::ddraw::caps;
use bugatron_runtime::testing::{call_com, call_import};
use bugatron_runtime_tests::*;

fn cstr(a: u32) -> String {
    let mut v = Vec::new();
    let mut p = a;
    loop {
        let b = read(p, 1)[0];
        if b == 0 { break; }
        v.push(b);
        p += 1;
    }
    String::from_utf8(v).unwrap()
}

fn put_guid(off: u32, g: &[u8; 16]) -> u32 {
    bugatron_runtime::with_rt(|rt| rt.mem.write(SCRATCH + off, g));
    SCRATCH + off
}

#[test]
fn driver_enumeration_reports_the_primary_display() {
    let mut cpu = setup();
    assert_eq!(call_import(&mut cpu, "ddraw.dll", "DirectDrawEnumerateA", &[HELPER_REC4_CONTINUE, 0x1234]), 0);
    let l = log();
    assert_eq!(l.len(), 4, "exactly one driver");
    assert_eq!(l[0], 0, "primary driver has a NULL GUID");
    assert_eq!(cstr(l[1]), "Primary Display Driver");
    assert_eq!(cstr(l[2]), "display");
    assert_eq!(l[3], 0x1234);
}

#[test]
fn query_interface_refcounts_and_unsupported_interfaces() {
    let mut cpu = setup();
    let dd4 = dd4(&mut cpu);
    assert_eq!(call_com(&mut cpu, dd4, 1, &[]), 2); // AddRef
    assert_eq!(call_com(&mut cpu, dd4, 2, &[]), 1); // Release
    let bogus = put_guid(0x900, &[0x11; 16]);
    w32(SCRATCH + 0x800, 0xdead);
    assert_eq!(call_com(&mut cpu, dd4, 0, &[bogus, SCRATCH + 0x800]), 0x8000_4002); // E_NOINTERFACE
    assert_eq!(r32(SCRATCH + 0x800), 0, "out pointer cleared on failure");
}

#[test]
fn get_caps_honours_the_callers_structure_size() {
    let mut cpu = setup();
    let dd4 = dd4(&mut cpu);
    let (hal, hel) = (SCRATCH + 0x1000, SCRATCH + 0x2000);
    w32(hal, 0x17c);
    w32(hel, 0x6c); // an older, smaller DDCAPS
    w32(hel + 0x6c, 0xcafe_f00d);
    assert_eq!(call_com(&mut cpu, dd4, 11, &[hal, hel]), 0);
    assert_eq!(read(hal, 0x17c), caps::HAL_DDCAPS.to_vec());
    assert_eq!(read(hel + 4, 0x68), caps::HEL_DDCAPS[4..0x6c].to_vec());
    assert_eq!(r32(hel), 0x6c);
    assert_eq!(r32(hel + 0x6c), 0xcafe_f00d, "nothing written past the caller's dwSize");
    // The game's windowed-rendering check reads DDCAPS2_CANRENDERWINDOWED.
    assert_ne!(r32(hal + 8) & 0x80000, 0);
}

#[test]
fn display_modes_are_32_bit_first_with_correct_pixel_formats() {
    let mut cpu = setup();
    let dd4 = dd4(&mut cpu);
    assert_eq!(call_com(&mut cpu, dd4, 8, &[0, 0, 0, HELPER_MODE_COPIER]), 0);
    let b = log_bytes();
    assert_eq!(b.len() % 0x7c, 0);
    let modes: Vec<(u32, u32, u32)> = b.chunks(0x7c).map(|m| (dw(m, 3), dw(m, 2), dw(m, 21))).collect();
    assert_eq!(modes[0], (640, 480, 32), "the game's default rule picks the first 640-wide mode");
    assert!(modes.contains(&(800, 600, 16)), "the README's 800x600x16 mode stays available");
    // The original's fullscreen projection assumes 4:3 (0x421fd0): offer 4:3 modes up to
    // 2048×1536, plus the period-common 5:4 1280×1024, at 16 and 32 bits.
    for &(w, h, bpp) in &modes {
        assert!(w * 3 == h * 4 || (w, h) == (1280, 1024), "{w}x{h}x{bpp}");
        assert!(modes.contains(&(w, h, if bpp == 32 { 16 } else { 32 })), "both depths for {w}x{h}");
    }
    assert_eq!(modes.iter().map(|m| m.0 * m.1).max(), Some(2048 * 1536));
    for m in b.chunks(0x7c) {
        assert_eq!(dw(m, 0), 0x7c);
        assert_eq!(dw(m, 1) & 0x100f, 0x100f, "CAPS|HEIGHT|WIDTH|PITCH|PIXELFORMAT");
        assert_eq!(dw(m, 4), dw(m, 3) * dw(m, 21) / 8, "pitch");
        assert_eq!(dw(m, 18), 0x20, "DDPIXELFORMAT.dwSize");
        assert_eq!(dw(m, 19), 0x40, "DDPF_RGB");
        let masks = (dw(m, 22), dw(m, 23), dw(m, 24));
        match dw(m, 21) {
            16 => assert_eq!(masks, (0xf800, 0x7e0, 0x1f)),
            32 => assert_eq!(masks, (0xff_0000, 0xff00, 0xff)),
            d => panic!("unexpected depth {d}"),
        }
    }
    // A callback returning DDENUMRET_CANCEL stops enumeration.
    clear_log();
    call_com(&mut cpu, dd4, 8, &[0, 0, 0x77, HELPER_REC2_CANCEL]);
    assert_eq!(log().len(), 2);
    assert_eq!(log()[1], 0x77);
}

#[test]
fn desktop_mode_is_32_bit() {
    let mut cpu = setup();
    let dd4 = dd4(&mut cpu);
    let sd = SCRATCH + 0x1000;
    w32(sd, 0x7c);
    assert_eq!(call_com(&mut cpu, dd4, 12, &[sd]), 0);
    let b = read(sd, 0x7c);
    assert_eq!(dw(&b, 21), 32);
}

#[test]
fn direct3d_devices_match_the_reference_driver() {
    let mut cpu = setup();
    let dd4 = dd4(&mut cpu);
    let iid = put_guid(0x900, &IID_IDIRECT3D3);
    let out = SCRATCH + 0x800;
    assert_eq!(call_com(&mut cpu, dd4, 0, &[iid, out]), 0);
    let d3d = r32(out);
    assert_eq!(call_com(&mut cpu, d3d, 3, &[HELPER_REC6_CONTINUE, 0x55]), 0);
    let l = log();
    assert_eq!(l.len(), 12, "two devices");
    let names: Vec<String> = l.chunks(6).map(|d| cstr(d[2])).collect();
    assert_eq!(names, ["RGB Emulation", "Direct3D HAL"], "reference order: index 1 is the HAL");
    for d in l.chunks(6) {
        assert_eq!(d[5], 0x55);
        assert_eq!(r32(d[3]), 0xfc, "HAL desc dwSize");
        assert_eq!(r32(d[4]), 0xfc, "HEL desc dwSize");
    }
    // The game treats a device as hardware when the HAL description's dwFlags is non-zero.
    let (rgb, hal) = (&l[0..6], &l[6..12]);
    assert_eq!(r32(rgb[3] + 4), 0);
    assert_ne!(r32(hal[3] + 4), 0);
    assert_eq!(read(hal[3] + 4, 0xf8), caps::HAL_DEVICE_DESC[4..].to_vec());
    // dwDeviceRenderBitDepth offers 16 and 32 bit (DDBD_16 | DDBD_32).
    assert_eq!(r32(hal[3] + 0x9c) & 0x500, 0x500);
}
