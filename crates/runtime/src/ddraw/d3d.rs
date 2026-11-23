//! Direct3D 6 immediate mode: `IDirect3DDevice3`, `IDirect3DViewport3`,
//! `IDirect3DLight`, `IDirect3DMaterial3` and `IDirect3DTexture2`.
//!
//! The device tracks the original's state exactly (render states, texture
//! stage states, light states, transforms, the current viewport, lights,
//! materials and textures) with the documented Direct3D 6 defaults. Draw calls
//! and clears are recorded, with a snapshot of the state they depend on, into
//! [`State::commands`] for the renderer (crate::render).

use super::{arg, iface, qi_result, read_guid, ret, Kind, DDERR_INVALIDPARAMS, DD_OK};
use crate::{call_guest_stdcall, fatal, with_rt, Cpu, Runtime};

pub const D3DERR_INVALIDVERTEXTYPE: u32 = 0x8876_0819;
pub const D3DERR_SCENE_IN_SCENE: u32 = 0x8876_0b5b;
pub const D3DERR_SCENE_NOT_IN_SCENE: u32 = 0x8876_0b5c;
pub const D3DERR_VIEWPORTHASNODEVICE: u32 = 0x8876_08ab;

/// `D3DRENDERSTATETYPE` values the emulation interprets.
pub mod rs {
    pub const TEXTUREHANDLE: usize = 1;
    pub const TEXTUREADDRESS: usize = 3;
    pub const TEXTUREPERSPECTIVE: usize = 4;
    pub const ZENABLE: usize = 7;
    pub const FILLMODE: usize = 8;
    pub const SHADEMODE: usize = 9;
    pub const ZWRITEENABLE: usize = 14;
    pub const ALPHATESTENABLE: usize = 15;
    pub const LASTPIXEL: usize = 16;
    pub const TEXTUREMAG: usize = 17;
    pub const TEXTUREMIN: usize = 18;
    pub const SRCBLEND: usize = 19;
    pub const DESTBLEND: usize = 20;
    pub const TEXTUREMAPBLEND: usize = 21;
    pub const CULLMODE: usize = 22;
    pub const ZFUNC: usize = 23;
    pub const ALPHAREF: usize = 24;
    pub const ALPHAFUNC: usize = 25;
    pub const DITHERENABLE: usize = 26;
    pub const ALPHABLENDENABLE: usize = 27;
    pub const FOGENABLE: usize = 28;
    pub const SPECULARENABLE: usize = 29;
    pub const FOGCOLOR: usize = 34;
    pub const COLORKEYENABLE: usize = 41;
    pub const TEXTUREADDRESSU: usize = 44;
    pub const TEXTUREADDRESSV: usize = 45;
    pub const COUNT: usize = 256;
}

/// `D3DTEXTURESTAGESTATETYPE` values.
pub mod tss {
    pub const COLOROP: usize = 1;
    pub const COLORARG1: usize = 2;
    pub const COLORARG2: usize = 3;
    pub const ALPHAOP: usize = 4;
    pub const ALPHAARG1: usize = 5;
    pub const ALPHAARG2: usize = 6;
    pub const TEXCOORDINDEX: usize = 11;
    pub const ADDRESS: usize = 12;
    pub const ADDRESSU: usize = 13;
    pub const ADDRESSV: usize = 14;
    pub const MAGFILTER: usize = 16;
    pub const MINFILTER: usize = 17;
    pub const MIPFILTER: usize = 18;
    pub const COUNT: usize = 32;
    pub const STAGES: usize = 8;
}

/// `D3DLIGHTSTATETYPE` values.
pub mod ls {
    pub const MATERIAL: usize = 1;
    pub const AMBIENT: usize = 2;
    pub const COLORMODEL: usize = 3;
    pub const FOGMODE: usize = 4;
    pub const FOGSTART: usize = 5;
    pub const FOGEND: usize = 6;
    pub const FOGDENSITY: usize = 7;
    pub const COLORVERTEX: usize = 8;
    pub const COUNT: usize = 9;
}

pub type Matrix = [f32; 16];
pub const IDENTITY: Matrix = [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.];

/// `D3DLIGHT2` (0x50 bytes) as set by the game.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Light {
    pub kind: u32,
    pub color: [f32; 4],
    pub position: [f32; 3],
    pub direction: [f32; 3],
    pub range: f32,
    pub falloff: f32,
    pub attenuation: [f32; 3],
    pub theta: f32,
    pub phi: f32,
    pub flags: u32,
}

