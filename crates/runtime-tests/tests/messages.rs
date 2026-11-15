//! USER32 window creation, message delivery and input state (headless).

use bugatron_runtime::testing::call_import;
use bugatron_runtime::user::{self, MAIN_HWND};
use bugatron_runtime::Cpu;
use bugatron_runtime_tests::*;

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
const WM_GETMINMAXINFO: u32 = 0x24;
const WM_NCCREATE: u32 = 0x81;
const WM_NCCALCSIZE: u32 = 0x83;
const WM_KEYDOWN: u32 = 0x100;
const WM_KEYUP: u32 = 0x101;
const WM_SYSKEYDOWN: u32 = 0x104;
const WM_SYSCOMMAND: u32 = 0x112;
const WM_LBUTTONDOWN: u32 = 0x201;

/// Messages logged by the recording window procedure: (msg, wParam, lParam).
fn messages() -> Vec<(u32, u32, u32)> {
    log().chunks(3).map(|c| (c[0], c[1], c[2])).collect()
}

fn create(cpu: &mut Cpu) -> u32 {
    // WNDCLASSA with lpfnWndProc and lpszClassName.
    let wc = SCRATCH + 0x100;
    let class = put_str(0, "BugClass");
    w32(wc + 4, HELPER_WNDPROC);
    w32(wc + 36, class);
    assert_ne!(call_import(cpu, "user32.dll", "RegisterClassA", &[wc]), 0);
    let title = put_str(0x40, "Bugatron");
    call_import(cpu, "user32.dll", "CreateWindowExA", &[8, class, title, 0x8000_0000, 0, 0, 1024, 768, 0, 0, 0x40_0000, 0])
}

/// Peek with PM_REMOVE; returns (message, wParam, lParam).
fn peek(cpu: &mut Cpu) -> Option<(u32, u32, u32)> {
    let m = SCRATCH + 0x400;
    (call_import(cpu, "user32.dll", "PeekMessageA", &[m, 0, 0, 0, 1]) != 0).then(|| (r32(m + 4), r32(m + 8), r32(m + 12)))
}

#[test]
fn create_window_sends_the_creation_sequence() {
    let mut cpu = setup();
    let hwnd = create(&mut cpu);
    assert_eq!(hwnd, MAIN_HWND);
    let m: Vec<u32> = messages().iter().map(|m| m.0).collect();
    assert_eq!(m, [WM_GETMINMAXINFO, WM_NCCREATE, WM_NCCALCSIZE, WM_CREATE]);
    // WM_NCCREATE and WM_CREATE carry the same CREATESTRUCTA.
    let msgs = messages();
    assert_eq!(msgs[1].2, msgs[3].2);
}

#[test]
fn show_window_activates_once() {
    let mut cpu = setup();
    create(&mut cpu);
    clear_log();
    assert_eq!(call_import(&mut cpu, "user32.dll", "ShowWindow", &[MAIN_HWND, 1]), 0, "previously hidden");
    let m: Vec<u32> = messages().iter().map(|m| m.0).collect();
    assert_eq!(m, [WM_SHOWWINDOW, WM_ACTIVATEAPP, WM_ACTIVATE, WM_SETFOCUS, WM_SIZE, WM_MOVE]);
    assert_eq!(messages()[4].2, 1024 | (768 << 16), "WM_SIZE lParam = client size");
    clear_log();
    assert_eq!(call_import(&mut cpu, "user32.dll", "ShowWindow", &[MAIN_HWND, 1]), 1, "already visible");
    assert!(messages().is_empty());
}

#[test]
fn posted_messages_are_peeked_and_dispatched_in_order() {
    let mut cpu = setup();
    create(&mut cpu);
    clear_log();
    call_import(&mut cpu, "user32.dll", "PostMessageA", &[MAIN_HWND, 0x400, 7, 2]);
    call_import(&mut cpu, "user32.dll", "PostMessageA", &[MAIN_HWND, 0x401, 8, 3]);
    // PM_NOREMOVE leaves the message queued.
    let m = SCRATCH + 0x400;
    assert_eq!(call_import(&mut cpu, "user32.dll", "PeekMessageA", &[m, 0, 0, 0, 0]), 1);
    assert_eq!(r32(m + 4), 0x400);
    assert_eq!(peek(&mut cpu), Some((0x400, 7, 2)));
    assert_eq!(r32(m), MAIN_HWND);
    call_import(&mut cpu, "user32.dll", "DispatchMessageA", &[m]);
    assert_eq!(peek(&mut cpu), Some((0x401, 8, 3)));
    assert_eq!(peek(&mut cpu), None);
    assert_eq!(messages(), [(0x400, 7, 2)]);
}

