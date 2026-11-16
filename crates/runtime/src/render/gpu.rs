//! SDL3 GPU backend: executes recorded Direct3D 6 commands and presents frames.
//!
//! Guest-visible DirectDraw/Direct3D capabilities (ddraw::caps) are independent
//! of what the host GPU can do: this module only has to reproduce the
//! documented Direct3D 6 behaviour of the commands the game issues.
//!
//! - Render targets and Z-buffers get GPU textures keyed by their guest pixel
//!   storage (`Surface::bits`), so `Flip` (which swaps storage between the
//!   front and back buffer) carries the GPU textures along. Colour targets are
//!   RGBA8; a Z16 buffer is a 16-bit `D16_UNORM` depth texture.
//! - Coherence: each storage records whether the GPU holds newer pixels than
//!   guest memory. CPU access (Lock, CPU blits) flushes pending work and reads
//!   the pixels back in the guest format; CPU writes (tracked by
//!   `Surface::generation`) are uploaded before the next GPU use, unless that
//!   use starts with a full clear.
//! - Texture surfaces are uploaded when their generation changes, expanded to
//!   RGBA8 by bit replication; colour-keyed texels become transparent.
//! - Recorded commands execute at `EndScene`, before CPU access, and before
//!   presentation: one copy pass (vertices, indices, textures), then render passes.
//! - Presentation scales the guest front buffer into the window
//!   (aspect-preserving, letterboxed) with a non-blocking present mode, so
//!   presentation never paces the simulation (clock.rs does).

use super::tnl::{self, GpuVertex};
use crate::ddraw::d3d::{rs, tss, Command, Draw};
use crate::{fatal, Runtime};
use sdl3_sys::error::SDL_GetError;
use sdl3_sys::gpu::*;
use sdl3_sys::video::SDL_Window;
use std::collections::HashMap;
use std::ffi::{c_void, CStr, CString};
use std::ptr::null_mut;

fn sdl_err(what: &str) -> ! {
    // SAFETY: SDL_GetError returns a valid C string.
    let e = unsafe { CStr::from_ptr(SDL_GetError()) }.to_string_lossy();
    fatal(&format!("{what}: {e}"))
}

const RT_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM;
const DEPTH_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_D16_UNORM;

