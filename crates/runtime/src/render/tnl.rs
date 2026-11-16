//! Direct3D 6 fixed-function transform and lighting, on the CPU.
//!
//! Turns a recorded [`Draw`] into a triangle list of clip-space vertices with
//! per-vertex colours exactly as the Direct3D 6 pipeline hands them to the
//! rasteriser. The GPU then only rasterises, textures, blends and depth-tests.
//!
//! Conventions reproduced here:
//! - row vectors: clip = [x y z 1] · World · View · Projection;
//! - `D3DVIEWPORT2` maps the clip volume (dvClipX, dvClipY, dvClipWidth,
//!   dvClipHeight, dvMinZ, dvMaxZ) onto the viewport rectangle;
//! - Direct3D 6 pixel centres are at integer screen coordinates; modern APIs
//!   (SDL3 GPU) put them at half-integers, so vertices are offset by +½ pixel;
//! - lighting for `D3DVERTEX` (FVF with a normal and no colour): directional,
//!   point and spot lights of the current viewport with the selected material
//!   and the `D3DLIGHTSTATE_AMBIENT` colour, in camera space with a local
//!   viewer; normals are not renormalised;
//! - lit and vertex colours are quantised to `D3DCOLOR` bytes (round to
//!   nearest), the form in which Direct3D 6 delivers them to the rasteriser;
//! - triangle fans and strips expand to lists with Direct3D winding; flat
//!   shading takes each triangle's first vertex colour.
//!
//! Approximations are listed in docs/graphics/renderer.md.

use crate::ddraw::d3d::{ls, rs, Draw, Light, Matrix};

/// One rasteriser vertex.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct GpuVertex {
    /// Clip-space position (x, y, z, w) in the modern convention.
    pub pos: [f32; 4],
    /// Diffuse colour and alpha, each a D3DCOLOR byte / 255.
    pub diffuse: [f32; 4],
    /// Specular colour (alpha unused), each a byte / 255.
    pub specular: [f32; 4],
    pub uv: [f32; 2],
    pub _pad: [f32; 2],
}

#[derive(Debug, Clone, Default)]
pub struct Mesh {
    pub vertices: Vec<GpuVertex>,
    /// Triangle list.
    pub indices: Vec<u32>,
}

/// Layout of one vertex in a flexible vertex format.
#[derive(Debug, Clone, Copy)]
struct Layout {
    stride: usize,
    pretransformed: bool,
    normal: Option<usize>,
    diffuse: Option<usize>,
    specular: Option<usize>,
    uv: Option<usize>,
}

fn layout(fvf: u32) -> Result<Layout, String> {
    let mut off = match fvf & 0xe {
        0x2 => 12,
        0x4 => 16,
        _ => return Err(format!("FVF {fvf:#x}: unsupported position type")),
    };
    let pretransformed = fvf & 0xe == 0x4;
    let mut take = |flag: u32, size: usize| {
        if fvf & flag != 0 {
            let o = off;
            off += size;
            Some(o)
        } else {
            None
        }
    };
    let normal = take(0x10, 12);
    let _reserved = take(0x20, 4);
    let diffuse = take(0x40, 4);
    let specular = take(0x80, 4);
    let ntex = (fvf >> 8) & 0xf;
    let uv = (ntex > 0).then_some(off);
    off += 8 * ntex as usize;
    Ok(Layout { stride: off, pretransformed, normal, diffuse, specular, uv })
}

