//! The SDL3 GPU backend end to end: recorded Direct3D commands are rendered
//! offscreen and read back through IDirectDrawSurface4::Lock, exactly as game
//! code observes them. Without a GPU device the tests are reported as ignored,
//! not passed; set BUGATRON_REQUIRE_GPU=1 to make that a failure (e.g. on Linux
//! under Xvfb with Mesa's lavapipe, which exercises the Vulkan/SPIR-V path).
//!
//! Runs on the main thread (harness = false): SDL's video subsystem requires it on macOS.

use bugatron_runtime::testing::call_com;
use bugatron_runtime::{with_rt, Cpu};
use bugatron_runtime_tests::*;
use std::sync::atomic::{AtomicBool, Ordering};

/// Set when a test found no GPU device and returned early.
static SKIPPED: AtomicBool = AtomicBool::new(false);

const IID_HAL: [u8; 16] = [0xe0, 0x3d, 0xe6, 0x84, 0xaa, 0x46, 0xcf, 0x11, 0x81, 0x6f, 0x00, 0x00, 0xc0, 0x20, 0x15, 0x6e];

struct Scene {
    dev: u32,
    vp: u32,
    back: u32,
}

/// 640×480 at `bpp` with r3d's device setup; None when no GPU is available.
fn scene(cpu: &mut Cpu, bpp: u32) -> Option<Scene> {
    with_rt(|rt| rt.offscreen_gpu = true);
    if !with_rt(bugatron_runtime::render::gpu_available) {
        let required = std::env::var("BUGATRON_REQUIRE_GPU").is_ok_and(|v| !v.is_empty() && v != "0");
        assert!(!required, "no SDL3 GPU device, but BUGATRON_REQUIRE_GPU is set");
        SKIPPED.store(true, Ordering::Relaxed);
        return None;
    }
    let (dd4, _front, back, _z) = display(cpu, bpp);
    let d3d = d3d3(cpu, dd4);
    let clsid = put_bytes(0x7000, &IID_HAL);
    let out = SCRATCH + 0x7100;
    assert_eq!(call_com(cpu, d3d, 8, &[clsid, back, out, 0]), 0);
    let dev = r32(out);
    call_com(cpu, d3d, 6, &[out, 0]);
    let vp = r32(out);
    call_com(cpu, dev, 5, &[vp]);
    let v2 = SCRATCH + 0x7200;
    for (i, v) in [0x2c, 0, 0, 640, 480].into_iter().enumerate() {
        w32(v2 + 4 * i as u32, v);
    }
    put_f32s(v2 + 20, &[-1.0, 1.0, 2.0, 2.0, 0.0, 1.0]);
    call_com(cpu, vp, 17, &[v2]);
    call_com(cpu, dev, 12, &[vp]);
    Some(Scene { dev, vp, back })
}

fn clear(cpu: &mut Cpu, s: &Scene, color: u32) {
    let r = SCRATCH + 0x100;
    for (i, v) in [0u32, 0, 640, 480].into_iter().enumerate() {
        w32(r + 4 * i as u32, v);
    }
    assert_eq!(call_com(cpu, s.vp, 20, &[1, r, 3, color, 1.0f32.to_bits(), 0]), 0);
}

/// Lock the back buffer and read pixel (x, y) as a raw guest value.
fn pixel(cpu: &mut Cpu, s: &Scene, x: u32, y: u32) -> u32 {
    let d = surface_desc(0x3000);
    assert_eq!(call_com(cpu, s.back, 25, &[0, d, 0x801, 0]), 0);
    let (p, pitch, bpp) = (r32(d + 0x24), r32(d + 0x10), r32(d + 0x54));
    let v = with_rt(|rt| if bpp == 16 { rt.mem.r16(p + y * pitch + 2 * x) as u32 } else { rt.mem.r32(p + y * pitch + 4 * x) });
    call_com(cpu, s.back, 32, &[0]);
    v
}