/// Uniforms of `shaders/fixed.wgsl` (`struct Stage`).
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct StageUniforms {
    color: [u32; 4],
    alpha: [u32; 4],
    test: [u32; 4],
    tfactor: [f32; 4],
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct PipelineKey {
    blend: bool,
    src: u32,
    dst: u32,
    z_test: bool,
    z_write: bool,
    z_func: u32,
    cull: u32,
    has_depth: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct SamplerKey {
    mag: u32,
    min: u32,
    u: u32,
    v: u32,
}

struct Storage {
    tex: *mut SDL_GPUTexture,
    w: u32,
    h: u32,
    depth: bool,
    /// The GPU texture holds pixels newer than guest memory.
    gpu_newer: bool,
    /// Generation of guest memory last uploaded (or read back).
    synced_generation: u64,
}

struct Texture {
    tex: *mut SDL_GPUTexture,
    w: u32,
    h: u32,
    generation: u64,
    keyed: bool,
}

pub struct Gpu {
    device: *mut SDL_GPUDevice,
    window: *mut SDL_Window,
    vs: *mut SDL_GPUShader,
    fs: *mut SDL_GPUShader,
    pipelines: HashMap<PipelineKey, *mut SDL_GPUGraphicsPipeline>,
    samplers: HashMap<SamplerKey, *mut SDL_GPUSampler>,
    /// Render targets and Z-buffers by guest storage address.
    storage: HashMap<u32, Storage>,
    /// Texture surfaces by surface object.
    textures: HashMap<u32, Texture>,
    white: *mut SDL_GPUTexture,
    vbuf: (*mut SDL_GPUBuffer, u32),
    ibuf: (*mut SDL_GPUBuffer, u32),
}

impl Gpu {
    /// Create the device for `window` (null: offscreen, for tests). Returns None
    /// when no GPU backend is available.
    pub fn new(window: *mut SDL_Window) -> Option<Gpu> {
        // SAFETY: plain SDL calls with valid arguments.
        unsafe {
            use sdl3_sys::init::{SDL_Init, SDL_WasInit, SDL_INIT_VIDEO};
            // GPU devices for a window need the video subsystem (initialised by the
            // window layer). Offscreen devices try it, and carry on without it.
            if SDL_WasInit(SDL_INIT_VIDEO) == 0u32 && window.is_null() {
                SDL_Init(SDL_INIT_VIDEO);
            }
            // DXIL is only built for Windows targets (build.rs); without it SDL
            // cannot use its Direct3D 12 backend.
            let mut formats = SDL_GPU_SHADERFORMAT_MSL | SDL_GPU_SHADERFORMAT_SPIRV;
            if !super::shaders::FIXED_VS_MAIN_DXIL.is_empty() {
                formats |= SDL_GPU_SHADERFORMAT_DXIL;
            }
            let device = SDL_CreateGPUDevice(formats, false, std::ptr::null());
            if device.is_null() {
                eprintln!("[render] SDL_CreateGPUDevice: {}", CStr::from_ptr(SDL_GetError()).to_string_lossy());
                return None;
            }
            if crate::trace_enabled("gpu") {
                let name = SDL_GetGPUDeviceDriver(device);
                let name = if name.is_null() { "?".into() } else { CStr::from_ptr(name).to_string_lossy() };
                eprintln!("[render] SDL GPU driver: {name}");
            }
            if !window.is_null() {
                if !SDL_ClaimWindowForGPUDevice(device, window) {
                    sdl_err("SDL_ClaimWindowForGPUDevice");
                }
                // Never block on the display: simulation pacing is clock.rs's job.
                for mode in [SDL_GPU_PRESENTMODE_MAILBOX, SDL_GPU_PRESENTMODE_IMMEDIATE] {
                    if SDL_WindowSupportsGPUPresentMode(device, window, mode) {
                        SDL_SetGPUSwapchainParameters(device, window, SDL_GPU_SWAPCHAINCOMPOSITION_SDR, mode);
                        break;
                    }
                }
            }
            let mut g = Gpu {
                device, window, vs: null_mut(), fs: null_mut(),
                pipelines: HashMap::new(), samplers: HashMap::new(), storage: HashMap::new(), textures: HashMap::new(),
                white: null_mut(), vbuf: (null_mut(), 0), ibuf: (null_mut(), 0),
            };
            g.vs = g.shader(SDL_GPU_SHADERSTAGE_VERTEX, 0, 0);
            g.fs = g.shader(SDL_GPU_SHADERSTAGE_FRAGMENT, 1, 1);
            g.white = g.create_texture(1, 1, false, false);
            g.upload_texture_pixels(g.white, 1, 1, &[255, 255, 255, 255]);
            Some(g)
        }
    }

    fn shader(&self, stage: SDL_GPUShaderStage, samplers: u32, uniforms: u32) -> *mut SDL_GPUShader {
        use super::shaders::*;
        // SAFETY: valid device; code pointers live for the call.
        unsafe {
            let formats = SDL_GetGPUShaderFormats(self.device);
            let vertex = stage == SDL_GPU_SHADERSTAGE_VERTEX;
            let (code, entry, format): (&[u8], &str, SDL_GPUShaderFormat) = if formats & SDL_GPU_SHADERFORMAT_MSL != 0 {
                if vertex { (FIXED_VS_MAIN_MSL.as_bytes(), FIXED_VS_MAIN_MSL_ENTRY, SDL_GPU_SHADERFORMAT_MSL) } else { (FIXED_FS_MAIN_MSL.as_bytes(), FIXED_FS_MAIN_MSL_ENTRY, SDL_GPU_SHADERFORMAT_MSL) }
            } else if formats & SDL_GPU_SHADERFORMAT_DXIL != 0 && !FIXED_VS_MAIN_DXIL.is_empty() {
                // HLSL from SPIRV-Cross names every entry point `main`.
                if vertex { (FIXED_VS_MAIN_DXIL, "main", SDL_GPU_SHADERFORMAT_DXIL) } else { (FIXED_FS_MAIN_DXIL, "main", SDL_GPU_SHADERFORMAT_DXIL) }
            } else {
                if vertex { (FIXED_VS_MAIN_SPV, "vs_main", SDL_GPU_SHADERFORMAT_SPIRV) } else { (FIXED_FS_MAIN_SPV, "fs_main", SDL_GPU_SHADERFORMAT_SPIRV) }
            };
            let entry = CString::new(entry).unwrap();
            let mut src = code.to_vec();
            if format == SDL_GPU_SHADERFORMAT_MSL {
                src.push(0);
            }
            let info = SDL_GPUShaderCreateInfo {
                code_size: code.len(),
                code: src.as_ptr(),
                entrypoint: entry.as_ptr(),
                format,
                stage,
                num_samplers: samplers,
                num_storage_textures: 0,
                num_storage_buffers: 0,
                num_uniform_buffers: uniforms,
                ..Default::default()
            };
            let s = SDL_CreateGPUShader(self.device, &info);
            if s.is_null() {
                sdl_err("SDL_CreateGPUShader");
            }
            s
        }
    }

    fn create_texture(&self, w: u32, h: u32, target: bool, depth: bool) -> *mut SDL_GPUTexture {
        let usage = if depth {
            SDL_GPU_TEXTUREUSAGE_DEPTH_STENCIL_TARGET
        } else if target {
            SDL_GPU_TEXTUREUSAGE_SAMPLER | SDL_GPU_TEXTUREUSAGE_COLOR_TARGET
        } else {
            SDL_GPU_TEXTUREUSAGE_SAMPLER
        };
        let info = SDL_GPUTextureCreateInfo {
            r#type: SDL_GPU_TEXTURETYPE_2D,
            format: if depth { DEPTH_FORMAT } else { RT_FORMAT },
            usage,
            width: w,
            height: h,
            layer_count_or_depth: 1,
            num_levels: 1,
            sample_count: SDL_GPU_SAMPLECOUNT_1,
            ..Default::default()
        };
        // SAFETY: valid device and create info.
        let t = unsafe { SDL_CreateGPUTexture(self.device, &info) };
        if t.is_null() {
            sdl_err("SDL_CreateGPUTexture");
        }
        t
    }

    /// Upload tightly packed RGBA8 pixels to a whole texture (own command buffer).
    fn upload_texture_pixels(&self, tex: *mut SDL_GPUTexture, w: u32, h: u32, rgba: &[u8]) {
        // SAFETY: valid device/texture; transfer buffer sized to the data.
        unsafe {
            let cmd = SDL_AcquireGPUCommandBuffer(self.device);
            let copy = SDL_BeginGPUCopyPass(cmd);
            let tb = self.staged(rgba);
            upload_tex(copy, tb, 0, tex, w, h);
            SDL_EndGPUCopyPass(copy);
            SDL_SubmitGPUCommandBuffer(cmd);
            SDL_ReleaseGPUTransferBuffer(self.device, tb);
        }
    }

    /// A transfer buffer holding `data`.
    unsafe fn staged(&self, data: &[u8]) -> *mut SDL_GPUTransferBuffer {
        let info = SDL_GPUTransferBufferCreateInfo { usage: SDL_GPU_TRANSFERBUFFERUSAGE_UPLOAD, size: data.len().max(4) as u32, ..Default::default() };
        let tb = SDL_CreateGPUTransferBuffer(self.device, &info);
        if tb.is_null() {
            sdl_err("SDL_CreateGPUTransferBuffer");
        }
        let p = SDL_MapGPUTransferBuffer(self.device, tb, false) as *mut u8;
        std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
        SDL_UnmapGPUTransferBuffer(self.device, tb);
        tb
    }

    fn ensure_buffer(&mut self, index: bool, size: u32) -> *mut SDL_GPUBuffer {
        let slot = if index { &mut self.ibuf } else { &mut self.vbuf };
        if slot.1 < size {
            if !slot.0.is_null() {
                // SAFETY: buffer created by this device.
                unsafe { SDL_ReleaseGPUBuffer(self.device, slot.0) };
            }
            let cap = size.next_power_of_two().max(1 << 16);
            let info = SDL_GPUBufferCreateInfo {
                usage: if index { SDL_GPU_BUFFERUSAGE_INDEX } else { SDL_GPU_BUFFERUSAGE_VERTEX },
                size: cap,
                ..Default::default()
            };
            // SAFETY: valid device.
            let b = unsafe { SDL_CreateGPUBuffer(self.device, &info) };
            if b.is_null() {
                sdl_err("SDL_CreateGPUBuffer");
            }
            *slot = (b, cap);
        }
        slot.0
    }

    fn pipeline(&mut self, k: PipelineKey) -> *mut SDL_GPUGraphicsPipeline {
        if let Some(&p) = self.pipelines.get(&k) {
            return p;
        }
        let attrs = [
            (0u32, SDL_GPU_VERTEXELEMENTFORMAT_FLOAT4, 0u32),
            (1, SDL_GPU_VERTEXELEMENTFORMAT_FLOAT4, 16),
            (2, SDL_GPU_VERTEXELEMENTFORMAT_FLOAT4, 32),
            (3, SDL_GPU_VERTEXELEMENTFORMAT_FLOAT2, 48),
        ]
        .map(|(location, format, offset)| SDL_GPUVertexAttribute { location, buffer_slot: 0, format, offset });
        let vb = SDL_GPUVertexBufferDescription {
            slot: 0,
            pitch: std::mem::size_of::<GpuVertex>() as u32,
            input_rate: SDL_GPU_VERTEXINPUTRATE_VERTEX,
            instance_step_rate: 0,
        };
        let blend = SDL_GPUColorTargetBlendState {
            src_color_blendfactor: blend_factor(k.src),
            dst_color_blendfactor: blend_factor(k.dst),
            color_blend_op: SDL_GPU_BLENDOP_ADD,
            src_alpha_blendfactor: blend_factor(k.src),
            dst_alpha_blendfactor: blend_factor(k.dst),
            alpha_blend_op: SDL_GPU_BLENDOP_ADD,
            // The guest formats have no destination alpha: keep it at 1 (DESTALPHA reads 1).
            color_write_mask: SDL_GPU_COLORCOMPONENT_R | SDL_GPU_COLORCOMPONENT_G | SDL_GPU_COLORCOMPONENT_B,
            enable_blend: k.blend,
            enable_color_write_mask: true,
            ..Default::default()
        };
        let ct = SDL_GPUColorTargetDescription { format: RT_FORMAT, blend_state: blend };
        let (cull_mode, front_face) = match k.cull {
            // Direct3D culls by screen-space winding: CCW culls counter-clockwise triangles.
            3 => (SDL_GPU_CULLMODE_BACK, SDL_GPU_FRONTFACE_CLOCKWISE),
            2 => (SDL_GPU_CULLMODE_FRONT, SDL_GPU_FRONTFACE_CLOCKWISE),
            _ => (SDL_GPU_CULLMODE_NONE, SDL_GPU_FRONTFACE_CLOCKWISE),
        };
        let info = SDL_GPUGraphicsPipelineCreateInfo {
            vertex_shader: self.vs,
            fragment_shader: self.fs,
            vertex_input_state: SDL_GPUVertexInputState {
                vertex_buffer_descriptions: &vb,
                num_vertex_buffers: 1,
                vertex_attributes: attrs.as_ptr(),
                num_vertex_attributes: attrs.len() as u32,
            },
            primitive_type: SDL_GPU_PRIMITIVETYPE_TRIANGLELIST,
            rasterizer_state: SDL_GPURasterizerState {
                fill_mode: SDL_GPU_FILLMODE_FILL,
                cull_mode,
                front_face,
                enable_depth_clip: true,
                ..Default::default()
            },
            depth_stencil_state: SDL_GPUDepthStencilState {
                compare_op: SDL_GPUCompareOp(k.z_func as i32), // D3DCMPFUNC and SDL share numbering
                enable_depth_test: k.z_test,
                enable_depth_write: k.z_write,
                ..Default::default()
            },
            target_info: SDL_GPUGraphicsPipelineTargetInfo {
                color_target_descriptions: &ct,
                num_color_targets: 1,
                depth_stencil_format: DEPTH_FORMAT,
                has_depth_stencil_target: k.has_depth,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: all pointers in `info` outlive the call.
        let p = unsafe { SDL_CreateGPUGraphicsPipeline(self.device, &info) };
        if p.is_null() {
            sdl_err("SDL_CreateGPUGraphicsPipeline");
        }
        self.pipelines.insert(k, p);
        p
    }

    fn sampler(&mut self, k: SamplerKey) -> *mut SDL_GPUSampler {
        if let Some(&s) = self.samplers.get(&k) {
            return s;
        }
        let filter = |f: u32| if f == 1 { SDL_GPU_FILTER_NEAREST } else { SDL_GPU_FILTER_LINEAR };
        let addr = |a: u32| match a {
            2 => SDL_GPU_SAMPLERADDRESSMODE_MIRRORED_REPEAT,
            3 | 4 => SDL_GPU_SAMPLERADDRESSMODE_CLAMP_TO_EDGE,
            _ => SDL_GPU_SAMPLERADDRESSMODE_REPEAT,
        };
        let info = SDL_GPUSamplerCreateInfo {
            min_filter: filter(k.min),
            mag_filter: filter(k.mag),
            mipmap_mode: SDL_GPU_SAMPLERMIPMAPMODE_NEAREST,
            address_mode_u: addr(k.u),
            address_mode_v: addr(k.v),
            address_mode_w: addr(k.u),
            ..Default::default()
        };
        // SAFETY: valid device.
        let s = unsafe { SDL_CreateGPUSampler(self.device, &info) };
        if s.is_null() {
            sdl_err("SDL_CreateGPUSampler");
        }
        self.samplers.insert(k, s);
        s
    }

    /// The GPU storage for guest surface `obj`, created on first use.
    fn storage_for(&mut self, rt: &Runtime, obj: u32) -> u32 {
        let s = rt.com.surface(obj);
        if !self.storage.contains_key(&s.bits) {
            let depth = s.fmt.is_depth();
            let tex = self.create_texture(s.w, s.h, !depth, depth);
            // generation 0: guest memory has not been uploaded yet.
            self.storage.insert(s.bits, Storage { tex, w: s.w, h: s.h, depth, gpu_newer: false, synced_generation: 0 });
        }
        s.bits
    }
}

fn blend_factor(d3d: u32) -> SDL_GPUBlendFactor {
    match d3d {
        1 => SDL_GPU_BLENDFACTOR_ZERO,
        2 => SDL_GPU_BLENDFACTOR_ONE,
        3 => SDL_GPU_BLENDFACTOR_SRC_COLOR,
        4 => SDL_GPU_BLENDFACTOR_ONE_MINUS_SRC_COLOR,
        5 | 12 => SDL_GPU_BLENDFACTOR_SRC_ALPHA,
        6 | 13 => SDL_GPU_BLENDFACTOR_ONE_MINUS_SRC_ALPHA,
        7 => SDL_GPU_BLENDFACTOR_DST_ALPHA,
        8 => SDL_GPU_BLENDFACTOR_ONE_MINUS_DST_ALPHA,
        9 => SDL_GPU_BLENDFACTOR_DST_COLOR,
        10 => SDL_GPU_BLENDFACTOR_ONE_MINUS_DST_COLOR,
        11 => SDL_GPU_BLENDFACTOR_SRC_ALPHA_SATURATE,
        _ => SDL_GPU_BLENDFACTOR_ONE,
    }
}

unsafe fn upload_tex(copy: *mut SDL_GPUCopyPass, tb: *mut SDL_GPUTransferBuffer, offset: u32, tex: *mut SDL_GPUTexture, w: u32, h: u32) {
    let src = SDL_GPUTextureTransferInfo { transfer_buffer: tb, offset, pixels_per_row: w, rows_per_layer: h };
    let dst = SDL_GPUTextureRegion { texture: tex, w, h, d: 1, ..Default::default() };
    SDL_UploadToGPUTexture(copy, &src, &dst, false);
}

/// RGBA8 pixels of a texture surface; colour-keyed texels (DDCKEY_SRCBLT range)
/// become fully transparent black when keying is in effect.
fn texture_rgba(rt: &Runtime, obj: u32, keyed: bool) -> Vec<u8> {
    let s = rt.com.surface(obj);
    let mem = rt.mem.slice(s.bits, s.size_bytes() as usize);
    let mut out = s.fmt.to_rgba8(mem, s.w, s.h, s.pitch);
    if let (true, Some((lo, hi))) = (keyed, s.ck_src) {
        let bpp = s.fmt.bytes_per_pixel() as usize;
        for y in 0..s.h as usize {
            for x in 0..s.w as usize {
                let v = s.fmt.load(&mem[y * s.pitch as usize + x * bpp..]);
                if v >= lo && v <= hi {
                    out[(y * s.w as usize + x) * 4..][..4].fill(0);
                }
            }
        }
    }
    out
}

/// Pixels of a render target or Z-buffer in guest memory, as GPU upload bytes.
fn storage_rgba(rt: &Runtime, obj: u32) -> Option<Vec<u8>> {
    let s = rt.com.surface(obj);
    if s.fmt.is_depth() {
        return None; // depth is never written by the CPU in this game
    }
    let mem = rt.mem.slice(s.bits, s.size_bytes() as usize);
    let mut px = s.fmt.to_rgba8(mem, s.w, s.h, s.pitch);
    for a in px.iter_mut().skip(3).step_by(4) {
        *a = 255;
    }
    Some(px)
}

/// A draw ready for submission.
struct Prepared {
    pipeline: PipelineKey,
    sampler: SamplerKey,
    texture: Option<u32>,
    keyed: bool,
    uniforms: StageUniforms,
    viewport: SDL_GPUViewport,
    first_index: u32,
    count: u32,
    base_vertex: i32,
}

fn prepare(d: &Draw) -> (StageUniforms, PipelineKey, SamplerKey, bool) {
    let r = &d.render;
    if r[rs::FOGENABLE] != 0 {
        static WARNED: std::sync::Once = std::sync::Once::new();
        WARNED.call_once(|| eprintln!("[render] D3DRENDERSTATE_FOGENABLE is set but fog is not emulated (r3d never enables it; docs/graphics/renderer.md A13)"));
    }
    let st = &d.stages[0];
    let has_tex = d.textures[0].is_some();
    let keyed = has_tex && r[rs::COLORKEYENABLE] != 0;
    let mut u = StageUniforms {
        color: [st[tss::COLOROP], st[tss::COLORARG1], st[tss::COLORARG2], r[rs::SPECULARENABLE]],
        alpha: [st[tss::ALPHAOP], st[tss::ALPHAARG1], st[tss::ALPHAARG2], 0],
        test: [if r[rs::ALPHATESTENABLE] != 0 { r[rs::ALPHAFUNC] } else { 0 }, r[rs::ALPHAREF], 0, 0],
        tfactor: tnl::unpack_color(r[60]),
    };
    if !has_tex {
        // No texture on stage 0: Direct3D passes the diffuse colour and alpha through.
        u.color[0] = 1; // D3DTOP_DISABLE
        u.alpha[0] = 1;
    }
    if keyed && u.test[0] == 0 {
        // Colour-keyed texels have alpha 0: reject them like the key test does.
        u.test = [6, 0, 0, 0]; // NOTEQUAL 0
    }
    let z_test = r[rs::ZENABLE] != 0 && d.zbuffer.is_some();
    let pk = PipelineKey {
        blend: r[rs::ALPHABLENDENABLE] != 0,
        src: r[rs::SRCBLEND],
        dst: r[rs::DESTBLEND],
        z_test,
        z_write: z_test && r[rs::ZWRITEENABLE] != 0,
        z_func: if r[rs::ZFUNC] == 0 { 4 } else { r[rs::ZFUNC] },
        cull: r[rs::CULLMODE],
        has_depth: d.zbuffer.is_some(),
    };
    // BOTHSRCALPHA / BOTHINVSRCALPHA set both factors from SRCBLEND.
    let mut pk = pk;
    if pk.src == 12 {
        pk.dst = 6;
    } else if pk.src == 13 {
        pk.src = 6;
        pk.dst = 5;
    }
    let sk = SamplerKey { mag: st[tss::MAGFILTER], min: st[tss::MINFILTER], u: st[tss::ADDRESSU], v: st[tss::ADDRESSV] };
    (u, pk, sk, keyed)
}

/// Execute all recorded commands.
pub fn flush(rt: &mut Runtime) {
    if rt.com.d3d.commands.is_empty() {
        return;
    }
    let commands = std::mem::take(&mut rt.com.d3d.commands);
    let Some(mut g) = rt.gpu.take() else { return };
    flush_with(&mut g, rt, commands);
    rt.gpu = Some(g);
}

fn flush_with(g: &mut Gpu, rt: &mut Runtime, commands: Vec<Command>) {
    // 1. Transform and light every draw; gather vertex and index data.
    let mut verts: Vec<GpuVertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut prepared: Vec<Option<Prepared>> = Vec::with_capacity(commands.len());
    for c in &commands {
        let Command::Draw(d) = c else {
            prepared.push(None);
            continue;
        };
        let mesh = match tnl::process(d) {
            Ok(m) => m,
            Err(e) => fatal(&format!("draw: {e}")),
        };
        let (uniforms, pipeline, sampler, keyed) = prepare(d);
        let v = d.viewport;
        prepared.push(Some(Prepared {
            pipeline, sampler, keyed, uniforms,
            texture: d.textures[0].map(|(s, _)| s),
            viewport: SDL_GPUViewport { x: v.x as f32, y: v.y as f32, w: v.w as f32, h: v.h as f32, min_depth: 0.0, max_depth: 1.0 },
            first_index: indices.len() as u32,
            count: mesh.indices.len() as u32,
            base_vertex: verts.len() as i32,
        }));
        verts.extend_from_slice(&mesh.vertices);
        indices.extend_from_slice(&mesh.indices);
    }

    // 2. Uploads: changed textures, CPU-newer render targets, geometry.
    let mut uploads: Vec<(*mut SDL_GPUTexture, u32, u32, Vec<u8>)> = Vec::new();
    for p in prepared.iter().flatten() {
        let Some(obj) = p.texture else { continue };
        let s = rt.com.surface(obj);
        let fresh = g.textures.get(&obj).is_some_and(|t| t.generation == s.generation && t.keyed == p.keyed && t.w == s.w && t.h == s.h);
        if !fresh {
            let tex = match g.textures.get(&obj) {
                Some(t) if t.w == s.w && t.h == s.h => t.tex,
                _ => g.create_texture(s.w, s.h, false, false),
            };
            uploads.push((tex, s.w, s.h, texture_rgba(rt, obj, p.keyed)));
            g.textures.insert(obj, Texture { tex, w: s.w, h: s.h, generation: s.generation, keyed: p.keyed });
        }
    }
    let mut first_use: HashMap<u32, bool> = HashMap::new(); // storage → first command is a full colour clear
    for c in &commands {
        let (target, full_clear) = match c {
            Command::Clear { target, rects, flags, .. } => {
                let s = rt.com.surface(*target);
                (*target, flags & 1 != 0 && rects.iter().any(|r| r[0] <= 0 && r[1] <= 0 && r[2] >= s.w as i32 && r[3] >= s.h as i32))
            }
            Command::Draw(d) => (d.target, false),
        };
        let bits = g.storage_for(rt, target);
        first_use.entry(bits).or_insert(full_clear);
    }
    for (&bits, &cleared) in &first_use {
        let obj = *rt.com.surfaces.iter().find(|(_, s)| s.bits == bits).unwrap().0;
        let generation = rt.com.surface(obj).generation;
        let st = g.storage.get(&bits).unwrap();
        if !st.gpu_newer && st.synced_generation != generation && !cleared {
            if let Some(px) = storage_rgba(rt, obj) {
                uploads.push((st.tex, st.w, st.h, px));
            }
        }
        g.storage.get_mut(&bits).unwrap().synced_generation = generation;
    }
    let vbytes: &[u8] = bytemuck_slice(&verts);
    let ibytes: &[u8] = bytemuck_slice(&indices);
    let vb = g.ensure_buffer(false, vbytes.len() as u32);
    let ib = g.ensure_buffer(true, ibytes.len() as u32);

    // SAFETY: SDL GPU calls with live objects; transfer buffers released after submission.
    unsafe {
        let cmd = SDL_AcquireGPUCommandBuffer(g.device);
        let copy = SDL_BeginGPUCopyPass(cmd);
        let mut staging = Vec::new();
        for (tex, w, h, px) in &uploads {
            let tb = g.staged(px);
            upload_tex(copy, tb, 0, *tex, *w, *h);
            staging.push(tb);
        }
        if !vbytes.is_empty() {
            let tb = g.staged(vbytes);
            SDL_UploadToGPUBuffer(copy, &SDL_GPUTransferBufferLocation { transfer_buffer: tb, offset: 0 }, &SDL_GPUBufferRegion { buffer: vb, offset: 0, size: vbytes.len() as u32 }, true);
            staging.push(tb);
            let tb = g.staged(ibytes);
            SDL_UploadToGPUBuffer(copy, &SDL_GPUTransferBufferLocation { transfer_buffer: tb, offset: 0 }, &SDL_GPUBufferRegion { buffer: ib, offset: 0, size: ibytes.len() as u32 }, true);
            staging.push(tb);
        }
        SDL_EndGPUCopyPass(copy);

        // 3. Render passes, in command order.
        let mut pass: *mut SDL_GPURenderPass = null_mut();
        let mut pass_target: Option<(u32, Option<u32>)> = None;
        for (c, p) in commands.iter().zip(&prepared) {
            match c {
                Command::Clear { target, zbuffer, rects, flags, color, z } => {
                    let s = rt.com.surface(*target);
                    if !rects.iter().all(|r| r[0] <= 0 && r[1] <= 0 && r[2] >= s.w as i32 && r[3] >= s.h as i32) {
                        fatal(&format!("Clear2 of a partial rectangle {rects:?} is not emulated"));
                    }
                    if !pass.is_null() {
                        SDL_EndGPURenderPass(pass);
                    }
                    let tbits = g.storage_for(rt, *target);
                    let zbits = zbuffer.map(|zb| g.storage_for(rt, zb));
                    let cc = tnl::unpack_color(*color);
                    pass = begin_pass(g, cmd, tbits, zbits, (flags & 1 != 0).then_some([cc[0], cc[1], cc[2], 1.0]), (flags & 2 != 0).then_some(*z));
                    pass_target = Some((tbits, zbits));
                    g.storage.get_mut(&tbits).unwrap().gpu_newer = true;
                }
                Command::Draw(d) => {
                    let p = p.as_ref().unwrap();
                    if p.count == 0 {
                        continue;
                    }
                    let tbits = g.storage_for(rt, d.target);
                    let zbits = d.zbuffer.map(|zb| g.storage_for(rt, zb));
                    if pass.is_null() || pass_target != Some((tbits, zbits)) {
                        if !pass.is_null() {
                            SDL_EndGPURenderPass(pass);
                        }
                        pass = begin_pass(g, cmd, tbits, zbits, None, None);
                        pass_target = Some((tbits, zbits));
                    }
                    g.storage.get_mut(&tbits).unwrap().gpu_newer = true;
                    let pipe = g.pipeline(p.pipeline);
                    SDL_BindGPUGraphicsPipeline(pass, pipe);
                    SDL_SetGPUViewport(pass, &p.viewport);
                    SDL_BindGPUVertexBuffers(pass, 0, &SDL_GPUBufferBinding { buffer: vb, offset: 0 }, 1);
                    SDL_BindGPUIndexBuffer(pass, &SDL_GPUBufferBinding { buffer: ib, offset: 0 }, SDL_GPU_INDEXELEMENTSIZE_32BIT);
                    let tex = p.texture.and_then(|t| g.textures.get(&t)).map(|t| t.tex).unwrap_or(g.white);
                    let samp = g.sampler(p.sampler);
                    SDL_BindGPUFragmentSamplers(pass, 0, &SDL_GPUTextureSamplerBinding { texture: tex, sampler: samp }, 1);
                    SDL_PushGPUFragmentUniformData(cmd, 0, &p.uniforms as *const _ as *const c_void, std::mem::size_of::<StageUniforms>() as u32);
                    SDL_DrawGPUIndexedPrimitives(pass, p.count, 1, p.first_index, p.base_vertex, 0);
                }
            }
        }
        if !pass.is_null() {
            SDL_EndGPURenderPass(pass);
        }
        if !SDL_SubmitGPUCommandBuffer(cmd) {
            sdl_err("SDL_SubmitGPUCommandBuffer");
        }
        for tb in staging {
            SDL_ReleaseGPUTransferBuffer(g.device, tb);
        }
    }
}

unsafe fn begin_pass(g: &Gpu, cmd: *mut SDL_GPUCommandBuffer, target: u32, z: Option<u32>, clear: Option<[f32; 4]>, clear_z: Option<f32>) -> *mut SDL_GPURenderPass {
    let ct = SDL_GPUColorTargetInfo {
        texture: g.storage[&target].tex,
        clear_color: clear.map(|c| sdl3_sys::pixels::SDL_FColor { r: c[0], g: c[1], b: c[2], a: c[3] }).unwrap_or_default(),
        load_op: if clear.is_some() { SDL_GPU_LOADOP_CLEAR } else { SDL_GPU_LOADOP_LOAD },
        store_op: SDL_GPU_STOREOP_STORE,
        ..Default::default()
    };
    let dt = z.map(|zb| SDL_GPUDepthStencilTargetInfo {
        texture: g.storage[&zb].tex,
        clear_depth: clear_z.unwrap_or(1.0),
        load_op: if clear_z.is_some() { SDL_GPU_LOADOP_CLEAR } else { SDL_GPU_LOADOP_LOAD },
        store_op: SDL_GPU_STOREOP_STORE,
        stencil_load_op: SDL_GPU_LOADOP_DONT_CARE,
        stencil_store_op: SDL_GPU_STOREOP_DONT_CARE,
        ..Default::default()
    });
    let pass = SDL_BeginGPURenderPass(cmd, &ct, 1, dt.as_ref().map_or(std::ptr::null(), |d| d as *const _));
    if pass.is_null() {
        sdl_err("SDL_BeginGPURenderPass");
    }
    pass
}

fn bytemuck_slice<T: Copy>(v: &[T]) -> &[u8] {
    // SAFETY: T is plain old data (GpuVertex is repr(C) f32s; u32).
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}

/// Make guest memory current for surface `obj` (readback of a GPU-newer render target).
pub fn sync_for_cpu(rt: &mut Runtime, obj: u32) {
    flush(rt); // pending draws may target this surface
    let bits = rt.com.surface(obj).bits;
    let needs = rt.gpu.as_ref().is_some_and(|g| g.storage.get(&bits).is_some_and(|s| s.gpu_newer && !s.depth));
    if !needs {
        return;
    }
    let g = rt.gpu.take().unwrap();
    let (tex, w, h) = { let s = &g.storage[&bits]; (s.tex, s.w, s.h) };
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    // SAFETY: download into a transfer buffer, wait for the fence, then map it.
    unsafe {
        let info = SDL_GPUTransferBufferCreateInfo { usage: SDL_GPU_TRANSFERBUFFERUSAGE_DOWNLOAD, size: w * h * 4, ..Default::default() };
        let tb = SDL_CreateGPUTransferBuffer(g.device, &info);
        let cmd = SDL_AcquireGPUCommandBuffer(g.device);
        let copy = SDL_BeginGPUCopyPass(cmd);
        let src = SDL_GPUTextureRegion { texture: tex, w, h, d: 1, ..Default::default() };
        let dst = SDL_GPUTextureTransferInfo { transfer_buffer: tb, offset: 0, pixels_per_row: w, rows_per_layer: h };
        SDL_DownloadFromGPUTexture(copy, &src, &dst);
        SDL_EndGPUCopyPass(copy);
        let fence = SDL_SubmitGPUCommandBufferAndAcquireFence(cmd);
        if fence.is_null() {
            sdl_err("SDL_SubmitGPUCommandBufferAndAcquireFence");
        }
        SDL_WaitForGPUFences(g.device, true, &fence, 1);
        SDL_ReleaseGPUFence(g.device, fence);
        let p = SDL_MapGPUTransferBuffer(g.device, tb, false) as *const u8;
        std::ptr::copy_nonoverlapping(p, rgba.as_mut_ptr(), rgba.len());
        SDL_UnmapGPUTransferBuffer(g.device, tb);
        SDL_ReleaseGPUTransferBuffer(g.device, tb);
    }
    let s = rt.com.surface(obj).clone();
    let mem = rt.mem.slice_mut(s.bits, s.size_bytes() as usize);
    s.fmt.from_rgba8(&rgba, s.w, s.h, mem, s.pitch);
    let mut g = g;
    let st = g.storage.get_mut(&bits).unwrap();
    st.gpu_newer = false;
    st.synced_generation = s.generation;
    rt.gpu = Some(g);
}

/// Present guest surface `front` (the visible front buffer) in the window.
pub fn present(rt: &mut Runtime, front: u32) {
    flush(rt);
    if rt.gpu.as_ref().is_some_and(|g| g.window.is_null()) {
        return; // offscreen
    }
    let Some(mut g) = rt.gpu.take() else { return };
    let bits = g.storage_for(rt, front);
    let s = rt.com.surface(front).clone();
    let st = g.storage.get_mut(&bits).unwrap();
    let upload = (!st.gpu_newer && st.synced_generation != s.generation).then(|| storage_rgba(rt, front)).flatten();
    st.synced_generation = s.generation;
    let (tex, w, h) = (st.tex, st.w, st.h);
    // SAFETY: SDL GPU calls with live objects.
    unsafe {
        let cmd = SDL_AcquireGPUCommandBuffer(g.device);
        let mut staging = null_mut();
        if let Some(px) = upload {
            let copy = SDL_BeginGPUCopyPass(cmd);
            staging = g.staged(&px);
            upload_tex(copy, staging, 0, tex, w, h);
            SDL_EndGPUCopyPass(copy);
        }
        let mut sw: *mut SDL_GPUTexture = null_mut();
        let (mut ww, mut wh) = (0u32, 0u32);
        if !SDL_WaitAndAcquireGPUSwapchainTexture(cmd, g.window, &mut sw, &mut ww, &mut wh) {
            sdl_err("SDL_WaitAndAcquireGPUSwapchainTexture");
        }
        if !sw.is_null() && ww > 0 && wh > 0 {
            let scale = (ww as f32 / w as f32).min(wh as f32 / h as f32);
            let (dw, dh) = ((w as f32 * scale) as u32, (h as f32 * scale) as u32);
            let blit = SDL_GPUBlitInfo {
                source: SDL_GPUBlitRegion { texture: tex, w, h, ..Default::default() },
                destination: SDL_GPUBlitRegion { texture: sw, x: (ww - dw) / 2, y: (wh - dh) / 2, w: dw, h: dh, ..Default::default() },
                load_op: SDL_GPU_LOADOP_CLEAR,
                clear_color: sdl3_sys::pixels::SDL_FColor { r: 0.0, g: 0.0, b: 0.0, a: 1.0 },
                filter: SDL_GPU_FILTER_LINEAR,
                ..Default::default()
            };
            SDL_BlitGPUTexture(cmd, &blit);
        }
        SDL_SubmitGPUCommandBuffer(cmd);
        if !staging.is_null() {
            SDL_ReleaseGPUTransferBuffer(g.device, staging);
        }
    }
    rt.gpu = Some(g);
}

/// Forget GPU resources of a destroyed surface.
pub fn surface_destroyed(rt: &mut Runtime, obj: u32, bits: u32) {
    if let Some(g) = rt.gpu.as_mut() {
        // SAFETY: textures created by this device and no longer referenced.
        unsafe {
            if let Some(t) = g.textures.remove(&obj) {
                SDL_ReleaseGPUTexture(g.device, t.tex);
            }
            if let Some(s) = g.storage.remove(&bits) {
                SDL_ReleaseGPUTexture(g.device, s.tex);
            }
        }
    }
}
