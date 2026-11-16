//! Host display detection and the guest display-mode policy.
//!
//! The original renders at whatever display mode its driver enumerates and the
//! user picks; its fullscreen projection assumes 4:3 (0x421fd0). The runtime
//! therefore offers, in addition to the period mode list (ddraw::MODES), a 4:3
//! mode sized to the host display, and selects it by default:
//! - the display's size is taken in physical pixels (points × pixel density),
//!   so HiDPI displays get their full resolution;
//! - the guest mode is the largest 4:3 size that fits, with the width a
//!   multiple of 8 (height a multiple of 6), at 32 bits per pixel;
//! - the window presents it letterboxed or pillarboxed, never stretched.
//!
//! `--video-mode` overrides the choice; any W×H can be offered to the game this
//! way (non-4:3 modes stretch, as they did on the original).

use sdl3_sys::video::*;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HostDisplay {
    /// Desktop size in points (logical units).
    pub w_points: u32,
    pub h_points: u32,
    /// Physical pixels per point.
    pub density: f32,
    /// Usable area (excluding menu bar/dock/taskbar) in points.
    pub usable_points: (u32, u32),
}

impl HostDisplay {
    /// Desktop size in physical pixels.
    pub fn pixels(&self) -> (u32, u32) {
        ((self.w_points as f32 * self.density).round() as u32, (self.h_points as f32 * self.density).round() as u32)
    }
}

/// Query the primary display (initialises SDL's video subsystem). None when headless.
pub fn query() -> Option<HostDisplay> {
    // SAFETY: SDL calls on the main thread; pointers checked before use.
    unsafe {
        use sdl3_sys::init::{SDL_Init, SDL_WasInit, SDL_INIT_VIDEO};
        if SDL_WasInit(SDL_INIT_VIDEO) == 0u32 && !SDL_Init(SDL_INIT_VIDEO) {
            return None;
        }
        let id = SDL_GetPrimaryDisplay();
        let mode = SDL_GetDesktopDisplayMode(id);
        if id == 0 || mode.is_null() {
            return None;
        }
        let m = &*mode;
        let mut r = sdl3_sys::rect::SDL_Rect::default();
        let usable = if SDL_GetDisplayUsableBounds(id, &mut r) { (r.w as u32, r.h as u32) } else { (m.w as u32, m.h as u32) };
        Some(HostDisplay {
            w_points: m.w as u32,
            h_points: m.h as u32,
            density: if m.pixel_density > 0.0 { m.pixel_density } else { 1.0 },
            usable_points: usable,
        })
    }
}

/// Largest 4:3 size (width a multiple of 8) within `w`×`h`, optionally capped in height.
pub fn fit_4_3(w: u32, h: u32, max_h: Option<u32>) -> (u32, u32) {
    let mut hh = h.min(w * 3 / 4).min(max_h.unwrap_or(u32::MAX));
    hh -= hh % 6;
    (hh / 3 * 4, hh)
}

/// Height cap for the automatic choice: above it, the per-frame back-buffer
/// readback (renderer.md, performance notes) costs too much for a full 60 Hz.
pub const AUTO_MAX_HEIGHT: u32 = 2160;

/// The guest mode chosen for a host display: (width, height, 32).
pub fn auto_mode(d: &HostDisplay) -> (u32, u32, u32) {
    let (pw, ph) = d.pixels();
    let (w, h) = fit_4_3(pw, ph, Some(AUTO_MAX_HEIGHT));
    (w.max(640), h.max(480), 32)
}

/// Initial window size in points: the largest 4:3 size within 90% of the usable area.
pub fn window_size(d: &HostDisplay) -> (u32, u32) {
    let (uw, uh) = d.usable_points;
    let (w, h) = fit_4_3(uw * 9 / 10, uh * 9 / 10, None);
    (w.max(640), h.max(480))
}

/// Parse a video mode written as "1600x1200x32" or "1600 x 1200 x 32".
pub fn parse_mode(s: &str) -> Result<(u32, u32, u32), String> {
    let parts: Vec<&str> = s.split(['x', 'X']).map(str::trim).collect();
    let [w, h, b] = parts[..] else { return Err(format!("expected WxHxBPP, got {s:?}")) };
    let n = |v: &str| v.parse::<u32>().map_err(|_| format!("bad number {v:?} in {s:?}"));
    let (w, h, b) = (n(w)?, n(h)?, n(b)?);
    if !(b == 16 || b == 32) {
        return Err(format!("{b} bits per pixel: only 16 and 32 are offered"));
    }
    if w < 320 || h < 200 || w > 4096 || h > 4096 {
        return Err(format!("{w}x{h} is outside 320x200..4096x4096"));
    }
    Ok((w, h, b))
}

/// The mode as the game's video dialog lists it ("%ld x %ld x %ld", 0x43b878).
pub fn mode_label((w, h, b): (u32, u32, u32)) -> String {
    format!("{w} x {h} x {b}")
}