#[test]
fn keyboard_input_produces_key_messages_and_async_state() {
    let mut cpu = setup();
    create(&mut cpu);
    // SDL scancode 82 = Up arrow → VK_UP 0x26, extended scan code 0x48.
    user::key_input(82, true, false, false);
    let (msg, vk, lp) = peek(&mut cpu).unwrap();
    assert_eq!((msg, vk), (WM_KEYDOWN, 0x26));
    assert_eq!(lp, 1 | (0x48 << 16) | (1 << 24));
    // Bit 15 = down, bit 0 = pressed since the last query.
    assert_eq!(call_import(&mut cpu, "user32.dll", "GetAsyncKeyState", &[0x26]), 0x8001);
    assert_eq!(call_import(&mut cpu, "user32.dll", "GetAsyncKeyState", &[0x26]), 0x8000);
    user::key_input(82, false, false, false);
    let (msg, _, lp) = peek(&mut cpu).unwrap();
    assert_eq!(msg, WM_KEYUP);
    assert_eq!(lp >> 30, 3, "previous state and transition bits");
    assert_eq!(call_import(&mut cpu, "user32.dll", "GetAsyncKeyState", &[0x26]), 0);
    // Left shift sets both VK_SHIFT and VK_LSHIFT.
    user::key_input(225, true, false, false);
    assert_eq!(call_import(&mut cpu, "user32.dll", "GetAsyncKeyState", &[0x10]) & 0x8000, 0x8000);
    assert_eq!(call_import(&mut cpu, "user32.dll", "GetAsyncKeyState", &[0xa0]) & 0x8000, 0x8000);
    assert_eq!(peek(&mut cpu).unwrap().1, 0x10, "WM_KEYDOWN carries the generic VK_SHIFT");
}

#[test]
fn virtual_key_mapping() {
    assert_eq!(user::vk_from_scancode(4), Some((0x41, 0x1e))); // A
    assert_eq!(user::vk_from_scancode(39), Some((0x30, 0x0b))); // 0
    assert_eq!(user::vk_from_scancode(40), Some((0x0d, 0x1c))); // Return
    assert_eq!(user::vk_from_scancode(41), Some((0x1b, 0x01))); // Escape
    assert_eq!(user::vk_from_scancode(44), Some((0x20, 0x39))); // Space
    assert_eq!(user::vk_from_scancode(58), Some((0x70, 0x3b))); // F1
    assert_eq!(user::vk_from_scancode(80), Some((0x25, 0x14b))); // Left
}

#[test]
fn mouse_buttons_and_cursor() {
    let mut cpu = setup();
    create(&mut cpu);
    user::mouse_button(1, true, (320, 240));
    let (msg, wp, lp) = peek(&mut cpu).unwrap();
    assert_eq!((msg, wp, lp), (WM_LBUTTONDOWN, 1, 320 | (240 << 16)));
    assert_eq!(call_import(&mut cpu, "user32.dll", "GetAsyncKeyState", &[0x01]) & 0x8000, 0x8000);
    let p = SCRATCH + 0x500;
    call_import(&mut cpu, "user32.dll", "GetCursorPos", &[p]);
    assert_eq!((r32(p), r32(p + 4)), (320, 240));
    call_import(&mut cpu, "user32.dll", "SetCursorPos", &[10, 20]);
    call_import(&mut cpu, "user32.dll", "GetCursorPos", &[p]);
    assert_eq!((r32(p), r32(p + 4)), (10, 20));
}

#[test]
fn alt_f4_reaches_wm_close_through_def_window_proc() {
    let mut cpu = setup();
    create(&mut cpu);
    user::key_input(61, true, false, true); // Alt+F4
    let (msg, vk, lp) = peek(&mut cpu).unwrap();
    assert_eq!((msg, vk), (WM_SYSKEYDOWN, 0x73));
    clear_log();
    call_import(&mut cpu, "user32.dll", "DefWindowProcA", &[MAIN_HWND, msg, vk, lp]);
    // DefWindowProc turns Alt+F4 into WM_SYSCOMMAND(SC_CLOSE); WM_CLOSE follows only
    // when the window procedure passes that on to DefWindowProc (as the game does).
    assert_eq!(messages(), [(WM_SYSCOMMAND, 0xf060, 0)]);
    clear_log();
    call_import(&mut cpu, "user32.dll", "DefWindowProcA", &[MAIN_HWND, WM_SYSCOMMAND, 0xf060, 0]);
    assert_eq!(messages().iter().map(|m| m.0).collect::<Vec<_>>(), [WM_CLOSE]);
}

#[test]
fn destroy_and_quit() {
    let mut cpu = setup();
    create(&mut cpu);
    clear_log();
    call_import(&mut cpu, "user32.dll", "DefWindowProcA", &[MAIN_HWND, WM_CLOSE, 0, 0]);
    let m: Vec<u32> = messages().iter().map(|m| m.0).collect();
    assert_eq!(m, [WM_KILLFOCUS, WM_DESTROY]);
    call_import(&mut cpu, "user32.dll", "PostQuitMessage", &[3]);
    assert_eq!(peek(&mut cpu), Some((WM_QUIT, 3, 0)));
    assert_eq!(peek(&mut cpu), None);
}

#[test]
fn system_metrics_follow_the_guest_screen() {
    let mut cpu = setup();
    assert_eq!(call_import(&mut cpu, "user32.dll", "GetSystemMetrics", &[0]), 1024);
    assert_eq!(call_import(&mut cpu, "user32.dll", "GetSystemMetrics", &[1]), 768);
}