fn rd(b: &[u8], o: usize) -> f32 {
    f32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn rd_u32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

pub fn mat_mul(a: &Matrix, b: &Matrix) -> Matrix {
    let mut m = [0f32; 16];
    for r in 0..4 {
        for c in 0..4 {
            m[r * 4 + c] = (0..4).map(|k| a[r * 4 + k] * b[k * 4 + c]).sum();
        }
    }
    m
}

fn xform4(v: [f32; 4], m: &Matrix) -> [f32; 4] {
    let mut o = [0f32; 4];
    for (c, out) in o.iter_mut().enumerate() {
        *out = v[0] * m[c] + v[1] * m[4 + c] + v[2] * m[8 + c] + v[3] * m[12 + c];
    }
    o
}

fn xform3(v: [f32; 3], m: &Matrix) -> [f32; 3] {
    let mut o = [0f32; 3];
    for (c, out) in o.iter_mut().enumerate() {
        *out = v[0] * m[c] + v[1] * m[4 + c] + v[2] * m[8 + c];
    }
    o
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = dot(v, v).sqrt();
    if l > 0.0 { [v[0] / l, v[1] / l, v[2] / l] } else { v }
}

/// D3DCOLOR (0xAARRGGBB) → [r, g, b, a] in 0..=1.
pub fn unpack_color(c: u32) -> [f32; 4] {
    [((c >> 16) & 0xff) as f32 / 255.0, ((c >> 8) & 0xff) as f32 / 255.0, (c & 0xff) as f32 / 255.0, (c >> 24) as f32 / 255.0]
}

/// Clamp to [0, 1] and quantise to a D3DCOLOR byte (round to nearest), returned as byte / 255.
pub fn quantize(c: f32) -> f32 {
    (c.clamp(0.0, 1.0) * 255.0).round() / 255.0
}

/// Lit (diffuse rgba, specular rgb) for a camera-space position and normal.
fn light_vertex(d: &Draw, p: [f32; 3], n: [f32; 3], view: &Matrix) -> ([f32; 4], [f32; 3]) {
    let Some(m) = d.material else {
        // No material selected: Direct3D lights with a black material.
        return ([0.0, 0.0, 0.0, 0.0], [0.0; 3]);
    };
    let amb = unpack_color(d.light_state[ls::AMBIENT]);
    let mut diff = [0f32; 3];
    let mut spec = [0f32; 3];
    for i in 0..3 {
        diff[i] = m.emissive[i] + m.ambient[i] * amb[i];
    }
    let specular_on = d.render[rs::SPECULARENABLE] != 0 && m.power > 0.0;
    for l in &d.lights {
        let Some((ldir, atten)) = light_direction(l, p, view) else { continue };
        let ndl = dot(n, ldir);
        if ndl <= 0.0 {
            continue;
        }
        for i in 0..3 {
            diff[i] += m.diffuse[i] * l.color[i] * ndl * atten;
        }
        if specular_on && l.flags & 0x2 == 0 {
            // D3DLIGHT_NO_SPECULAR (0x2) suppresses the specular term.
            let v = normalize([-p[0], -p[1], -p[2]]);
            let h = normalize([ldir[0] + v[0], ldir[1] + v[1], ldir[2] + v[2]]);
            let ndh = dot(n, h);
            if ndh > 0.0 {
                let s = ndh.powf(m.power) * atten;
                for i in 0..3 {
                    spec[i] += m.specular[i] * l.color[i] * s;
                }
            }
        }
    }
    ([diff[0], diff[1], diff[2], m.diffuse[3]], spec)
}

/// Unit vector from the vertex towards the light (camera space) and attenuation.
fn light_direction(l: &Light, p: [f32; 3], view: &Matrix) -> Option<([f32; 3], f32)> {
    match l.kind {
        3 => {
            // D3DLIGHT_DIRECTIONAL: dvDirection is in world space.
            let d = normalize(xform3(l.direction, view));
            Some(([-d[0], -d[1], -d[2]], 1.0))
        }
        1 | 2 => {
            // POINT / SPOT
            let lp = xform4([l.position[0], l.position[1], l.position[2], 1.0], view);
            let v = [lp[0] - p[0], lp[1] - p[1], lp[2] - p[2]];
            let dist = dot(v, v).sqrt();
            if l.range > 0.0 && dist > l.range {
                return None;
            }
            let a = l.attenuation;
            let atten = 1.0 / (a[0] + a[1] * dist + a[2] * dist * dist).max(f32::MIN_POSITIVE);
            let ldir = normalize(v);
            let spot = if l.kind == 2 {
                let axis = normalize(xform3(l.direction, view));
                let rho = -dot(ldir, axis);
                let (ct, cp) = ((l.theta / 2.0).cos(), (l.phi / 2.0).cos());
                if rho > ct {
                    1.0
                } else if rho <= cp {
                    0.0
                } else {
                    ((rho - cp) / (ct - cp)).powf(l.falloff)
                }
            } else {
                1.0
            };
            Some((ldir, atten * spot))
        }
        _ => None,
    }
}

/// Transform, light and assemble one draw.
pub fn process(d: &Draw) -> Result<Mesh, String> {
    let lay = layout(d.fvf)?;
    let n = d.vertices.len() / lay.stride;
    let world_view = mat_mul(&d.world, &d.view);
    let wvp = mat_mul(&world_view, &d.projection);
    let vp = &d.viewport;
    let (vw, vh) = (vp.w.max(1) as f32, vp.h.max(1) as f32);
    let lit = lay.normal.is_some() && lay.diffuse.is_none() && d.flags & 0x10 == 0 && !lay.pretransformed;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let b = &d.vertices[i * lay.stride..(i + 1) * lay.stride];
        let pos = [rd(b, 0), rd(b, 4), rd(b, 8)];
        let mut clip = if lay.pretransformed {
            // D3DTLVERTEX: screen x, y, z and rhw. Rebuild clip coordinates in the
            // viewport's frame so the GPU's viewport transform reproduces them.
            let rhw = rd(b, 12);
            let w = if rhw != 0.0 { 1.0 / rhw } else { 1.0 };
            let xn = (pos[0] - vp.x as f32) / vw * 2.0 - 1.0;
            let yn = 1.0 - (pos[1] - vp.y as f32) / vh * 2.0;
            [xn * w, yn * w, pos[2] * w, w]
        } else {
            let c = xform4([pos[0], pos[1], pos[2], 1.0], &wvp);
            // D3DVIEWPORT2 clip volume → standard clip space (x, y in [-w, w], z in [0, w]).
            let (cx, cy, cw, ch) = (vp.clip_x, vp.clip_y, vp.clip_w, vp.clip_h);
            let (zn, zf) = (vp.min_z, vp.max_z);
            let x = if cw != 0.0 { 2.0 * (c[0] - cx * c[3]) / cw - c[3] } else { c[0] };
            let y = if ch != 0.0 { 2.0 * (c[1] - (cy - ch) * c[3]) / ch - c[3] } else { c[1] };
            let z = if zf != zn { (c[2] - zn * c[3]) / (zf - zn) } else { c[2] };
            [x, y, z, c[3]]
        };
        // Direct3D 6 pixel centres are at integers: shift by +½ pixel (right, down).
        clip[0] += clip[3] / vw;
        clip[1] -= clip[3] / vh;
        let (mut diffuse, mut specular) = ([1.0f32, 1.0, 1.0, 1.0], [0.0f32; 3]);
        if lit {
            let o = lay.normal.unwrap();
            let nrm = xform3([rd(b, o), rd(b, o + 4), rd(b, o + 8)], &world_view);
            let pc = xform4([pos[0], pos[1], pos[2], 1.0], &world_view);
            let (dc, sc) = light_vertex(d, [pc[0], pc[1], pc[2]], nrm, &d.view);
            diffuse = dc;
            specular = sc;
        } else {
            if let Some(o) = lay.diffuse {
                diffuse = unpack_color(rd_u32(b, o));
            }
            if let Some(o) = lay.specular {
                let s = unpack_color(rd_u32(b, o));
                specular = [s[0], s[1], s[2]];
            }
        }
        let uv = lay.uv.map(|o| [rd(b, o), rd(b, o + 4)]).unwrap_or([0.0; 2]);
        out.push(GpuVertex {
            pos: clip,
            diffuse: diffuse.map(quantize),
            specular: [quantize(specular[0]), quantize(specular[1]), quantize(specular[2]), 0.0],
            uv,
            _pad: [0.0; 2],
        });
    }
    let order: Vec<u32> = match &d.indices {
        Some(ix) => ix.iter().map(|&i| i as u32).collect(),
        None => (0..n as u32).collect(),
    };
    let tris = triangles(d.primitive, &order)?;
    let mut mesh = Mesh { vertices: out, indices: tris };
    if d.render[rs::SHADEMODE] == 1 {
        flat_shade(&mut mesh);
    }
    Ok(mesh)
}

