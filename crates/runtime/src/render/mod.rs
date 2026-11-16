//! Renderer integration points used by the DirectDraw/Direct3D emulation.
//!
//! The GPU renderer (SDL3 GPU) plugs in here. Until a render target has been
//! drawn by the GPU, guest memory is authoritative and these are no-ops.

pub mod tnl;

use crate::Runtime;

/// Bring `obj`'s guest pixels up to date before CPU access (GPU → guest readback).
pub(crate) fn sync_for_cpu(_rt: &mut Runtime, _obj: u32) {}

/// Called before `front` and `back` exchange their pixel storage in `Flip`.
pub(crate) fn before_flip(rt: &mut Runtime, _front: u32, _back: u32) {
    if crate::trace_enabled("draws") {
        trace_frame(rt);
    }
    // No renderer yet: recorded commands are dropped at each frame.
    rt.com.d3d.commands.clear();
}

/// `EndScene`: recorded commands may be submitted.
pub(crate) fn end_scene(_rt: &mut Runtime) {}

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
    for (k, n) in kinds {
        eprintln!("{n:>5} {k}");
    }
}