/// `D3DMATERIAL` (0x50 bytes).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Material {
    pub diffuse: [f32; 4],
    pub ambient: [f32; 4],
    pub specular: [f32; 4],
    pub emissive: [f32; 4],
    pub power: f32,
    pub texture: u32,
    pub ramp_size: u32,
}

/// `D3DVIEWPORT2`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Viewport {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub clip_x: f32,
    pub clip_y: f32,
    pub clip_w: f32,
    pub clip_h: f32,
    pub min_z: f32,
    pub max_z: f32,
}

#[derive(Debug, Clone, Default)]
pub struct ViewportObj {
    pub device: u32,
    pub vp: Viewport,
    pub lights: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct Device {
    pub target: u32,
    pub render: Vec<u32>,
    pub stages: Vec<[u32; tss::COUNT]>,
    pub light_state: [u32; ls::COUNT],
    pub world: Matrix,
    pub view: Matrix,
    pub projection: Matrix,
    pub viewports: Vec<u32>,
    pub current_viewport: u32,
    /// Bound `IDirect3DTexture2` per stage (0 = none).
    pub textures: [u32; tss::STAGES],
    pub in_scene: bool,
    /// Primitive under construction between Begin and End: (type, FVF, flags, vertex bytes).
    pub begun: Option<(u32, u32, u32, Vec<u8>)>,
}

impl Device {
    /// Direct3D 6 defaults (IDirect3DDevice3 documentation). Z is enabled when
    /// the render target has an attached Z-buffer at creation.
    pub fn new(target: u32, has_z: bool) -> Device {
        let mut r = vec![0u32; rs::COUNT];
        r[rs::TEXTUREADDRESS] = 1; // WRAP
        r[rs::TEXTUREPERSPECTIVE] = 1;
        r[rs::ZENABLE] = has_z as u32;
        r[rs::FILLMODE] = 3; // SOLID
        r[rs::SHADEMODE] = 2; // GOURAUD
        r[rs::ZWRITEENABLE] = 1;
        r[rs::LASTPIXEL] = 1;
        r[rs::TEXTUREMAG] = 1; // NEAREST
        r[rs::TEXTUREMIN] = 1;
        r[rs::SRCBLEND] = 2; // ONE
        r[rs::DESTBLEND] = 1; // ZERO
        r[rs::TEXTUREMAPBLEND] = 4; // MODULATE
        r[rs::CULLMODE] = 3; // CCW
        r[rs::ZFUNC] = 4; // LESSEQUAL
        r[rs::ALPHAFUNC] = 8; // ALWAYS
        r[rs::SPECULARENABLE] = 1;
        r[rs::TEXTUREADDRESSU] = 1;
        r[rs::TEXTUREADDRESSV] = 1;
        let mut stages = vec![[0u32; tss::COUNT]; tss::STAGES];
        for (i, s) in stages.iter_mut().enumerate() {
            s[tss::COLOROP] = if i == 0 { 4 } else { 1 }; // MODULATE / DISABLE
            s[tss::COLORARG1] = 2; // TEXTURE
            s[tss::COLORARG2] = 1; // CURRENT
            s[tss::ALPHAOP] = if i == 0 { 2 } else { 1 }; // SELECTARG1 / DISABLE
            s[tss::ALPHAARG1] = 2;
            s[tss::ALPHAARG2] = 1;
            s[tss::TEXCOORDINDEX] = i as u32;
            s[tss::ADDRESS] = 1;
            s[tss::ADDRESSU] = 1;
            s[tss::ADDRESSV] = 1;
            s[tss::MAGFILTER] = 1; // POINT
            s[tss::MINFILTER] = 1;
            s[tss::MIPFILTER] = 1; // NONE
        }
        let mut light_state = [0u32; ls::COUNT];
        light_state[ls::COLORMODEL] = 2; // RGB
        light_state[ls::FOGDENSITY] = 1.0f32.to_bits();
        light_state[ls::COLORVERTEX] = 1;
        Device {
            target, render: r, stages, light_state, world: IDENTITY, view: IDENTITY, projection: IDENTITY,
            viewports: Vec::new(), current_viewport: 0, textures: [0; tss::STAGES], in_scene: false, begun: None,
        }
    }
}

/// A recorded clear or draw, with the state it depends on.
#[derive(Debug, Clone)]
pub enum Command {
    /// `Viewport3::Clear2`: rectangles in render-target pixels.
    Clear { target: u32, zbuffer: Option<u32>, rects: Vec<[i32; 4]>, flags: u32, color: u32, z: f32 },
    Draw(Box<Draw>),
}

#[derive(Debug, Clone)]
pub struct Draw {
    pub target: u32,
    pub zbuffer: Option<u32>,
    pub viewport: Viewport,
    pub primitive: u32,
    pub fvf: u32,
    /// D3DDP_* flags of the call.
    pub flags: u32,
    pub vertices: Vec<u8>,
    pub indices: Option<Vec<u16>>,
    pub render: Vec<u32>,
    pub stages: Vec<[u32; tss::COUNT]>,
    pub light_state: [u32; ls::COUNT],
    pub world: Matrix,
    pub view: Matrix,
    pub projection: Matrix,
    /// Active lights of the current viewport.
    pub lights: Vec<Light>,
    /// Material selected by `D3DLIGHTSTATE_MATERIAL` (if any).
    pub material: Option<Material>,
    /// Texture surface per stage and its generation when recorded.
    pub textures: Vec<Option<(u32, u64)>>,
}

#[derive(Default)]
pub struct State {
    pub devices: crate::fastmap::FastMap<u32, Device>,
    pub viewports: crate::fastmap::FastMap<u32, ViewportObj>,
    pub lights: crate::fastmap::FastMap<u32, Light>,
    pub materials: crate::fastmap::FastMap<u32, Material>,
    /// `IDirect3DTexture2` object → surface object.
    pub textures: crate::fastmap::FastMap<u32, u32>,
    /// Material handles (`D3DMATERIALHANDLE`) → material objects.
    pub material_handles: crate::fastmap::FastMap<u32, u32>,
    /// Texture handles (`D3DTEXTUREHANDLE`) → texture objects.
    pub texture_handles: crate::fastmap::FastMap<u32, u32>,
    next_handle: u32,
    /// Commands recorded since the renderer last consumed them.
    pub commands: Vec<Command>,
}

impl State {
    fn handle(&mut self) -> u32 {
        self.next_handle += 1;
        self.next_handle
    }
}

fn f(rt: &Runtime, a: u32) -> f32 {
    f32::from_bits(rt.mem.r32(a))
}

fn read_matrix(rt: &Runtime, p: u32) -> Matrix {
    let mut m = [0f32; 16];
    for (i, v) in m.iter_mut().enumerate() {
        *v = f(rt, p + 4 * i as u32);
    }
    m
}

pub(crate) fn read_light(rt: &Runtime, p: u32) -> Light {
    let v3 = |o: u32| [f(rt, p + o), f(rt, p + o + 4), f(rt, p + o + 8)];
    Light {
        kind: rt.mem.r32(p + 4),
        color: [f(rt, p + 8), f(rt, p + 12), f(rt, p + 16), f(rt, p + 20)],
        position: v3(0x18),
        direction: v3(0x24),
        range: f(rt, p + 0x30),
        falloff: f(rt, p + 0x34),
        attenuation: [f(rt, p + 0x38), f(rt, p + 0x3c), f(rt, p + 0x40)],
        theta: f(rt, p + 0x44),
        phi: f(rt, p + 0x48),
        // D3DLIGHT (0x40 bytes) has no dwFlags; such lights are always active.
        flags: if rt.mem.r32(p) >= 0x50 { rt.mem.r32(p + 0x4c) } else { 1 },
    }
}

pub(crate) fn read_material(rt: &Runtime, p: u32) -> Material {
    let c = |o: u32| [f(rt, p + o), f(rt, p + o + 4), f(rt, p + o + 8), f(rt, p + o + 12)];
    Material {
        diffuse: c(4),
        ambient: c(0x14),
        specular: c(0x24),
        emissive: c(0x34),
        power: f(rt, p + 0x44),
        texture: rt.mem.r32(p + 0x48),
        ramp_size: rt.mem.r32(p + 0x4c),
    }
}

fn read_viewport2(rt: &Runtime, p: u32) -> Viewport {
    Viewport {
        x: rt.mem.r32(p + 4), y: rt.mem.r32(p + 8), w: rt.mem.r32(p + 12), h: rt.mem.r32(p + 16),
        clip_x: f(rt, p + 20), clip_y: f(rt, p + 24), clip_w: f(rt, p + 28), clip_h: f(rt, p + 32),
        min_z: f(rt, p + 36), max_z: f(rt, p + 40),
    }
}

/// Size in bytes of one vertex of flexible vertex format `fvf`, or None if unsupported.
pub fn fvf_size(fvf: u32) -> Option<u32> {
    let mut n = match fvf & 0xe {
        0x2 => 12,  // XYZ
        0x4 => 16,  // XYZRHW
        _ => return None,
    };
    if fvf & 0x10 != 0 { n += 12; } // NORMAL
    if fvf & 0x20 != 0 { n += 4; } // RESERVED1
    if fvf & 0x40 != 0 { n += 4; } // DIFFUSE
    if fvf & 0x80 != 0 { n += 4; } // SPECULAR
    let tex = (fvf >> 8) & 0xf;
    if fvf & 0xffff_0000 != 0 {
        return None; // non-default texture coordinate sizes
    }
    Some(n + 8 * tex)
}

/// The Z-buffer attached to `target`, if any.
fn zbuffer_of(rt: &Runtime, target: u32) -> Option<u32> {
    rt.com.surfaces.get(&target)?.attached.iter().copied().find(|a| rt.com.surfaces.get(a).is_some_and(|s| s.fmt.is_depth()))
}

/// Snapshot the device state for one draw call.
fn snapshot(rt: &Runtime, dev: &Device, primitive: u32, fvf: u32, flags: u32, vertices: Vec<u8>, indices: Option<Vec<u16>>) -> Draw {
    let st = &rt.com.d3d;
    let vpo = st.viewports.get(&dev.current_viewport);
    let lights = vpo.map(|v| v.lights.iter().filter_map(|l| st.lights.get(l)).filter(|l| l.flags & 1 != 0).copied().collect()).unwrap_or_default();
    let material = st.material_handles.get(&dev.light_state[ls::MATERIAL]).and_then(|m| st.materials.get(m)).copied();
    // Stage 0 texture: SetTexture, or the legacy TEXTUREHANDLE render state.
    let mut tex_objs = dev.textures;
    if tex_objs[0] == 0 && dev.render[rs::TEXTUREHANDLE] != 0 {
        tex_objs[0] = st.texture_handles.get(&dev.render[rs::TEXTUREHANDLE]).copied().unwrap_or(0);
    }
    let textures = tex_objs.iter().map(|&t| {
        let s = *st.textures.get(&t)?;
        Some((s, rt.com.surfaces.get(&s)?.generation))
    }).collect();
    Draw {
        target: dev.target,
        zbuffer: zbuffer_of(rt, dev.target),
        viewport: vpo.map(|v| v.vp).unwrap_or_default(),
        primitive, fvf, flags, vertices, indices,
        render: dev.render.clone(),
        stages: dev.stages.clone(),
        light_state: dev.light_state,
        world: dev.world, view: dev.view, projection: dev.projection,
        lights, material, textures,
    }
}

pub(super) fn create_device(rt: &mut Runtime, target: u32) -> u32 {
    let has_z = zbuffer_of(rt, target).is_some();
    let dev = rt.new_com(iface::DEVICE3, Kind::Device);
    rt.com.d3d.devices.insert(dev, Device::new(target, has_z));
    dev
}

fn device<'a>(rt: &'a mut Runtime, this: u32) -> &'a mut Device {
    rt.com.d3d.devices.get_mut(&this).unwrap_or_else(|| fatal(&format!("not a device: {this:#x}")))
}