/// A pre-lit (D3DLVERTEX) triangle with screen-space positions via identity transforms.
fn triangle(cpu: &mut Cpu, s: &Scene, pts: [(f32, f32); 3], z: f32, color: u32) {
    let v = SCRATCH + 0x800;
    for (i, (x, y)) in pts.iter().enumerate() {
        let a = v + 32 * i as u32;
        // Screen (D3D6 pixel centres at integers) → NDC.
        put_f32s(a, &[x / 320.0 - 1.0, 1.0 - y / 240.0, z, 0.0]);
        w32(a + 16, color);
        w32(a + 20, 0);
        put_f32s(a + 24, &[0.0, 0.0]);
    }
    assert_eq!(call_com(cpu, s.dev, 22, &[22, 1]), 0, "CULLMODE NONE");
    assert_eq!(call_com(cpu, s.dev, 28, &[4, 0x1e2, v, 3, 0x10]), 0);
}

fn clear_and_lock_read_back_exact_xrgb8888() {
    let mut cpu = setup();
    let Some(s) = scene(&mut cpu, 32) else { return };
    clear(&mut cpu, &s, 0xff12_3456);
    call_com(&mut cpu, s.dev, 9, &[]);
    call_com(&mut cpu, s.dev, 10, &[]);
    assert_eq!(pixel(&mut cpu, &s, 0, 0) & 0xff_ffff, 0x12_3456);
    assert_eq!(pixel(&mut cpu, &s, 639, 479) & 0xff_ffff, 0x12_3456);
}

fn prelit_triangle_covers_pixels_by_direct3d6_centres() {
    let mut cpu = setup();
    let Some(s) = scene(&mut cpu, 32) else { return };
    clear(&mut cpu, &s, 0xff00_0000);
    call_com(&mut cpu, s.dev, 9, &[]);
    // Covers the D3D6 pixel centres (10..=19, 10..=19) region's upper-left half.
    triangle(&mut cpu, &s, [(9.5, 9.5), (30.5, 9.5), (9.5, 30.5)], 0.5, 0xff80_c040);
    call_com(&mut cpu, s.dev, 10, &[]);
    assert_eq!(pixel(&mut cpu, &s, 10, 10) & 0xff_ffff, 0x80_c040, "exact vertex colour");
    assert_eq!(pixel(&mut cpu, &s, 9, 10) & 0xff_ffff, 0, "left of the edge");
    assert_eq!(pixel(&mut cpu, &s, 10, 9) & 0xff_ffff, 0, "above the edge");
    assert_eq!(pixel(&mut cpu, &s, 25, 25) & 0xff_ffff, 0, "beyond the hypotenuse");
}

fn cpu_writes_survive_later_gpu_drawing() {
    let mut cpu = setup();
    let Some(s) = scene(&mut cpu, 32) else { return };
    clear(&mut cpu, &s, 0xff00_0000);
    call_com(&mut cpu, s.dev, 9, &[]);
    call_com(&mut cpu, s.dev, 10, &[]);
    // CPU writes a pixel (as r3d_2DBlit does), then the GPU draws elsewhere.
    let d = surface_desc(0x3000);
    call_com(&mut cpu, s.back, 25, &[0, d, 0x801, 0]);
    let (p, pitch) = (r32(d + 0x24), r32(d + 0x10));
    w32(p + 100 * pitch + 4 * 200, 0x00ab_cdef);
    call_com(&mut cpu, s.back, 32, &[0]);
    call_com(&mut cpu, s.dev, 9, &[]);
    triangle(&mut cpu, &s, [(0.0, 0.0), (8.0, 0.0), (0.0, 8.0)], 0.5, 0xffff_ffff);
    call_com(&mut cpu, s.dev, 10, &[]);
    assert_eq!(pixel(&mut cpu, &s, 200, 100) & 0xff_ffff, 0xab_cdef, "uploaded before drawing");
    assert_eq!(pixel(&mut cpu, &s, 1, 1) & 0xff_ffff, 0xff_ffff);
}

fn depth_test_lessequal_keeps_the_nearer_triangle() {
    let mut cpu = setup();
    let Some(s) = scene(&mut cpu, 32) else { return };
    clear(&mut cpu, &s, 0xff00_0000);
    call_com(&mut cpu, s.dev, 9, &[]);
    triangle(&mut cpu, &s, [(0.0, 0.0), (40.0, 0.0), (0.0, 40.0)], 0.25, 0xffff_0000);
    triangle(&mut cpu, &s, [(0.0, 0.0), (40.0, 0.0), (0.0, 40.0)], 0.75, 0xff00_ff00);
    call_com(&mut cpu, s.dev, 10, &[]);
    assert_eq!(pixel(&mut cpu, &s, 5, 5) & 0xff_ffff, 0xff_0000);
}

