//! Direct3D 6 device state tracking and command recording.

use bugatron_runtime::ddraw::d3d::{ls, rs, tss, Command};
use bugatron_runtime::ddraw::pixel::PixelFormat;
use bugatron_runtime::testing::call_com;
use bugatron_runtime::{with_rt, Cpu};
use bugatron_runtime_tests::*;

const IID_HAL: [u8; 16] = [0xe0, 0x3d, 0xe6, 0x84, 0xaa, 0x46, 0xcf, 0x11, 0x81, 0x6f, 0x00, 0x00, 0xc0, 0x20, 0x15, 0x6e];
const IID_TEXTURE2: [u8; 16] = [0x02, 0x15, 0x28, 0x93, 0xf8, 0x8c, 0xd0, 0x11, 0x89, 0xab, 0x00, 0xa0, 0xc9, 0x05, 0x41, 0x29];

// IDirect3DDevice3 slots
const ADD_VIEWPORT: u32 = 5;
const ENUM_TEXTURE_FORMATS: u32 = 8;
const BEGIN_SCENE: u32 = 9;
const END_SCENE: u32 = 10;
const SET_CURRENT_VIEWPORT: u32 = 12;
const BEGIN: u32 = 16;
const VERTEX: u32 = 18;
const END: u32 = 20;
const GET_RENDER_STATE: u32 = 21;
const SET_RENDER_STATE: u32 = 22;
const SET_LIGHT_STATE: u32 = 24;
const SET_TRANSFORM: u32 = 25;
const DRAW_PRIMITIVE: u32 = 28;
const DRAW_INDEXED_PRIMITIVE: u32 = 29;
const SET_TEXTURE: u32 = 38;
const GET_TSS: u32 = 39;

struct Scene {
    d3d: u32,
    back: u32,
    dev: u32,
    vp: u32,
    dd4: u32,
}

/// The device setup _r3d_CreateDirectX (0x425780) performs.
fn scene(cpu: &mut Cpu) -> Scene {
    let (dd4, _front, back, _z) = display(cpu, 32);
    let d3d = d3d3(cpu, dd4);
    let clsid = put_bytes(0x7000, &IID_HAL);
    let out = SCRATCH + 0x7100;
    assert_eq!(call_com(cpu, d3d, 8, &[clsid, back, out, 0]), 0);
    let dev = r32(out);
    assert_eq!(call_com(cpu, d3d, 6, &[out, 0]), 0);
    let vp = r32(out);
    assert_eq!(call_com(cpu, dev, ADD_VIEWPORT, &[vp]), 0);
    let v2 = SCRATCH + 0x7200;
    for (i, v) in [0x2c, 0, 0, 640, 480].into_iter().enumerate() {
        w32(v2 + 4 * i as u32, v);
    }
    put_f32s(v2 + 20, &[-1.0, 1.0, 2.0, 2.0, 0.0, 1.0]);
    assert_eq!(call_com(cpu, vp, 17, &[v2]), 0);
    assert_eq!(call_com(cpu, dev, SET_CURRENT_VIEWPORT, &[vp]), 0);
    Scene { d3d, back, dev, vp, dd4 }
}

fn commands() -> Vec<Command> {
    with_rt(|rt| std::mem::take(&mut rt.com.d3d.commands))
}

#[test]
fn defaults_follow_direct3d6_with_z_enabled_by_the_attached_zbuffer() {
    let mut cpu = setup();
    let s = scene(&mut cpu);
    let out = SCRATCH + 0x100;
    for (state, want) in [(rs::ZENABLE, 1), (rs::ZWRITEENABLE, 1), (rs::ZFUNC, 4), (rs::CULLMODE, 3), (rs::SHADEMODE, 2), (rs::SRCBLEND, 2), (rs::DESTBLEND, 1), (rs::TEXTUREMAPBLEND, 4)] {
        call_com(&mut cpu, s.dev, GET_RENDER_STATE, &[state as u32, out]);
        assert_eq!(r32(out), want, "render state {state}");
    }
    call_com(&mut cpu, s.dev, GET_TSS, &[0, tss::COLOROP as u32, out]);
    assert_eq!(r32(out), 4, "stage 0 MODULATE");
    call_com(&mut cpu, s.dev, GET_TSS, &[1, tss::COLOROP as u32, out]);
    assert_eq!(r32(out), 1, "stage 1 DISABLE");
}