/// Expand a primitive to a triangle list with Direct3D winding.
pub fn triangles(primitive: u32, order: &[u32]) -> Result<Vec<u32>, String> {
    let mut t = Vec::new();
    match primitive {
        4 => t.extend_from_slice(&order[..order.len() / 3 * 3]),
        5 => {
            // TRIANGLESTRIP: odd triangles swap their first two vertices.
            for i in 0..order.len().saturating_sub(2) {
                if i % 2 == 0 {
                    t.extend_from_slice(&[order[i], order[i + 1], order[i + 2]]);
                } else {
                    t.extend_from_slice(&[order[i + 1], order[i], order[i + 2]]);
                }
            }
        }
        6 => {
            // TRIANGLEFAN
            for i in 1..order.len().saturating_sub(1) {
                t.extend_from_slice(&[order[0], order[i], order[i + 1]]);
            }
        }
        p => return Err(format!("primitive type {p} (points/lines) is not emulated")),
    }
    Ok(t)
}

/// D3DSHADE_FLAT: every triangle takes its first vertex's colours.
fn flat_shade(m: &mut Mesh) {
    let mut verts = Vec::with_capacity(m.indices.len());
    for tri in m.indices.chunks(3) {
        let first = m.vertices[tri[0] as usize];
        for &i in tri {
            let mut v = m.vertices[i as usize];
            v.diffuse = first.diffuse;
            v.specular = first.specular;
            verts.push(v);
        }
    }
    m.indices = (0..verts.len() as u32).collect();
    m.vertices = verts;
}