fn alpha_blending_srcalpha_invsrcalpha() {
    let mut cpu = setup();
    let Some(s) = scene(&mut cpu, 32) else { return };
    clear(&mut cpu, &s, 0xff00_00ff);
    call_com(&mut cpu, s.dev, 9, &[]);
    call_com(&mut cpu, s.dev, 22, &[27, 1]); // ALPHABLENDENABLE
    call_com(&mut cpu, s.dev, 22, &[19, 5]); // SRCALPHA
    call_com(&mut cpu, s.dev, 22, &[20, 6]); // INVSRCALPHA
    triangle(&mut cpu, &s, [(0.0, 0.0), (40.0, 0.0), (0.0, 40.0)], 0.5, 0x80ff_0000);
    call_com(&mut cpu, s.dev, 10, &[]);
    let p = pixel(&mut cpu, &s, 5, 5);
    let (r, b) = ((p >> 16) & 0xff, p & 0xff);
    // 0xff·(128/255) and 0xff·(127/255): GPUs may round differently by one step.
    assert!((r as i32 - 128).abs() <= 1 && (b as i32 - 127).abs() <= 1, "{p:#x}");
}

fn sixteen_bit_target_truncates_to_rgb565() {
    let mut cpu = setup();
    let Some(s) = scene(&mut cpu, 16) else { return };
    clear(&mut cpu, &s, 0xff87_432f);
    call_com(&mut cpu, s.dev, 9, &[]);
    call_com(&mut cpu, s.dev, 10, &[]);
    assert_eq!(pixel(&mut cpu, &s, 3, 3), ((0x87 & 0xf8) << 8) | ((0x43 & 0xfc) << 3) | (0x2f >> 3));
}

/// A 2×2 RGB565 texture drawn with point sampling and MODULATE by white:
/// texels expand by bit replication, exactly as hardware does.
fn textured_quad_modulate_and_colour_key() {
    let mut cpu = setup();
    let Some(s) = scene(&mut cpu, 32) else { return };
    let dd4 = with_rt(|rt| rt.com.objs.iter().find(|(_, o)| o.iface == 1).map(|(&k, _)| k).unwrap());
    let d = surface_desc(0x2000);
    w32(d + 4, 0x1007);
    w32(d + 8, 2);
    w32(d + 0xc, 2);
    with_rt(|rt| rt.mem.write(d + 0x48, &bugatron_runtime::ddraw::pixel::PixelFormat::RGB565.to_ddpf()));
    w32(d + 0x68, 0x1000);
    let (_, surf) = create_surface(&mut cpu, dd4, d);
    let ld = surface_desc(0x3000);
    call_com(&mut cpu, surf, 25, &[0, ld, 1, 0]);
    let (p, pitch) = (r32(ld + 0x24), r32(ld + 0x10));
    with_rt(|rt| {
        rt.mem.w16(p, 0xf800); // red
        rt.mem.w16(p + 2, 0x07e0); // green
        rt.mem.w16(p + pitch, 0x001f); // blue
        rt.mem.w16(p + pitch + 2, 0x8410); // grey 10000 100000 10000 → 0x84, 0x82, 0x84
    });
    call_com(&mut cpu, surf, 32, &[0]);
    let iid = put_bytes(0x2100, &[0x02, 0x15, 0x28, 0x93, 0xf8, 0x8c, 0xd0, 0x11, 0x89, 0xab, 0x00, 0xa0, 0xc9, 0x05, 0x41, 0x29]);
    call_com(&mut cpu, surf, 0, &[iid, SCRATCH + 0x2200]);
    let tex = r32(SCRATCH + 0x2200);
    clear(&mut cpu, &s, 0xff00_0000);
    call_com(&mut cpu, s.dev, 9, &[]);
    assert_eq!(call_com(&mut cpu, s.dev, 38, &[0, tex]), 0);
    call_com(&mut cpu, s.dev, 22, &[22, 1]); // no culling
    // Quad covering pixels 0..=39 × 0..=39 with uv 0..1 (D3D6 centres: edges at -0.5 / 39.5).
    let v = SCRATCH + 0x800;
    let corners = [(-0.5f32, -0.5f32, 0.0f32, 0.0f32), (39.5, -0.5, 1.0, 0.0), (-0.5, 39.5, 0.0, 1.0), (39.5, 39.5, 1.0, 1.0)];
    for (i, (x, y, u, w)) in corners.iter().enumerate() {
        let a = v + 32 * i as u32;
        put_f32s(a, &[x / 320.0 - 1.0, 1.0 - y / 240.0, 0.5, 0.0]);
        w32(a + 16, 0xffff_ffff);
        w32(a + 20, 0);
        put_f32s(a + 24, &[*u, *w]);
    }
    assert_eq!(call_com(&mut cpu, s.dev, 28, &[5, 0x1e2, v, 4, 0x10]), 0); // TRIANGLESTRIP
    call_com(&mut cpu, s.dev, 10, &[]);
    assert_eq!(pixel(&mut cpu, &s, 5, 5) & 0xff_ffff, 0xff_0000);
    assert_eq!(pixel(&mut cpu, &s, 34, 5) & 0xff_ffff, 0x00_ff00);
    assert_eq!(pixel(&mut cpu, &s, 5, 34) & 0xff_ffff, 0x00_00ff);
    assert_eq!(pixel(&mut cpu, &s, 34, 34) & 0xff_ffff, 0x84_8284);
    // DDCKEY_SRCBLT on green with D3DRENDERSTATE_COLORKEYENABLE: that texel is not drawn.
    let ck = put_bytes(0x5100, &[0xe0, 0x07, 0, 0, 0xe0, 0x07, 0, 0]);
    call_com(&mut cpu, surf, 29, &[8, ck]);
    clear(&mut cpu, &s, 0xff10_2030);
    call_com(&mut cpu, s.dev, 9, &[]);
    call_com(&mut cpu, s.dev, 22, &[41, 1]);
    call_com(&mut cpu, s.dev, 28, &[5, 0x1e2, v, 4, 0x10]);
    call_com(&mut cpu, s.dev, 10, &[]);
    assert_eq!(pixel(&mut cpu, &s, 34, 5) & 0xff_ffff, 0x10_2030, "keyed texel shows the background");
    assert_eq!(pixel(&mut cpu, &s, 5, 5) & 0xff_ffff, 0xff_0000);
}