#[test]
fn legacy_render_states_mirror_onto_stage_zero() {
    let mut cpu = setup();
    let s = scene(&mut cpu);
    let out = SCRATCH + 0x100;
    call_com(&mut cpu, s.dev, SET_RENDER_STATE, &[rs::TEXTUREMAG as u32, 2]); // D3DFILTER_LINEAR
    call_com(&mut cpu, s.dev, SET_RENDER_STATE, &[rs::TEXTUREMIN as u32, 2]);
    call_com(&mut cpu, s.dev, GET_TSS, &[0, tss::MAGFILTER as u32, out]);
    assert_eq!(r32(out), 2, "D3DTFG_LINEAR");
    call_com(&mut cpu, s.dev, GET_TSS, &[0, tss::MINFILTER as u32, out]);
    assert_eq!(r32(out), 2, "D3DTFN_LINEAR");
    call_com(&mut cpu, s.dev, SET_RENDER_STATE, &[rs::TEXTUREADDRESS as u32, 3]); // CLAMP
    call_com(&mut cpu, s.dev, GET_TSS, &[0, tss::ADDRESSU as u32, out]);
    assert_eq!(r32(out), 3);
}

#[test]
fn scenes_must_be_balanced() {
    let mut cpu = setup();
    let s = scene(&mut cpu);
    assert_eq!(call_com(&mut cpu, s.dev, END_SCENE, &[]), 0x8876_0b5c);
    assert_eq!(call_com(&mut cpu, s.dev, BEGIN_SCENE, &[]), 0);
    assert_eq!(call_com(&mut cpu, s.dev, BEGIN_SCENE, &[]), 0x8876_0b5b);
    assert_eq!(call_com(&mut cpu, s.dev, END_SCENE, &[]), 0);
}

