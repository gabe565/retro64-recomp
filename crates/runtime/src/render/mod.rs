//! Renderer integration points used by the DirectDraw/Direct3D emulation.
//!
//! The GPU renderer (SDL3 GPU) plugs in here. Until a render target has been
//! drawn by the GPU, guest memory is authoritative and these are no-ops.

use crate::Runtime;

/// Bring `obj`'s guest pixels up to date before CPU access (GPU → guest readback).
pub(crate) fn sync_for_cpu(_rt: &mut Runtime, _obj: u32) {}

/// Called before `front` and `back` exchange their pixel storage in `Flip`.
pub(crate) fn before_flip(_rt: &mut Runtime, _front: u32, _back: u32) {}