/// `IDirect3D3` methods that create Direct3D objects. Returns false if not handled.
pub(super) fn call_d3d(cpu: &mut Cpu, this: u32, slot: u32) -> bool {
    let _ = this;
    match slot {
        // CreateLight(this, LPDIRECT3DLIGHT*, outer)
        4 => {
            let out = arg(cpu, 1);
            with_rt(|rt| {
                let o = rt.new_com(iface::LIGHT, Kind::Light);
                rt.com.d3d.lights.insert(o, Light { flags: 1, ..Default::default() });
                rt.mem.w32(out, o);
            });
            ret(cpu, DD_OK, 3);
        }
        // CreateMaterial(this, LPDIRECT3DMATERIAL3*, outer)
        5 => {
            let out = arg(cpu, 1);
            with_rt(|rt| {
                let o = rt.new_com(iface::MATERIAL3, Kind::Material);
                rt.com.d3d.materials.insert(o, Material::default());
                rt.mem.w32(out, o);
            });
            ret(cpu, DD_OK, 3);
        }
        // CreateViewport(this, LPDIRECT3DVIEWPORT3*, outer)
        6 => {
            let out = arg(cpu, 1);
            with_rt(|rt| {
                let o = rt.new_com(iface::VIEWPORT3, Kind::Viewport);
                rt.com.d3d.viewports.insert(o, ViewportObj::default());
                rt.mem.w32(out, o);
            });
            ret(cpu, DD_OK, 3);
        }
        // CreateDevice(this, REFCLSID, LPDIRECTDRAWSURFACE4 target, LPDIRECT3DDEVICE3*, outer)
        8 => {
            let (clsid, target, out) = (arg(cpu, 1), arg(cpu, 2), arg(cpu, 3));
            let id = read_guid(clsid);
            if id != super::guid::IID_IDIRECT3DHALDEVICE && id != super::guid::IID_IDIRECT3DRGBDEVICE {
                fatal(&format!("CreateDevice: unknown device {id:02x?}"));
            }
            with_rt(|rt| {
                rt.com.surface(target);
                let d = create_device(rt, target);
                rt.mem.w32(out, d);
            });
            ret(cpu, DD_OK, 5);
        }
        // EvictManagedTextures(this)
        11 => ret(cpu, DD_OK, 1),
        _ => return false,
    }
    true
}

