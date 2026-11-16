//! Direct3D 6 transform and lighting on the CPU.

// Links the generated guest code that defines recomp_lookup (MinGW's linker
// does not drop the runtime's unused reference to it).
use bugatron_runtime_tests as _;
use bugatron_runtime::ddraw::d3d::{Device, Draw, Light, Material, Viewport, IDENTITY, ls, rs};
use bugatron_runtime::render::tnl::{process, quantize, triangles};

fn base(fvf: u32, vertices: Vec<f32>) -> Draw {
    let dev = Device::new(0, true);
    Draw {
        target: 0, zbuffer: None,
        viewport: Viewport { x: 0, y: 0, w: 640, h: 480, clip_x: -1.0, clip_y: 1.0, clip_w: 2.0, clip_h: 2.0, min_z: 0.0, max_z: 1.0 },
        primitive: 4, fvf, flags: 0,
        vertices: vertices.iter().flat_map(|v| v.to_le_bytes()).collect(),
        indices: None,
        render: dev.render, stages: dev.stages, light_state: dev.light_state,
        world: IDENTITY, view: IDENTITY, projection: IDENTITY,
        lights: vec![], material: None, textures: vec![None; 8],
    }
}

/// Screen position (modern pixel-centre convention) of clip-space `p` in a 640×480 viewport.
fn screen(p: [f32; 4]) -> (f32, f32) {
    ((p[0] / p[3] + 1.0) * 320.0, (1.0 - p[1] / p[3]) * 240.0)
}

/// A D3DVERTEX (position, normal, uv).
fn vtx(p: [f32; 3], n: [f32; 3]) -> Vec<f32> {
    vec![p[0], p[1], p[2], n[0], n[1], n[2], 0.25, 0.75]
}

#[test]
fn viewport_mapping_and_half_pixel_offset() {
    // D3DLVERTEX with identity transforms: NDC (-1, 1) is D3D6 screen (0, 0),
    // i.e. the centre of pixel (0, 0), which is (0.5, 0.5) in the modern convention.
    let mut v = Vec::new();
    for p in [[-1.0f32, 1.0, 0.5], [1.0, 1.0, 0.5], [-1.0, -1.0, 0.5]] {
        v.extend_from_slice(&p);
        v.push(0.0); // reserved
        v.push(f32::from_bits(0x80ff_0000)); // diffuse
        v.push(f32::from_bits(0x0000_ff00)); // specular
        v.extend_from_slice(&[0.0, 0.0]);
    }
    let m = process(&base(0x1e2, v)).unwrap();
    let (x, y) = screen(m.vertices[0].pos);
    assert!((x - 0.5).abs() < 1e-4 && (y - 0.5).abs() < 1e-4, "({x}, {y})");
    let (x, _) = screen(m.vertices[1].pos);
    assert!((x - 640.5).abs() < 1e-3);
    assert_eq!(m.vertices[0].pos[2], 0.5);
    assert_eq!(m.vertices[0].diffuse, [1.0, 0.0, 0.0, quantize(128.0 / 255.0)]);
    assert_eq!(m.vertices[0].specular[..3], [0.0, 1.0, 0.0]);
    assert_eq!(m.indices, [0, 1, 2]);
}

#[test]
fn custom_clip_volume_maps_onto_the_viewport() {
    // Clip volume x in [0, 4], y in [-2, 2] (dvClipY is the top), z in [0.5, 1].
    let mut d = base(0x1e2, [[2.0f32, 0.0, 0.75], [0.0, 2.0, 0.5], [4.0, -2.0, 1.0]].iter()
        .flat_map(|p| [p[0], p[1], p[2], 0.0, 0.0, 0.0, 0.0, 0.0]).collect());
    d.viewport.clip_x = 0.0;
    d.viewport.clip_y = 2.0;
    d.viewport.clip_w = 4.0;
    d.viewport.clip_h = 4.0;
    d.viewport.min_z = 0.5;
    let m = process(&d).unwrap();
    let (x, y) = screen(m.vertices[0].pos);
    assert!((x - 320.5).abs() < 1e-3 && (y - 240.5).abs() < 1e-3);
    assert!((m.vertices[0].pos[2] - 0.5).abs() < 1e-6);
    let (x, y) = screen(m.vertices[1].pos);
    assert!((x - 0.5).abs() < 1e-3 && (y - 0.5).abs() < 1e-3);
    assert_eq!(m.vertices[1].pos[2], 0.0);
    assert_eq!(m.vertices[2].pos[2], 1.0);
}

#[test]
fn transforms_are_row_vector_world_view_projection() {
    let mut d = base(0x112, [vtx([1.0, 0.0, 0.0], [0.0, 0.0, -1.0]), vtx([0.0; 3], [0.0, 0.0, -1.0]), vtx([0.0; 3], [0.0, 0.0, -1.0])].concat());
    d.world = [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0.5, 0., 0., 1.]; // translate x
    d.view = [0., 1., 0., 0., -1., 0., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.]; // rotate 90° about z
    let m = process(&d).unwrap();
    let p = m.vertices[0].pos;
    // (1 + 0.5, 0, 0) rotated: x' = -y = 0, y' = x = 1.5 (then the ½-pixel offset).
    assert!((p[0] - 1.0 / 640.0).abs() < 1e-6, "{p:?}");
    assert!((p[1] - (1.5 - 1.0 / 480.0)).abs() < 1e-6);
}

