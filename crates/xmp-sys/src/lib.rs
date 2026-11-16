//! Minimal bindings to the vendored libxmp-lite and a safe module player.

use std::os::raw::{c_int, c_long, c_void};

type XmpContext = *mut std::os::raw::c_char;

extern "C" {
    fn xmp_create_context() -> XmpContext;
    fn xmp_free_context(ctx: XmpContext);
    fn xmp_load_module_from_memory(ctx: XmpContext, mem: *const c_void, size: c_long) -> c_int;
    fn xmp_release_module(ctx: XmpContext);
    fn xmp_start_player(ctx: XmpContext, rate: c_int, format: c_int) -> c_int;
    fn xmp_play_buffer(ctx: XmpContext, buf: *mut c_void, size: c_int, loops: c_int) -> c_int;
    fn xmp_end_player(ctx: XmpContext);
    fn xmp_restart_module(ctx: XmpContext);
    fn xmp_set_player(ctx: XmpContext, param: c_int, value: c_int) -> c_int;
}

pub const XMP_PLAYER_INTERP: c_int = 2;
pub const XMP_INTERP_LINEAR: c_int = 1;

/// A loaded module ready to render 16-bit stereo at a fixed rate, looping forever.
pub struct Player {
    ctx: XmpContext,
    started: bool,
}

// The context is only used through &mut self; moving it between threads is fine.
unsafe impl Send for Player {}

impl Player {
    /// Load a module (XM, MOD, S3M or IT) from memory and start it at `rate` Hz.
    pub fn load(data: &[u8], rate: u32) -> Result<Player, String> {
        // SAFETY: plain libxmp calls; `data` outlives the load (libxmp copies it).
        unsafe {
            let ctx = xmp_create_context();
            if ctx.is_null() {
                return Err("xmp_create_context failed".into());
            }
            let r = xmp_load_module_from_memory(ctx, data.as_ptr() as *const c_void, data.len() as c_long);
            if r != 0 {
                xmp_free_context(ctx);
                return Err(format!("libxmp could not load the module ({r})"));
            }
            let mut p = Player { ctx, started: false };
            if xmp_start_player(ctx, rate as c_int, 0) != 0 {
                return Err("xmp_start_player failed".into());
            }
            xmp_set_player(ctx, XMP_PLAYER_INTERP, XMP_INTERP_LINEAR);
            p.started = true;
            Ok(p)
        }
    }

    /// Render interleaved 16-bit stereo frames into `out` (len must be even).
    pub fn render(&mut self, out: &mut [i16]) {
        // SAFETY: `out` is a valid buffer of the given byte size; loops = 0 is infinite.
        let r = unsafe { xmp_play_buffer(self.ctx, out.as_mut_ptr() as *mut c_void, (out.len() * 2) as c_int, 0) };
        if r != 0 {
            out.fill(0);
        }
    }

    /// Restart from the first pattern.
    pub fn restart(&mut self) {
        // SAFETY: valid, started context.
        unsafe { xmp_restart_module(self.ctx) };
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // SAFETY: context created by `load`, released once.
        unsafe {
            if self.started {
                xmp_end_player(self.ctx);
            }
            xmp_release_module(self.ctx);
            xmp_free_context(self.ctx);
        }
    }
}