/// Methods of the Direct3D objects. Returns false if not handled.
pub(super) fn call(cpu: &mut Cpu, iface_id: u32, this: u32, slot: u32) -> bool {
    match (iface_id, slot) {
        (_, 0) => {
            let (riid, out) = (arg(cpu, 1), arg(cpu, 2));
            qi_result(cpu, iface_id, read_guid(riid), out, None);
        }
        // ---- IDirect3DDevice3 ----
        // AddViewport(this, viewport)
        (iface::DEVICE3, 5) => {
            let vp = arg(cpu, 1);
            with_rt(|rt| {
                device(rt, this).viewports.push(vp);
                rt.com.d3d.viewports.get_mut(&vp).unwrap_or_else(|| fatal("AddViewport: not a viewport")).device = this;
                rt.com.objs.get_mut(&vp).unwrap().refs += 1;
            });
            ret(cpu, DD_OK, 2);
        }
        // EnumTextureFormats(this, callback, ctx)
        (iface::DEVICE3, 8) => {
            let (cb, ctx) = (arg(cpu, 1), arg(cpu, 2));
            let buf = with_rt(|rt| rt.heap_alloc(32, true));
            for pf in super::caps::TEXTURE_FORMATS.chunks(32) {
                with_rt(|rt| rt.mem.write(buf, pf));
                if call_guest_stdcall(cpu, cb, &[buf, ctx]) == 0 {
                    break;
                }
            }
            with_rt(|rt| rt.heap.free(buf));
            ret(cpu, DD_OK, 3);
        }
        // BeginScene / EndScene
        (iface::DEVICE3, 9) | (iface::DEVICE3, 10) => {
            let begin = slot == 9;
            let r = with_rt(|rt| {
                let d = device(rt, this);
                if d.in_scene == begin {
                    return if begin { D3DERR_SCENE_IN_SCENE } else { D3DERR_SCENE_NOT_IN_SCENE };
                }
                d.in_scene = begin;
                if !begin {
                    crate::render::end_scene(rt);
                }
                DD_OK
            });
            ret(cpu, r, 1);
        }
        // SetCurrentViewport(this, viewport)
        (iface::DEVICE3, 12) => {
            let vp = arg(cpu, 1);
            with_rt(|rt| device(rt, this).current_viewport = vp);
            ret(cpu, DD_OK, 2);
        }
        // Begin(this, primitive, FVF, flags)
        (iface::DEVICE3, 16) => {
            let (p, fvf, flags) = (arg(cpu, 1), arg(cpu, 2), arg(cpu, 3));
            let r = if fvf_size(fvf).is_none() {
                D3DERR_INVALIDVERTEXTYPE
            } else {
                with_rt(|rt| device(rt, this).begun = Some((p, fvf, flags, Vec::new())));
                DD_OK
            };
            ret(cpu, r, 4);
        }
        // Vertex(this, LPVOID vertex)
        (iface::DEVICE3, 18) => {
            let v = arg(cpu, 1);
            with_rt(|rt| {
                let fvf = device(rt, this).begun.as_ref().unwrap_or_else(|| fatal("Vertex outside Begin/End")).1;
                let bytes = rt.mem.slice(v, fvf_size(fvf).unwrap() as usize).to_vec();
                device(rt, this).begun.as_mut().unwrap().3.extend_from_slice(&bytes);
            });
            ret(cpu, DD_OK, 2);
        }
        // End(this, flags)
        (iface::DEVICE3, 20) => {
            with_rt(|rt| {
                let (p, fvf, flags, verts) = device(rt, this).begun.take().unwrap_or_else(|| fatal("End without Begin"));
                let dev = rt.com.d3d.devices[&this].clone();
                let d = snapshot(rt, &dev, p, fvf, flags, verts, None);
                rt.com.d3d.commands.push(Command::Draw(Box::new(d)));
            });
            ret(cpu, DD_OK, 2);
        }
        // GetRenderState(this, state, LPDWORD)
        (iface::DEVICE3, 21) => {
            let (s, out) = (arg(cpu, 1) as usize, arg(cpu, 2));
            with_rt(|rt| {
                let v = device(rt, this).render.get(s).copied().unwrap_or(0);
                rt.mem.w32(out, v);
            });
            ret(cpu, DD_OK, 3);
        }
        // SetRenderState(this, state, value)
        (iface::DEVICE3, 22) => {
            let (s, v) = (arg(cpu, 1) as usize, arg(cpu, 2));
            let r = with_rt(|rt| {
                let d = device(rt, this);
                if s >= rs::COUNT {
                    return DDERR_INVALIDPARAMS;
                }
                d.render[s] = v;
                // Legacy states mirror onto texture stage 0, as Direct3D 6 does.
                match s {
                    rs::TEXTUREMAG => d.stages[0][tss::MAGFILTER] = legacy_filter(v).0,
                    rs::TEXTUREMIN => {
                        let (min, mip) = legacy_filter(v);
                        d.stages[0][tss::MINFILTER] = min;
                        d.stages[0][tss::MIPFILTER] = mip;
                    }
                    rs::TEXTUREADDRESS => {
                        d.stages[0][tss::ADDRESS] = v;
                        d.stages[0][tss::ADDRESSU] = v;
                        d.stages[0][tss::ADDRESSV] = v;
                        d.render[rs::TEXTUREADDRESSU] = v;
                        d.render[rs::TEXTUREADDRESSV] = v;
                    }
                    rs::TEXTUREADDRESSU => d.stages[0][tss::ADDRESSU] = v,
                    rs::TEXTUREADDRESSV => d.stages[0][tss::ADDRESSV] = v,
                    rs::TEXTUREHANDLE => d.textures[0] = 0,
                    _ => {}
                }
                DD_OK
            });
            ret(cpu, r, 3);
        }
        // GetLightState(this, state, LPDWORD)
        (iface::DEVICE3, 23) => {
            let (s, out) = (arg(cpu, 1) as usize, arg(cpu, 2));
            with_rt(|rt| {
                let v = device(rt, this).light_state.get(s).copied().unwrap_or(0);
                rt.mem.w32(out, v);
            });
            ret(cpu, DD_OK, 3);
        }
        // SetLightState(this, state, value)
        (iface::DEVICE3, 24) => {
            let (s, v) = (arg(cpu, 1) as usize, arg(cpu, 2));
            let r = with_rt(|rt| match device(rt, this).light_state.get_mut(s) {
                Some(x) => {
                    *x = v;
                    DD_OK
                }
                None => DDERR_INVALIDPARAMS,
            });
            ret(cpu, r, 3);
        }
        // SetTransform(this, type, LPD3DMATRIX)
        (iface::DEVICE3, 25) => {
            let (t, p) = (arg(cpu, 1), arg(cpu, 2));
            let r = with_rt(|rt| {
                let m = read_matrix(rt, p);
                let d = device(rt, this);
                match t {
                    1 => d.world = m,
                    2 => d.view = m,
                    3 => d.projection = m,
                    _ => return DDERR_INVALIDPARAMS,
                }
                DD_OK
            });
            ret(cpu, r, 3);
        }
        // DrawPrimitive(this, type, FVF, LPVOID vertices, count, flags)
        (iface::DEVICE3, 28) => {
            let (p, fvf, v, n, flags) = (arg(cpu, 1), arg(cpu, 2), arg(cpu, 3), arg(cpu, 4), arg(cpu, 5));
            let r = with_rt(|rt| {
                let Some(sz) = fvf_size(fvf) else { return D3DERR_INVALIDVERTEXTYPE };
                let verts = rt.mem.slice(v, (sz * n) as usize).to_vec();
                let dev = rt.com.d3d.devices[&this].clone();
                let d = snapshot(rt, &dev, p, fvf, flags, verts, None);
                rt.com.d3d.commands.push(Command::Draw(Box::new(d)));
                DD_OK
            });
            ret(cpu, r, 6);
        }
        // DrawIndexedPrimitive(this, type, FVF, vertices, vcount, LPWORD indices, icount, flags)
        (iface::DEVICE3, 29) => {
            let a: Vec<u32> = (1..8).map(|i| arg(cpu, i)).collect();
            let (p, fvf, v, nv, idx, ni, flags) = (a[0], a[1], a[2], a[3], a[4], a[5], a[6]);
            let r = with_rt(|rt| {
                let Some(sz) = fvf_size(fvf) else { return D3DERR_INVALIDVERTEXTYPE };
                let verts = rt.mem.slice(v, (sz * nv) as usize).to_vec();
                let indices: Vec<u16> = (0..ni).map(|i| rt.mem.r16(idx + 2 * i)).collect();
                if indices.iter().any(|&i| i as u32 >= nv) {
                    return DDERR_INVALIDPARAMS;
                }
                let dev = rt.com.d3d.devices[&this].clone();
                let d = snapshot(rt, &dev, p, fvf, flags, verts, Some(indices));
                rt.com.d3d.commands.push(Command::Draw(Box::new(d)));
                DD_OK
            });
            ret(cpu, r, 8);
        }
        // SetTexture(this, stage, LPDIRECT3DTEXTURE2)
        (iface::DEVICE3, 38) => {
            let (stage, t) = (arg(cpu, 1) as usize, arg(cpu, 2));
            let r = with_rt(|rt| {
                if stage >= tss::STAGES || (t != 0 && !rt.com.d3d.textures.contains_key(&t)) {
                    return DDERR_INVALIDPARAMS;
                }
                let d = device(rt, this);
                d.textures[stage] = t;
                if stage == 0 {
                    d.render[rs::TEXTUREHANDLE] = 0;
                }
                DD_OK
            });
            ret(cpu, r, 3);
        }
        // GetTextureStageState(this, stage, type, LPDWORD)
        (iface::DEVICE3, 39) => {
            let (stage, t, out) = (arg(cpu, 1) as usize, arg(cpu, 2) as usize, arg(cpu, 3));
            with_rt(|rt| {
                let v = device(rt, this).stages.get(stage).and_then(|s| s.get(t)).copied().unwrap_or(0);
                rt.mem.w32(out, v);
            });
            ret(cpu, DD_OK, 4);
        }
        // SetTextureStageState(this, stage, type, value)
        (iface::DEVICE3, 40) => {
            let (stage, t, v) = (arg(cpu, 1) as usize, arg(cpu, 2) as usize, arg(cpu, 3));
            let r = with_rt(|rt| match device(rt, this).stages.get_mut(stage).and_then(|s| s.get_mut(t)) {
                Some(x) => {
                    *x = v;
                    DD_OK
                }
                None => DDERR_INVALIDPARAMS,
            });
            ret(cpu, r, 4);
        }
        // ---- IDirect3DViewport3 ----
        // SetViewport2(this, LPD3DVIEWPORT2) / SetViewport(this, LPD3DVIEWPORT) — the game uses SetViewport2
        (iface::VIEWPORT3, 17) => {
            let p = arg(cpu, 1);
            with_rt(|rt| {
                let vp = read_viewport2(rt, p);
                rt.com.d3d.viewports.get_mut(&this).unwrap().vp = vp;
            });
            ret(cpu, DD_OK, 2);
        }
        // GetViewport2(this, LPD3DVIEWPORT2)
        (iface::VIEWPORT3, 16) => {
            let p = arg(cpu, 1);
            with_rt(|rt| {
                let v = rt.com.d3d.viewports[&this].vp;
                let vals = [0x2c, v.x, v.y, v.w, v.h, v.clip_x.to_bits(), v.clip_y.to_bits(), v.clip_w.to_bits(), v.clip_h.to_bits(), v.min_z.to_bits(), v.max_z.to_bits()];
                for (i, x) in vals.into_iter().enumerate() {
                    rt.mem.w32(p + 4 * i as u32, x);
                }
            });
            ret(cpu, DD_OK, 2);
        }
        // AddLight(this, light) / DeleteLight(this, light)
        (iface::VIEWPORT3, 13) | (iface::VIEWPORT3, 14) => {
            let l = arg(cpu, 1);
            with_rt(|rt| {
                let lights = &mut rt.com.d3d.viewports.get_mut(&this).unwrap().lights;
                if slot == 13 {
                    lights.push(l);
                } else {
                    lights.retain(|&x| x != l);
                }
            });
            ret(cpu, DD_OK, 2);
        }
        // Clear2(this, count, LPD3DRECT, flags, color, z, stencil)
        (iface::VIEWPORT3, 20) => {
            let (n, prects, flags, color, z) = (arg(cpu, 1), arg(cpu, 2), arg(cpu, 3), arg(cpu, 4), arg(cpu, 5));
            let r = with_rt(|rt| {
                let vpo = rt.com.d3d.viewports[&this].clone();
                let Some(dev) = rt.com.d3d.devices.get(&vpo.device) else { return D3DERR_VIEWPORTHASNODEVICE };
                let target = dev.target;
                let rects = (0..n).map(|i| {
                    let p = prects + 16 * i;
                    [rt.mem.r32(p) as i32, rt.mem.r32(p + 4) as i32, rt.mem.r32(p + 8) as i32, rt.mem.r32(p + 12) as i32]
                }).collect();
                let zbuffer = zbuffer_of(rt, target);
                rt.com.d3d.commands.push(Command::Clear { target, zbuffer, rects, flags, color, z: f32::from_bits(z) });
                DD_OK
            });
            ret(cpu, r, 7);
        }
        // ---- IDirect3DLight ----
        // SetLight(this, LPD3DLIGHT) / GetLight
        (iface::LIGHT, 4) => {
            let p = arg(cpu, 1);
            with_rt(|rt| {
                let l = read_light(rt, p);
                rt.com.d3d.lights.insert(this, l);
            });
            ret(cpu, DD_OK, 2);
        }
        // ---- IDirect3DMaterial3 ----
        // SetMaterial(this, LPD3DMATERIAL)
        (iface::MATERIAL3, 3) => {
            let p = arg(cpu, 1);
            with_rt(|rt| {
                let m = read_material(rt, p);
                rt.com.d3d.materials.insert(this, m);
            });
            ret(cpu, DD_OK, 2);
        }
        // GetHandle(this, LPDIRECT3DDEVICE3, LPD3DMATERIALHANDLE)
        (iface::MATERIAL3, 5) => {
            let out = arg(cpu, 2);
            with_rt(|rt| {
                let existing = rt.com.d3d.material_handles.iter().find(|(_, &o)| o == this).map(|(&h, _)| h);
                let h = existing.unwrap_or_else(|| {
                    let h = rt.com.d3d.handle();
                    rt.com.d3d.material_handles.insert(h, this);
                    h
                });
                rt.mem.w32(out, h);
            });
            ret(cpu, DD_OK, 3);
        }
        // ---- IDirect3DTexture2 ----
        // GetHandle(this, LPDIRECT3DDEVICE2, LPD3DTEXTUREHANDLE)
        (iface::TEXTURE2, 3) => {
            let out = arg(cpu, 2);
            with_rt(|rt| {
                let existing = rt.com.d3d.texture_handles.iter().find(|(_, &o)| o == this).map(|(&h, _)| h);
                let h = existing.unwrap_or_else(|| {
                    let h = rt.com.d3d.handle();
                    rt.com.d3d.texture_handles.insert(h, this);
                    h
                });
                rt.mem.w32(out, h);
            });
            ret(cpu, DD_OK, 3);
        }
        // Load(this, LPDIRECT3DTEXTURE2 source): copy the source surface's pixels.
        (iface::TEXTURE2, 5) => {
            let src = arg(cpu, 1);
            let r = with_rt(|rt| {
                let (Some(&ds), Some(&ss)) = (rt.com.d3d.textures.get(&this), rt.com.d3d.textures.get(&src)) else {
                    return DDERR_INVALIDPARAMS;
                };
                let (d, s) = (rt.com.surface(ds).clone(), rt.com.surface(ss).clone());
                if (d.w, d.h, d.fmt) != (s.w, s.h, s.fmt) {
                    return DDERR_INVALIDPARAMS;
                }
                super::surface::sync_for_cpu(rt, ss);
                let bytes = rt.mem.slice(s.bits, s.size_bytes() as usize).to_vec();
                rt.mem.write(d.bits, &bytes[..d.size_bytes() as usize]);
                let ck = s.ck_src;
                let dm = rt.com.surface_mut(ds);
                dm.ck_src = ck;
                super::surface::cpu_modified(rt, ds);
                DD_OK
            });
            ret(cpu, r, 2);
        }
        _ => return false,
    }
    true
}

/// Legacy `D3DFILTER_*` → (`D3DTFG/D3DTFN` filter, `D3DTFP` mip filter).
fn legacy_filter(v: u32) -> (u32, u32) {
    match v {
        1 => (1, 1), // NEAREST → POINT, no mip
        2 => (2, 1), // LINEAR
        3 => (1, 2), // MIPNEAREST
        4 => (2, 2), // MIPLINEAR
        5 => (1, 3), // LINEARMIPNEAREST
        6 => (2, 3), // LINEARMIPLINEAR
        _ => (1, 1),
    }
}

/// `IDirectDrawSurface4::QueryInterface(IID_IDirect3DTexture2)`.
pub(super) fn texture_for_surface(rt: &mut Runtime, surface: u32) -> u32 {
    if let Some((&t, _)) = rt.com.d3d.textures.iter().find(|(_, &s)| s == surface) {
        rt.com.objs.get_mut(&t).unwrap().refs += 1;
        return t;
    }
    let t = rt.new_com(iface::TEXTURE2, Kind::Texture);
    rt.com.d3d.textures.insert(t, surface);
    rt.com.objs.get_mut(&surface).unwrap().refs += 1;
    t
}