#[test]
fn draws_record_vertices_indices_and_a_state_snapshot() {
    let mut cpu = setup();
    let s = scene(&mut cpu);
    // A material selected through its handle, a light, ambient and transforms.
    let out = SCRATCH + 0x100;
    call_com(&mut cpu, s.d3d, 5, &[out, 0]);
    let mat = r32(out);
    let m = SCRATCH + 0x200;
    w32(m, 0x50);
    put_f32s(m + 4, &[0.5, 0.25, 1.0, 1.0, 0.1, 0.1, 0.1, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 8.0]);
    assert_eq!(call_com(&mut cpu, mat, 3, &[m]), 0);
    assert_eq!(call_com(&mut cpu, mat, 5, &[s.dev, out]), 0);
    let h = r32(out);
    call_com(&mut cpu, mat, 5, &[s.dev, out]);
    assert_eq!(r32(out), h, "handles are stable");
    call_com(&mut cpu, s.dev, SET_LIGHT_STATE, &[ls::MATERIAL as u32, h]);
    call_com(&mut cpu, s.dev, SET_LIGHT_STATE, &[ls::AMBIENT as u32, 0xff20_2020]);
    call_com(&mut cpu, s.d3d, 4, &[out, 0]);
    let light = r32(out);
    let l = SCRATCH + 0x300;
    w32(l, 0x50);
    w32(l + 4, 3); // DIRECTIONAL
    put_f32s(l + 8, &[1.0, 1.0, 1.0, 1.0]);
    put_f32s(l + 0x24, &[0.0, 0.0, 1.0]);
    w32(l + 0x4c, 1); // D3DLIGHT_ACTIVE
    call_com(&mut cpu, light, 4, &[l]);
    call_com(&mut cpu, s.vp, 13, &[light]);
    let world = SCRATCH + 0x400;
    put_f32s(world, &[2.0, 0., 0., 0., 0., 2., 0., 0., 0., 0., 2., 0., 1., 2., 3., 1.]);
    call_com(&mut cpu, s.dev, SET_TRANSFORM, &[1, world]);
    // DrawIndexedPrimitive(TRIANGLELIST, D3DFVF_VERTEX, 3 vertices, 3 indices, DONOTUPDATEEXTENTS).
    let verts = SCRATCH + 0x500;
    put_f32s(verts, &[0., 0., 0., 0., 0., -1., 0., 0., 1., 0., 0., 0., 0., -1., 1., 0., 0., 1., 0., 0., 0., -1., 0., 1.]);
    let idx = SCRATCH + 0x600;
    with_rt(|rt| for (i, v) in [0u16, 1, 2].iter().enumerate() { rt.mem.w16(idx + 2 * i as u32, *v) });
    call_com(&mut cpu, s.dev, BEGIN_SCENE, &[]);
    assert_eq!(call_com(&mut cpu, s.dev, DRAW_INDEXED_PRIMITIVE, &[4, 0x112, verts, 3, idx, 3, 8]), 0);
    // An index past the vertex count is rejected.
    with_rt(|rt| rt.mem.w16(idx + 4, 3));
    assert_ne!(call_com(&mut cpu, s.dev, DRAW_INDEXED_PRIMITIVE, &[4, 0x112, verts, 3, idx, 3, 8]), 0);
    // Unsupported vertex formats are rejected.
    assert_eq!(call_com(&mut cpu, s.dev, DRAW_PRIMITIVE, &[4, 0x1, verts, 3, 0]), 0x8876_0819);
    let cmds = commands();
    assert_eq!(cmds.len(), 1);
    let Command::Draw(d) = &cmds[0] else { panic!("expected a draw") };
    assert_eq!((d.primitive, d.fvf, d.flags), (4, 0x112, 8));
    assert_eq!(d.vertices.len(), 3 * 32);
    assert_eq!(d.indices.as_deref(), Some(&[0u16, 1, 2][..]));
    assert_eq!(d.world[12..15], [1.0, 2.0, 3.0]);
    assert_eq!(d.viewport.w, 640);
    assert_eq!(d.lights.len(), 1);
    assert_eq!(d.lights[0].direction, [0.0, 0.0, 1.0]);
    assert_eq!(d.material.unwrap().diffuse, [0.5, 0.25, 1.0, 1.0]);
    assert_eq!(d.material.unwrap().power, 8.0);
    assert_eq!(d.light_state[ls::AMBIENT], 0xff20_2020);
    assert_eq!(d.target, s.back);
    assert!(d.zbuffer.is_some());
    // Inactive lights are not part of the snapshot.
    w32(l + 0x4c, 0);
    call_com(&mut cpu, light, 4, &[l]);
    with_rt(|rt| rt.mem.w16(idx + 4, 2));
    call_com(&mut cpu, s.dev, DRAW_INDEXED_PRIMITIVE, &[4, 0x112, verts, 3, idx, 3, 8]);
    let Command::Draw(d) = &commands()[0] else { panic!() };
    assert!(d.lights.is_empty());
}

#[test]
fn begin_vertex_end_records_one_primitive() {
    let mut cpu = setup();
    let s = scene(&mut cpu);
    // D3DFVF_LVERTEX (0x1e2), 32 bytes per vertex, as r3d_RenderEnd submits alpha polygons.
    assert_eq!(call_com(&mut cpu, s.dev, BEGIN, &[4, 0x1e2, 0x18]), 0);
    for i in 0..3u32 {
        let v = SCRATCH + 0x800 + 32 * i;
        put_f32s(v, &[i as f32, 0.0, 1.0]);
        w32(v + 16, 0x8040_2010 + i);
        assert_eq!(call_com(&mut cpu, s.dev, VERTEX, &[v]), 0);
    }
    assert_eq!(call_com(&mut cpu, s.dev, END, &[0]), 0);
    let cmds = commands();
    let Command::Draw(d) = &cmds[0] else { panic!() };
    assert_eq!((d.primitive, d.fvf, d.flags), (4, 0x1e2, 0x18));
    assert_eq!(d.vertices.len(), 96);
    assert_eq!(dw(&d.vertices, 8 + 4), 0x8040_2011, "second vertex colour");
    assert!(d.indices.is_none());
}

