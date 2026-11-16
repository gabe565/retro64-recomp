//! USER32 windows and messages over SDL3.
//!
//! The game has one top-level window whose window procedure (0x407d10) is
//! original code. This module delivers the messages Windows would send to it:
//! creation (`WM_GETMINMAXINFO`, `WM_NCCREATE`, `WM_NCCALCSIZE`, `WM_CREATE`),
//! showing and activation, input translated from SDL events, and the close and
//! destroy sequence. Messages are dispatched by the game's own message loop
//! through `PeekMessageA`/`DispatchMessageA`.
//!
//! Coordinates: the guest screen is the current guest display mode (desktop
//! until `SetDisplayMode`). The host window shows it scaled; mouse positions
//! are mapped back into guest screen coordinates.

use crate::{call_guest_stdcall, fatal, with_rt, Cpu};
use sdl3_sys::{error::*, events::*, init::*, keycode::*, video::*};
use std::collections::{HashMap, VecDeque};
use std::ffi::CString;

pub const MAIN_HWND: u32 = 0x0001_0010;
const ICON_HANDLE: u32 = 0x0003_0001;
const CURSOR_HANDLE: u32 = 0x0003_0002;
const STOCK_OBJECT_BASE: u32 = 0x0004_0000;

const WM_CREATE: u32 = 0x01;
const WM_DESTROY: u32 = 0x02;
const WM_MOVE: u32 = 0x03;
const WM_SIZE: u32 = 0x05;
const WM_ACTIVATE: u32 = 0x06;
const WM_SETFOCUS: u32 = 0x07;
const WM_KILLFOCUS: u32 = 0x08;
const WM_CLOSE: u32 = 0x10;
const WM_QUIT: u32 = 0x12;
const WM_SHOWWINDOW: u32 = 0x18;
const WM_ACTIVATEAPP: u32 = 0x1c;
const WM_SETCURSOR: u32 = 0x20;
const WM_GETMINMAXINFO: u32 = 0x24;
const WM_NCCREATE: u32 = 0x81;
const WM_NCCALCSIZE: u32 = 0x83;
const WM_KEYDOWN: u32 = 0x100;
const WM_KEYUP: u32 = 0x101;
const WM_SYSKEYDOWN: u32 = 0x104;
const WM_SYSKEYUP: u32 = 0x105;
const WM_SYSCOMMAND: u32 = 0x112;
const WM_MOUSEMOVE: u32 = 0x200;
const WM_LBUTTONDOWN: u32 = 0x201;
const WM_LBUTTONUP: u32 = 0x202;
const WM_RBUTTONDOWN: u32 = 0x204;
const WM_RBUTTONUP: u32 = 0x205;
const SC_CLOSE: u32 = 0xf060;
const VK_F4: u32 = 0x73;

#[derive(Clone, Copy, Debug)]
pub struct Msg {
    pub hwnd: u32,
    pub message: u32,
    pub wparam: u32,
    pub lparam: u32,
}

pub struct Window {
    pub wndproc: u32,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub visible: bool,
    pub sdl: *mut SDL_Window,
}

#[derive(Default)]
pub struct User {
    classes: HashMap<String, u32>,
    pub window: Option<Window>,
    queue: VecDeque<Msg>,
    quit: Option<u32>,
    /// Async key state by virtual-key code: (down, pressed since last query).
    pub(crate) keys: Vec<(bool, bool)>,
    /// Guest screen size (current guest display mode).
    pub screen: (i32, i32),
    /// Cursor position in guest screen coordinates.
    cursor: (i32, i32),
    sdl_ready: bool,
    /// No host window or SDL (tests, CI): messages and state only.
    pub headless: bool,
}

impl User {
    pub fn new(screen: (i32, i32)) -> User {
        User { screen, keys: vec![(false, false); 256], ..Default::default() }
    }
}

fn arg(cpu: &Cpu, i: u32) -> u32 {
    with_rt(|rt| rt.mem.r32(cpu.esp + 4 + 4 * i))
}

fn ret(cpu: &mut Cpu, value: u32, nargs: u32) {
    cpu.eax = value;
    cpu.esp += 4 + 4 * nargs;
}

fn sdl_error() -> String {
    // SAFETY: SDL_GetError returns a valid NUL-terminated string.
    unsafe { std::ffi::CStr::from_ptr(SDL_GetError()) }.to_string_lossy().into_owned()
}

