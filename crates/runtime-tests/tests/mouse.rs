//! Guest cursor driven by relative host motion, as the original reads it.

use bugatron_runtime::testing::call_import;
use bugatron_runtime::user::{mouse_button_rt, mouse_motion_rt};
use bugatron_runtime::{with_rt, Cpu};
use bugatron_runtime_tests::*;

fn cursor(cpu: &mut Cpu) -> (i32, i32) {
    let p = SCRATCH + 0x100;
    call_import(cpu, "user32.dll", "GetCursorPos", &[p]);
    (r32(p) as i32, r32(p + 4) as i32)
}

/// One frame of the original's gameplay mouse read (0x411f70): offset of the
/// cursor from the screen centre × 0.0625, capped at 2, then SetCursorPos(centre).
fn gameplay_speed(cpu: &mut Cpu, w: i32, h: i32) -> f32 {
    let (x, _) = cursor(cpu);
    let speed = ((x - w / 2).abs() as f32 * 0.0625).min(2.0);
    call_import(cpu, "user32.dll", "SetCursorPos", &[(w / 2) as u32, (h / 2) as u32]);
    speed
}

#[test]
fn gameplay_sensitivity_is_independent_of_resolution() {
    for (w, h) in [(640, 480), (1600, 1200), (2616, 1962)] {
        let mut cpu = setup();
        with_rt(|rt| rt.user.screen = (w, h));
        call_import(&mut cpu, "user32.dll", "SetCursorPos", &[(w / 2) as u32, (h / 2) as u32]);
        // 8 points of host motion in a frame: half speed, at every resolution.
        with_rt(|rt| mouse_motion_rt(rt, 8.0, 3.0));
        assert_eq!(gameplay_speed(&mut cpu, w, h), 0.5, "{w}x{h}");
        // No motion in the next frame: the re-centred cursor reads as stationary.
        assert_eq!(gameplay_speed(&mut cpu, w, h), 0.0);
        // A large flick saturates at the original's cap.
        with_rt(|rt| mouse_motion_rt(rt, -100.0, 0.0));
        assert_eq!(gameplay_speed(&mut cpu, w, h), 2.0);
    }
}

#[test]
fn motion_accumulates_sub_pixel_and_clamps_to_the_screen() {
    let mut cpu = setup();
    with_rt(|rt| rt.user.screen = (640, 480));
    call_import(&mut cpu, "user32.dll", "SetCursorPos", &[100, 100]);
    with_rt(|rt| mouse_motion_rt(rt, 0.5, -0.25));
    assert_eq!(cursor(&mut cpu), (100, 100));
    with_rt(|rt| mouse_motion_rt(rt, 0.5, -0.75));
    assert_eq!(cursor(&mut cpu), (101, 99), "fractions add up");
    with_rt(|rt| mouse_motion_rt(rt, 10_000.0, 10_000.0));
    assert_eq!(cursor(&mut cpu), (639, 479));
    with_rt(|rt| mouse_motion_rt(rt, -10_000.0, -10_000.0));
    assert_eq!(cursor(&mut cpu), (0, 0));
    call_import(&mut cpu, "user32.dll", "SetCursorPos", &[5000, 7]);
    assert_eq!(cursor(&mut cpu), (639, 7), "SetCursorPos is clamped like Windows");
}

#[test]
fn menu_pointer_moves_one_guest_pixel_per_point_and_clicks_where_it_is() {
    let mut cpu = setup();
    with_rt(|rt| rt.user.screen = (2616, 1962));
    call_import(&mut cpu, "user32.dll", "SetCursorPos", &[1308, 981]);
    with_rt(|rt| mouse_motion_rt(rt, -30.0, 12.0));
    let p = cursor(&mut cpu);
    assert_eq!(p, (1278, 993));
    with_rt(|rt| mouse_button_rt(rt, 1, true, p));
    let m = SCRATCH + 0x200;
    let mut last = (0, 0);
    while call_import(&mut cpu, "user32.dll", "PeekMessageA", &[m, 0, 0, 0, 1]) != 0 {
        last = (r32(m + 4), r32(m + 12));
    }
    assert_eq!(last, (0x201, 1278 | (993 << 16)), "WM_LBUTTONDOWN at the guest cursor");
}

#[test]
fn scripted_relative_motion() {
    use bugatron_runtime::input_script::{apply, parse, Input};
    let mut cpu = setup();
    with_rt(|rt| rt.user.screen = (640, 480));
    call_import(&mut cpu, "user32.dll", "SetCursorPos", &[320, 240]);
    let ev = parse("3:rel@-16x4").unwrap();
    assert_eq!(ev, [(3, Input::Rel((-16, 4)))]);
    with_rt(|rt| apply(rt, &ev, 3));
    assert_eq!(cursor(&mut cpu), (304, 244));
}
