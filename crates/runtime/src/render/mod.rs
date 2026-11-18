//! Renderer integration points used by the DirectDraw/Direct3D emulation.
//!
//! The SDL3 GPU backend (gpu.rs) is created lazily for the game window. In
//! headless runs (tests, `SDL_VIDEODRIVER=dummy`) there is no backend: recorded
//! commands are dropped at each frame and guest memory stays authoritative.

pub mod capture;
pub mod gpu;
pub mod shaders;
pub mod tnl;

use crate::Runtime;

/// The GPU backend, created on first use for the game window.
fn backend(rt: &mut Runtime) -> bool {
    if rt.gpu.is_some() {
        return true;
    }
    if rt.gpu_unavailable {
        return false;
    }
    let window = if rt.offscreen_gpu {
        std::ptr::null_mut()
    } else {
        if rt.user.headless {
            return false;
        }
        let Some(w) = rt.user.window.as_ref().filter(|w| !w.sdl.is_null()) else { return false };
        w.sdl
    };
    match gpu::Gpu::new(window) {
        Some(g) => {
            rt.gpu = Some(g);
            true
        }
        None => {
            eprintln!("[render] no SDL3 GPU device available; running without display");
            rt.gpu_unavailable = true;
            false
        }
    }
}

/// Bring `obj`'s guest pixels up to date before CPU access (GPU → guest readback).
pub(crate) fn sync_for_cpu(rt: &mut Runtime, obj: u32) {
    if (rt.gpu.is_some() || !rt.com.d3d.commands.is_empty()) && backend(rt) {
        gpu::sync_for_cpu(rt, obj);
    }
}

/// Called before `front` and `back` exchange their pixel storage in `Flip`.
pub(crate) fn before_flip(rt: &mut Runtime, _front: u32, _back: u32) {
    if backend(rt) {
        gpu::flush(rt);
    } else {
        rt.com.d3d.commands.clear();
    }
}

/// After `Flip`: show the new front buffer.
pub(crate) fn after_flip(rt: &mut Runtime, front: u32) {
    if backend(rt) && !crate::overlay::hide_loading_frame(rt) {
        gpu::present(rt, front);
    }
    capture::maybe_capture(rt, front);
    if let Some(mut hook) = rt.frame_hook.take() {
        let n = rt.com.flips;
        hook(rt, n);
        rt.frame_hook.get_or_insert(hook);
    }
    if backend(rt) {
        crate::overlay::after_flip(rt, front);
    }
}

/// A surface's pixel storage is going away.
pub(crate) fn surface_destroyed(rt: &mut Runtime, obj: u32, bits: u32) {
    if rt.gpu.is_some() {
        gpu::surface_destroyed(rt, obj, bits);
    }
}

/// Whether a GPU backend is (or can be) active.
pub fn gpu_available(rt: &mut Runtime) -> bool {
    backend(rt)
}

/// `EndScene`: recorded commands may be submitted.
pub(crate) fn end_scene(rt: &mut Runtime) {
    if crate::trace_enabled("draws") {
        trace_frame(rt);
    }
    if backend(rt) {
        gpu::flush(rt);
    }
}

/// Summarise a frame's recorded commands (BUGATRON_TRACE=draws).
fn trace_frame(rt: &Runtime) {
    use crate::ddraw::d3d::{rs, tss, Command};
    use std::collections::BTreeMap;
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    for c in &rt.com.d3d.commands {
        let k = match c {
            Command::Clear { flags, color, rects, .. } => format!("clear flags={flags:#x} color={color:#010x} rects={rects:?}"),
            Command::Draw(d) => {
                let r = &d.render;
                let st = &d.stages[0];
                let tex = d.textures[0].map(|(s, _)| format!("{:?}", rt.com.surfaces.get(&s).map(|s| (s.w, s.h, s.fmt.bpp, s.fmt.a != 0, s.ck_src.is_some()))));
                format!(
                    "draw prim={} fvf={:#x} flags={:#x} idx={} z={}/{}/{} blend={}:{}/{} atest={}:{}/{} cull={} shade={} spec={} ck={} dither={} tmb={} mag={} min={} cop={}/{}/{} aop={}/{}/{} addr={} lit={} mat={} tex={}",
                    d.primitive, d.fvf, d.flags, d.indices.is_some(),
                    r[rs::ZENABLE], r[rs::ZWRITEENABLE], r[rs::ZFUNC],
                    r[rs::ALPHABLENDENABLE], r[rs::SRCBLEND], r[rs::DESTBLEND],
                    r[rs::ALPHATESTENABLE], r[rs::ALPHAFUNC], r[rs::ALPHAREF],
                    r[rs::CULLMODE], r[rs::SHADEMODE], r[rs::SPECULARENABLE], r[rs::COLORKEYENABLE], r[rs::DITHERENABLE], r[rs::TEXTUREMAPBLEND],
                    st[tss::MAGFILTER], st[tss::MINFILTER],
                    st[tss::COLOROP], st[tss::COLORARG1], st[tss::COLORARG2], st[tss::ALPHAOP], st[tss::ALPHAARG1], st[tss::ALPHAARG2],
                    st[tss::ADDRESSU], d.lights.len(), d.material.is_some(), tex.unwrap_or_else(|| "none".into())
                )
            }
        };
        *kinds.entry(k).or_default() += 1;
    }
    eprintln!("=== frame {} ===", rt.com.flips);
    if crate::trace_enabled("worlds") {
        for c in &rt.com.d3d.commands {
            if let Command::Draw(d) = c {
                let n = d.vertices.len() / crate::ddraw::d3d::fvf_size(d.fvf).unwrap_or(32) as usize;
                if n >= 1 {
                    eprintln!("world n={n} {:?}", d.world);
                }
            }
        }
    }
    if let Some(Command::Draw(d)) = rt.com.d3d.commands.iter().find(|c| matches!(c, Command::Draw(_))) {
        eprintln!("viewport {:?}\nprojection {:?}\nview {:?}", d.viewport, d.projection, d.view);
    }
    for (k, n) in kinds {
        eprintln!("{n:>5} {k}");
    }
}