/// Initialise SDL video once (respects SDL_VIDEODRIVER, e.g. "dummy" for headless runs).
pub fn ensure_sdl() {
    if with_rt(|rt| rt.user.sdl_ready || rt.user.headless) {
        return;
    }
    // SAFETY: called on the main thread before any other SDL use.
    if !unsafe { SDL_Init(SDL_INIT_VIDEO | SDL_INIT_EVENTS) } {
        fatal(&format!("SDL_Init: {}", sdl_error()));
    }
    with_rt(|rt| rt.user.sdl_ready = true);
}

/// Send `msg` to the window procedure (synchronous, like SendMessage).
pub fn send(cpu: &mut Cpu, hwnd: u32, msg: u32, wp: u32, lp: u32) -> u32 {
    let proc_ = with_rt(|rt| rt.user.window.as_ref().map(|w| w.wndproc)).unwrap_or_else(|| fatal("message to a window that does not exist"));
    if crate::trace_enabled("messages") {
        eprintln!("[msg] {hwnd:#x} {msg:#06x} wp={wp:#x} lp={lp:#x}");
    }
    call_guest_stdcall(cpu, proc_, &[hwnd, msg, wp, lp])
}

/// Queue a message as if it had been posted (also used by tests to inject input).
pub fn post(m: Msg) {
    with_rt(|rt| rt.user.queue.push_back(m));
}

fn mk_lparam(lo: i32, hi: i32) -> u32 {
    (lo as u16 as u32) | ((hi as u16 as u32) << 16)
}

/// SDL scancode (USB HID usage) → Win32 virtual-key code and PS/2 scan code.
pub fn vk_from_scancode(sc: i32) -> Option<(u32, u32)> {
    Some(match sc {
        4..=29 => (0x41 + (sc - 4) as u32, [0x1e, 0x30, 0x2e, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24, 0x25, 0x26, 0x32, 0x31, 0x18, 0x19, 0x10, 0x13, 0x1f, 0x14, 0x16, 0x2f, 0x11, 0x2d, 0x15, 0x2c][(sc - 4) as usize]),
        30..=38 => (0x31 + (sc - 30) as u32, 0x02 + (sc - 30) as u32),
        39 => (0x30, 0x0b),
        40 => (0x0d, 0x1c),
        41 => (0x1b, 0x01),
        42 => (0x08, 0x0e),
        43 => (0x09, 0x0f),
        44 => (0x20, 0x39),
        45 => (0xbd, 0x0c),
        46 => (0xbb, 0x0d),
        58..=67 => (0x70 + (sc - 58) as u32, 0x3b + (sc - 58) as u32),
        68 => (0x7a, 0x57),
        69 => (0x7b, 0x58),
        72 => (0x13, 0x45),
        73 => (0x2d, 0x152),
        74 => (0x24, 0x147),
        75 => (0x21, 0x149),
        76 => (0x2e, 0x153),
        77 => (0x23, 0x14f),
        78 => (0x22, 0x151),
        79 => (0x27, 0x14d),
        80 => (0x25, 0x14b),
        81 => (0x28, 0x150),
        82 => (0x26, 0x148),
        88 => (0x0d, 0x11c),
        89..=97 => (0x61 + (sc - 89) as u32, 0),
        98 => (0x60, 0x52),
        224 | 228 => (0x11, if sc == 224 { 0x1d } else { 0x11d }),
        225 | 229 => (0x10, if sc == 225 { 0x2a } else { 0x36 }),
        226 | 230 => (0x12, if sc == 226 { 0x38 } else { 0x138 }),
        _ => return None,
    })
}

/// Left/right-specific virtual keys for modifier scancodes (GetAsyncKeyState).
fn side_vk(sc: i32) -> Option<u32> {
    Some(match sc { 224 => 0xa2, 228 => 0xa3, 225 => 0xa0, 229 => 0xa1, 226 => 0xa4, 230 => 0xa5, _ => return None })
}