fn lit_draw(normal: [f32; 3]) -> Draw {
    let mut d = base(0x112, [vtx([0.0, 0.0, 5.0], normal), vtx([1.0, 0.0, 5.0], normal), vtx([0.0, 1.0, 5.0], normal)].concat());
    d.material = Some(Material {
        diffuse: [0.8, 0.6, 0.4, 0.5], ambient: [1.0, 1.0, 1.0, 1.0], specular: [1.0, 1.0, 1.0, 1.0],
        emissive: [0.0, 0.0, 0.1, 0.0], power: 10.0, texture: 0, ramp_size: 0,
    });
    d.light_state[ls::AMBIENT] = 0xff20_2020;
    // The game's light: directional, white, shining along +z (into the screen).
    d.lights = vec![Light { kind: 3, color: [1.0; 4], direction: [0.0, 0.0, 1.0], flags: 1, ..Default::default() }];
    d
}

#[test]
fn directional_lighting_ambient_emissive_and_specular() {
    // Normal facing the viewer (−z): N·L = 1, and the half vector at the vertex in
    // front of the eye is −z, so the specular term peaks.
    let m = process(&lit_draw([0.0, 0.0, -1.0])).unwrap();
    let a = 0x20 as f32 / 255.0;
    let v = m.vertices[0];
    assert_eq!(v.diffuse, [quantize(a + 0.8), quantize(a + 0.6), quantize(0.1 + a + 0.4), quantize(0.5)]);
    assert_eq!(v.specular[..3], [1.0, 1.0, 1.0]);
    // Back-facing to the light: ambient + emissive only, no specular.
    let m = process(&lit_draw([0.0, 0.0, 1.0])).unwrap();
    let v = m.vertices[0];
    assert_eq!(v.diffuse, [quantize(a), quantize(a), quantize(0.1 + a), quantize(0.5)]);
    assert_eq!(v.specular[..3], [0.0, 0.0, 0.0]);
    // 60° to the light: diffuse scales by cos 60° = 0.5.
    let n = [0.0, (60f32).to_radians().sin(), -(60f32).to_radians().cos()];
    let v = process(&lit_draw(n)).unwrap().vertices[0];
    assert_eq!(v.diffuse[0], quantize(a + 0.8 * 0.5));
}

#[test]
fn specular_can_be_disabled_by_state_or_light_flag() {
    let mut d = lit_draw([0.0, 0.0, -1.0]);
    d.render[rs::SPECULARENABLE] = 0;
    assert_eq!(process(&d).unwrap().vertices[0].specular[..3], [0.0; 3]);
    let mut d = lit_draw([0.0, 0.0, -1.0]);
    d.lights[0].flags |= 0x2; // D3DLIGHT_NO_SPECULAR
    assert_eq!(process(&d).unwrap().vertices[0].specular[..3], [0.0; 3]);
}

#[test]
fn unlit_draws_without_a_material_are_black_and_donotlight_keeps_white() {
    let mut d = lit_draw([0.0, 0.0, -1.0]);
    d.material = None;
    assert_eq!(process(&d).unwrap().vertices[0].diffuse, [0.0; 4]);
    let mut d = lit_draw([0.0, 0.0, -1.0]);
    d.flags = 0x10; // D3DDP_DONOTLIGHT
    assert_eq!(process(&d).unwrap().vertices[0].diffuse, [1.0; 4]);
}

#[test]
fn texture_coordinates_pass_through() {
    let m = process(&lit_draw([0.0, 0.0, -1.0])).unwrap();
    assert_eq!(m.vertices[2].uv, [0.25, 0.75]);
}

#[test]
fn strips_fans_and_indices_keep_direct3d_winding() {
    assert_eq!(triangles(5, &[0, 1, 2, 3, 4]).unwrap(), [0, 1, 2, 2, 1, 3, 2, 3, 4]);
    assert_eq!(triangles(6, &[7, 8, 9, 10]).unwrap(), [7, 8, 9, 7, 9, 10]);
    assert_eq!(triangles(4, &[3, 2, 1, 0]).unwrap(), [3, 2, 1], "incomplete triangles are dropped");
    assert!(triangles(2, &[0, 1]).is_err(), "lines are not emulated");
    let mut d = lit_draw([0.0, 0.0, -1.0]);
    d.indices = Some(vec![2, 1, 0]);
    assert_eq!(process(&d).unwrap().indices, [2, 1, 0]);
}

#[test]
fn flat_shading_uses_the_first_vertex_of_each_triangle() {
    let mut v = Vec::new();
    for c in [0xffff_0000u32, 0xff00_ff00, 0xff00_00ff, 0xffff_ffff] {
        v.extend_from_slice(&[0.0, 0.0, 0.5, 0.0, f32::from_bits(c), 0.0, 0.0, 0.0]);
    }
    let mut d = base(0x1e2, v);
    d.primitive = 6;
    d.render[rs::SHADEMODE] = 1;
    let m = process(&d).unwrap();
    assert_eq!(m.indices.len(), 6);
    for tri in m.indices.chunks(3) {
        let c: Vec<[f32; 4]> = tri.iter().map(|&i| m.vertices[i as usize].diffuse).collect();
        assert!(c.iter().all(|x| *x == [1.0, 0.0, 0.0, 1.0]), "fan centre colour: {c:?}");
    }
}

#[test]
fn pretransformed_vertices_keep_their_screen_position() {
    // D3DTLVERTEX at screen (100, 50), z 0.25, rhw 0.5.
    let v = vec![100.0, 50.0, 0.25, 0.5, f32::from_bits(0xffffffff), 0.0, 0.0, 0.0,
                 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
    let m = process(&base(0x1c4, v)).unwrap();
    let p = m.vertices[0].pos;
    assert_eq!(p[3], 2.0);
    let (x, y) = screen(p);
    assert!((x - 100.5).abs() < 1e-3 && (y - 50.5).abs() < 1e-3, "({x}, {y})");
    assert!((p[2] / p[3] - 0.25).abs() < 1e-6);
}