#[test]
fn textures_come_from_surfaces_and_snapshot_their_generation() {
    let mut cpu = setup();
    let s = scene(&mut cpu);
    let d = surface_desc(0x2000);
    w32(d + 4, 0x1007);
    w32(d + 8, 16);
    w32(d + 0xc, 16);
    with_rt(|rt| rt.mem.write(d + 0x48, &PixelFormat::RGB565.to_ddpf()));
    w32(d + 0x68, 0x1000 | 0x800); // TEXTURE | SYSTEMMEMORY
    let (_, surf) = create_surface(&mut cpu, s.dd4, d);
    let iid = put_bytes(0x2100, &IID_TEXTURE2);
    let out = SCRATCH + 0x2200;
    assert_eq!(call_com(&mut cpu, surf, 0, &[iid, out]), 0);
    let tex = r32(out);
    assert_ne!(tex, surf);
    assert_eq!(call_com(&mut cpu, surf, 0, &[iid, out]), 0);
    assert_eq!(r32(out), tex, "one texture interface per surface");
    assert_eq!(call_com(&mut cpu, s.dev, SET_TEXTURE, &[0, tex]), 0);
    assert_ne!(call_com(&mut cpu, s.dev, SET_TEXTURE, &[0, 0x1234]), 0);
    let verts = SCRATCH + 0x500;
    put_f32s(verts, &[0.0; 24]);
    call_com(&mut cpu, s.dev, DRAW_PRIMITIVE, &[6, 0x112, verts, 3, 0]);
    let g0 = with_rt(|rt| rt.com.surface(surf).generation);
    let Command::Draw(dr) = &commands()[0] else { panic!() };
    assert_eq!(dr.textures[0], Some((surf, g0)));
    // Writing texels (Lock/Unlock) changes the generation seen by later draws.
    let ld = surface_desc(0x3000);
    call_com(&mut cpu, surf, 25, &[0, ld, 1, 0]);
    call_com(&mut cpu, surf, 32, &[0]);
    call_com(&mut cpu, s.dev, DRAW_PRIMITIVE, &[6, 0x112, verts, 3, 0]);
    let Command::Draw(dr) = &commands()[0] else { panic!() };
    assert!(dr.textures[0].unwrap().1 > g0);
    let _ = s.back;
}

#[test]
fn clear2_records_rectangles_flags_colour_and_depth() {
    let mut cpu = setup();
    let s = scene(&mut cpu);
    let r = SCRATCH + 0x100;
    for (i, v) in [0u32, 0, 640, 480].into_iter().enumerate() {
        w32(r + 4 * i as u32, v);
    }
    assert_eq!(call_com(&mut cpu, s.vp, 20, &[1, r, 3, 0xff00_0000, 1.0f32.to_bits(), 0]), 0);
    let cmds = commands();
    let Command::Clear { target, zbuffer, rects, flags, color, z } = &cmds[0] else { panic!() };
    assert_eq!((*target, *flags, *color, *z), (s.back, 3, 0xff00_0000, 1.0));
    assert_eq!(rects, &vec![[0, 0, 640, 480]]);
    assert!(zbuffer.is_some());
}

#[test]
fn texture_formats_enumerate_the_reference_set() {
    let mut cpu = setup();
    let s = scene(&mut cpu);
    clear_log();
    assert_eq!(call_com(&mut cpu, s.dev, ENUM_TEXTURE_FORMATS, &[HELPER_REC2_CONTINUE, 9]), 0);
    let l = log();
    assert_eq!(l.len(), 10, "five formats");
    let formats: Vec<PixelFormat> = l.chunks(2).map(|c| PixelFormat::from_ddpf(&read(c[0], 32))).collect();
    // The callback receives a reused buffer: only the last format is still in it.
    assert_eq!(formats.last(), Some(&PixelFormat::RGB555));
}