/// Host window position → guest screen coordinates.
fn to_guest(x: f32, y: f32) -> (i32, i32) {
    with_rt(|rt| {
        let (gw, gh) = rt.user.screen;
        let Some(w) = rt.user.window.as_ref().filter(|w| !w.sdl.is_null()) else { return (0, 0) };
        let (mut ww, mut wh) = (0, 0);
        // SAFETY: valid window handle.
        unsafe { SDL_GetWindowSize(w.sdl, &mut ww, &mut wh) };
        if ww <= 0 || wh <= 0 {
            return (0, 0);
        }
        let s = (ww as f32 / gw as f32).min(wh as f32 / gh as f32);
        let (ox, oy) = ((ww as f32 - gw as f32 * s) / 2.0, (wh as f32 - gh as f32 * s) / 2.0);
        ((((x - ox) / s) as i32).clamp(0, gw - 1), (((y - oy) / s) as i32).clamp(0, gh - 1))
    })
}

/// Translate pending SDL events into window messages.
pub fn pump() {
    if !with_rt(|rt| rt.user.sdl_ready && rt.user.window.is_some() && !rt.user.headless) {
        return;
    }
    // SAFETY: SDL_Event is plain data; SDL fills it.
    let mut ev: SDL_Event = unsafe { std::mem::zeroed() };
    while unsafe { SDL_PollEvent(&mut ev) } {
        let ty = unsafe { ev.r#type };
        if crate::trace_enabled("sdl") {
            eprintln!("[sdl] event {ty:#x}");
        }
        if ty == SDL_EVENT_QUIT.0 || ty == SDL_EVENT_WINDOW_CLOSE_REQUESTED.0 {
            post(Msg { hwnd: MAIN_HWND, message: WM_SYSCOMMAND, wparam: SC_CLOSE, lparam: 0 });
        } else if ty == SDL_EVENT_KEY_DOWN.0 || ty == SDL_EVENT_KEY_UP.0 {
            let k = unsafe { ev.key };
            key_input(k.scancode.0, k.down, k.repeat, (k.r#mod.0 & SDL_KMOD_ALT.0) != 0);
        } else if ty == SDL_EVENT_MOUSE_MOTION.0 {
            let m = unsafe { ev.motion };
            mouse_move(to_guest(m.x, m.y));
        } else if ty == SDL_EVENT_MOUSE_BUTTON_DOWN.0 || ty == SDL_EVENT_MOUSE_BUTTON_UP.0 {
            let b = unsafe { ev.button };
            mouse_button(b.button as i32, b.down, to_guest(b.x, b.y));
        }
    }
}

/// A key changed state (SDL scancode). Updates the async key state and queues
/// `WM_KEYDOWN`/`WM_KEYUP`, or the `WM_SYS*` variants for Alt combinations and
/// F10, with Windows' lParam layout (repeat 1, scan code, extended, context, previous, transition).
pub fn key_input(sc: i32, down: bool, repeat: bool, alt: bool) {
    let Some((vk, scan)) = vk_from_scancode(sc) else { return };
    with_rt(|rt| {
        for v in [Some(vk), side_vk(sc)].into_iter().flatten() {
            let e = &mut rt.user.keys[v as usize];
            if down && !e.0 {
                e.1 = true;
            }
            e.0 = down;
        }
    });
    let sys = alt || vk == 0x79 || vk == 0x12;
    let message = match (down, sys) {
        (true, false) => WM_KEYDOWN,
        (false, false) => WM_KEYUP,
        (true, true) => WM_SYSKEYDOWN,
        (false, true) => WM_SYSKEYUP,
    };
    let mut lp = 1 | ((scan & 0xff) << 16) | (((scan >> 8) & 1) << 24);
    if alt {
        lp |= 1 << 29;
    }
    if repeat || !down {
        lp |= 1 << 30;
    }
    if !down {
        lp |= 1 << 31;
    }
    post(Msg { hwnd: MAIN_HWND, message, wparam: vk, lparam: lp });
}

/// The pointer moved to guest screen position `p`.
pub fn mouse_move(p: (i32, i32)) {
    with_rt(|rt| rt.user.cursor = p);
    post(Msg { hwnd: MAIN_HWND, message: WM_MOUSEMOVE, wparam: 0, lparam: mk_lparam(p.0, p.1) });
}

/// Mouse button (1 = left, 3 = right) changed state at guest screen position `p`.
pub fn mouse_button(button: i32, down: bool, p: (i32, i32)) {
    let (vk, message) = match (button, down) {
        (1, true) => (0x01, WM_LBUTTONDOWN),
        (1, false) => (0x01, WM_LBUTTONUP),
        (3, true) => (0x02, WM_RBUTTONDOWN),
        (3, false) => (0x02, WM_RBUTTONUP),
        _ => return,
    };
    with_rt(|rt| {
        rt.user.cursor = p;
        let e = &mut rt.user.keys[vk as usize];
        if down && !e.0 {
            e.1 = true;
        }
        e.0 = down;
    });
    let wparam = if down { vk } else { 0 }; // MK_LBUTTON = 1, MK_RBUTTON = 2
    post(Msg { hwnd: MAIN_HWND, message, wparam, lparam: mk_lparam(p.0, p.1) });
}

/// USER32/GDI32 imports for windows and messages. Returns false if not handled here.
pub(crate) fn import(cpu: &mut Cpu, name: &str) -> bool {
    match name {
        "LoadIconA" => ret(cpu, ICON_HANDLE, 2),
        "LoadCursorA" => ret(cpu, CURSOR_HANDLE, 2),
        "SetCursor" => ret(cpu, 0, 1), // the game passes NULL (hide); the host cursor is managed by the presenter
        "RegisterClassA" => {
            // WNDCLASSA: style, lpfnWndProc, cbClsExtra, cbWndExtra, hInstance, hIcon, hCursor, hbrBackground, lpszMenuName, lpszClassName
            let wc = arg(cpu, 0);
            let (proc_, name) = with_rt(|rt| (rt.mem.r32(wc + 4), rt.mem.cstr(rt.mem.r32(wc + 36))));
            with_rt(|rt| rt.user.classes.insert(name.to_ascii_lowercase(), proc_));
            ret(cpu, 0xc001, 1);
        }
        "CreateWindowExA" => create_window(cpu),
        "ShowWindow" => {
            let (hwnd, cmd) = (arg(cpu, 0), arg(cpu, 1));
            if hwnd != MAIN_HWND { fatal(&format!("ShowWindow on unknown window {hwnd:#x}")); }
            let was = with_rt(|rt| rt.user.window.as_ref().unwrap().visible);
            if !was && cmd != 0 {
                with_rt(|rt| {
                    let w = rt.user.window.as_mut().unwrap();
                    w.visible = true;
                    if !w.sdl.is_null() {
                        // SAFETY: valid window.
                        unsafe { SDL_ShowWindow(w.sdl) };
                    }
                });
                // The activation sequence Windows sends to a newly shown top-level window.
                let (w, h, x, y) = with_rt(|rt| { let w = rt.user.window.as_ref().unwrap(); (w.w, w.h, w.x, w.y) });
                send(cpu, hwnd, WM_SHOWWINDOW, 1, 0);
                send(cpu, hwnd, WM_ACTIVATEAPP, 1, 0);
                send(cpu, hwnd, WM_ACTIVATE, 1, 0);
                send(cpu, hwnd, WM_SETFOCUS, 0, 0);
                send(cpu, hwnd, WM_SIZE, 0, mk_lparam(w, h));
                send(cpu, hwnd, WM_MOVE, 0, mk_lparam(x, y));
            }
            ret(cpu, was as u32, 2);
        }
        "UpdateWindow" => ret(cpu, 1, 1), // no WM_PAINT handling in the window procedure
        "SetFocus" => ret(cpu, MAIN_HWND, 1),
        "GetSystemMetrics" => {
            let i = arg(cpu, 0);
            let (w, h) = with_rt(|rt| rt.user.screen);
            let v = match i { 0 => w, 1 => h, _ => fatal(&format!("GetSystemMetrics({i}) not emulated")) };
            ret(cpu, v as u32, 1);
        }
        "GetClientRect" => {
            let (hwnd, r) = (arg(cpu, 0), arg(cpu, 1));
            if hwnd != MAIN_HWND { fatal("GetClientRect on unknown window"); }
            with_rt(|rt| {
                let w = rt.user.window.as_ref().unwrap();
                for (i, v) in [0, 0, w.w, w.h].into_iter().enumerate() { rt.mem.w32(r + 4 * i as u32, v as u32); }
            });
            ret(cpu, 1, 2);
        }
        "ClientToScreen" => {
            let (_, p) = (arg(cpu, 0), arg(cpu, 1));
            with_rt(|rt| {
                let w = rt.user.window.as_ref().unwrap();
                let (x, y) = (rt.mem.r32(p) as i32 + w.x, rt.mem.r32(p + 4) as i32 + w.y);
                rt.mem.w32(p, x as u32);
                rt.mem.w32(p + 4, y as u32);
            });
            ret(cpu, 1, 2);
        }
        "GetStockObject" => { let i = arg(cpu, 0); ret(cpu, STOCK_OBJECT_BASE + i, 1) }
        "PeekMessageA" => {
            let (pmsg, hwnd, lo, hi, remove) = (arg(cpu, 0), arg(cpu, 1), arg(cpu, 2), arg(cpu, 3), arg(cpu, 4));
            if hwnd != 0 || lo != 0 || hi != 0 { fatal("PeekMessageA with a filter is not emulated"); }
            pump();
            let m = with_rt(|rt| {
                let m = rt.user.queue.front().copied().or_else(|| rt.user.quit.map(|c| Msg { hwnd: 0, message: WM_QUIT, wparam: c, lparam: 0 }));
                if m.is_some() && remove & 1 != 0 {
                    if rt.user.queue.pop_front().is_none() { rt.user.quit = None; }
                }
                m
            });
            match m {
                Some(m) => {
                    with_rt(|rt| {
                        let t = rt.clock_ms();
                        let (cx, cy) = rt.user.cursor;
                        for (i, v) in [m.hwnd, m.message, m.wparam, m.lparam, t, cx as u32, cy as u32].into_iter().enumerate() {
                            rt.mem.w32(pmsg + 4 * i as u32, v);
                        }
                    });
                    ret(cpu, 1, 5);
                }
                None => ret(cpu, 0, 5),
            }
        }
        "TranslateMessage" => ret(cpu, 0, 1), // WM_CHAR is not used by the window procedure
        "DispatchMessageA" => {
            let p = arg(cpu, 0);
            let (h, m, wp, lp) = with_rt(|rt| (rt.mem.r32(p), rt.mem.r32(p + 4), rt.mem.r32(p + 8), rt.mem.r32(p + 12)));
            let r = if h == MAIN_HWND { send(cpu, h, m, wp, lp) } else { 0 };
            ret(cpu, r, 1);
        }
        "SendMessageA" => {
            let (h, m, wp, lp) = (arg(cpu, 0), arg(cpu, 1), arg(cpu, 2), arg(cpu, 3));
            let r = send(cpu, h, m, wp, lp);
            ret(cpu, r, 4);
        }
        "PostMessageA" => {
            let (h, m, wp, lp) = (arg(cpu, 0), arg(cpu, 1), arg(cpu, 2), arg(cpu, 3));
            post(Msg { hwnd: h, message: m, wparam: wp, lparam: lp });
            ret(cpu, 1, 4);
        }
        "PostQuitMessage" => { let c = arg(cpu, 0); with_rt(|rt| rt.user.quit = Some(c)); ret(cpu, 0, 1) }
        "DefWindowProcA" => {
            let (h, m, wp, lp) = (arg(cpu, 0), arg(cpu, 1), arg(cpu, 2), arg(cpu, 3));
            let r = match m {
                WM_NCCREATE => 1,
                WM_CLOSE => { destroy(cpu, h); 0 }
                WM_SYSCOMMAND if wp & 0xfff0 == SC_CLOSE => { send(cpu, h, WM_CLOSE, 0, 0); 0 }
                WM_SYSKEYDOWN if wp == VK_F4 && lp & (1 << 29) != 0 => { send(cpu, h, WM_SYSCOMMAND, SC_CLOSE, 0); 0 }
                WM_SETCURSOR => 0,
                _ => 0,
            };
            ret(cpu, r, 4);
        }
        "DestroyWindow" => { let h = arg(cpu, 0); destroy(cpu, h); ret(cpu, 1, 1) }
        "GetAsyncKeyState" => {
            let vk = arg(cpu, 0) as usize & 0xff;
            pump();
            let v = with_rt(|rt| {
                let e = &mut rt.user.keys[vk];
                let r = if e.0 { 0x8000 } else { 0 } | e.1 as u32;
                e.1 = false;
                r
            });
            ret(cpu, v, 1);
        }
        "GetCursorPos" => {
            let p = arg(cpu, 0);
            with_rt(|rt| { let (x, y) = rt.user.cursor; rt.mem.w32(p, x as u32); rt.mem.w32(p + 4, y as u32); });
            ret(cpu, 1, 1);
        }
        "SetCursorPos" => {
            // Moves the guest cursor only; the host pointer is not warped.
            let (x, y) = (arg(cpu, 0) as i32, arg(cpu, 1) as i32);
            with_rt(|rt| rt.user.cursor = (x, y));
            ret(cpu, 1, 2);
        }
        _ => return false,
    }
    true
}

fn create_window(cpu: &mut Cpu) {
    // (dwExStyle, lpClassName, lpWindowName, dwStyle, x, y, w, h, parent, menu, hInstance, lpParam)
    let a: Vec<u32> = (0..12).map(|i| arg(cpu, i)).collect();
    let (class, title) = with_rt(|rt| (rt.mem.cstr(a[1]), rt.mem.cstr(a[2])));
    let proc_ = with_rt(|rt| rt.user.classes.get(&class.to_ascii_lowercase()).copied())
        .unwrap_or_else(|| fatal(&format!("CreateWindowExA: class {class:?} not registered")));
    if with_rt(|rt| rt.user.window.is_some()) {
        fatal("CreateWindowExA: only one top-level window is emulated");
    }
    let cw = |v: u32| if v == 0x8000_0000 { None } else { Some(v as i32) }; // CW_USEDEFAULT
    let (x, y) = (cw(a[4]).unwrap_or(0), cw(a[5]).unwrap_or(0));
    let (w, h) = (cw(a[6]).unwrap_or(640), cw(a[7]).unwrap_or(480));
    ensure_sdl();
    let sdl = if with_rt(|rt| rt.user.headless) {
        std::ptr::null_mut()
    } else {
        let t = CString::new(title.clone()).unwrap_or_default();
        // Host window: hidden until ShowWindow; the presenter scales the guest screen into it.
        // SAFETY: SDL is initialised; title is NUL-terminated.
        let sdl = unsafe { SDL_CreateWindow(t.as_ptr(), 1280, 960, SDL_WINDOW_HIDDEN | SDL_WINDOW_RESIZABLE | SDL_WINDOW_HIGH_PIXEL_DENSITY) };
        if sdl.is_null() {
            fatal(&format!("SDL_CreateWindow: {}", sdl_error()));
        }
        sdl
    };
    with_rt(|rt| rt.user.window = Some(Window { wndproc: proc_, x, y, w, h, visible: false, sdl }));
    let hwnd = MAIN_HWND;
    // CREATESTRUCTA and MINMAXINFO in guest memory.
    let (cs, mmi) = with_rt(|rt| {
        let cs = rt.heap_alloc(48, true);
        for (i, v) in [a[11], a[10], a[9], a[8], h as u32, w as u32, y as u32, x as u32, a[3], a[2], a[1], a[0]].into_iter().enumerate() {
            rt.mem.w32(cs + 4 * i as u32, v);
        }
        let mmi = rt.heap_alloc(40, true);
        let (sw, sh) = rt.user.screen;
        for (i, v) in [0, 0, sw, sh, 0, 0, 112, 27, sw + 8, sh + 8].into_iter().enumerate() {
            rt.mem.w32(mmi + 4 * i as u32, v as u32);
        }
        (cs, mmi)
    });
    send(cpu, hwnd, WM_GETMINMAXINFO, 0, mmi);
    if send(cpu, hwnd, WM_NCCREATE, 0, cs) == 0 {
        fatal("window procedure rejected WM_NCCREATE");
    }
    let rect = with_rt(|rt| {
        let r = rt.heap_alloc(16, true);
        for (i, v) in [x, y, x + w, y + h].into_iter().enumerate() { rt.mem.w32(r + 4 * i as u32, v as u32); }
        r
    });
    send(cpu, hwnd, WM_NCCALCSIZE, 0, rect);
    if send(cpu, hwnd, WM_CREATE, 0, cs) == 0xffff_ffff {
        fatal("window procedure failed WM_CREATE");
    }
    with_rt(|rt| { rt.heap.free(cs); rt.heap.free(mmi); rt.heap.free(rect); });
    ret(cpu, hwnd, 12);
}

fn destroy(cpu: &mut Cpu, hwnd: u32) {
    if hwnd != MAIN_HWND || with_rt(|rt| rt.user.window.is_none()) {
        return;
    }
    send(cpu, hwnd, WM_KILLFOCUS, 0, 0);
    send(cpu, hwnd, WM_DESTROY, 0, 0);
    with_rt(|rt| {
        if let Some(w) = rt.user.window.take() {
            if !w.sdl.is_null() {
                // SAFETY: valid window, destroyed once.
                unsafe { SDL_DestroyWindow(w.sdl) };
            }
        }
    });
}