fn main() {
    let tests: &[(&str, fn())] = &[
        ("clear_and_lock_read_back_exact_xrgb8888", clear_and_lock_read_back_exact_xrgb8888),
        ("prelit_triangle_covers_pixels_by_direct3d6_centres", prelit_triangle_covers_pixels_by_direct3d6_centres),
        ("cpu_writes_survive_later_gpu_drawing", cpu_writes_survive_later_gpu_drawing),
        ("depth_test_lessequal_keeps_the_nearer_triangle", depth_test_lessequal_keeps_the_nearer_triangle),
        ("alpha_blending_srcalpha_invsrcalpha", alpha_blending_srcalpha_invsrcalpha),
        ("sixteen_bit_target_truncates_to_rgb565", sixteen_bit_target_truncates_to_rgb565),
        ("textured_quad_modulate_and_colour_key", textured_quad_modulate_and_colour_key),
    ];
    let filter = std::env::args().skip(1).find(|a| !a.starts_with('-'));
    let mut failed = Vec::new();
    let mut ran = 0;
    let mut ignored = 0;
    for (name, f) in tests {
        if filter.as_deref().is_some_and(|f| !name.contains(f)) {
            continue;
        }
        ran += 1;
        SKIPPED.store(false, Ordering::Relaxed);
        match std::panic::catch_unwind(f) {
            Ok(()) if SKIPPED.load(Ordering::Relaxed) => {
                println!("test {name} ... ignored (no SDL3 GPU device)");
                ignored += 1;
                continue;
            }
            Ok(()) => println!("test {name} ... ok"),
            Err(_) => {
                println!("test {name} ... FAILED");
                failed.push(*name);
            }
        }
    }
    println!("\ntest result: {}. {} passed; {} failed; {ignored} ignored", if failed.is_empty() { "ok" } else { "FAILED" }, ran - ignored - failed.len(), failed.len());
    if !failed.is_empty() {
        std::process::exit(101);
    }
}
