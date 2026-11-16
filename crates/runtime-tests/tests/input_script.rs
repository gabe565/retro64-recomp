//! Scripted input: parsing and delivery through the message path.

use bugatron_runtime::input_script::{apply, parse, scancode, Input};
use bugatron_runtime::testing::call_import;
use bugatron_runtime::with_rt;
use bugatron_runtime_tests::*;

#[test]
fn parses_taps_holds_releases_and_mouse() {
    let ev = parse("600:F1, 700:left+,760:LEFT-,800:click@70x120,900:move@1x2").unwrap();
    assert_eq!(ev, [
        (600, Input::Key(58, true)),
        (602, Input::Key(58, false)),
        (700, Input::Key(80, true)),
        (760, Input::Key(80, false)),
        (800, Input::Move((70, 120))),
        (800, Input::Button(true, (70, 120))),
        (802, Input::Button(false, (70, 120))),
        (900, Input::Move((1, 2))),
    ]);
    assert_eq!(scancode("ctrl"), Ok(224));
    assert_eq!(scancode("0"), Ok(39));
    assert_eq!(scancode("9"), Ok(38));
    assert!(parse("600:NOPE").is_err());
    assert!(parse("abc:F1").is_err());
    assert!(parse("600:click@70").is_err());
}

#[test]
fn delivery_reaches_messages_and_async_key_state() {
    let mut cpu = setup();
    let ev = parse("5:UP+,5:click@10x20").unwrap();
    with_rt(|rt| apply(rt, &ev, 4));
    assert_eq!(call_import(&mut cpu, "user32.dll", "GetAsyncKeyState", &[0x26]), 0, "nothing before flip 5");
    with_rt(|rt| apply(rt, &ev, 5));
    assert_eq!(call_import(&mut cpu, "user32.dll", "GetAsyncKeyState", &[0x26]), 0x8001);
    assert_eq!(call_import(&mut cpu, "user32.dll", "GetAsyncKeyState", &[0x01]) & 0x8000, 0x8000);
    let p = SCRATCH + 0x100;
    call_import(&mut cpu, "user32.dll", "GetCursorPos", &[p]);
    assert_eq!((r32(p), r32(p + 4)), (10, 20));
    let m = SCRATCH + 0x200;
    let mut msgs = Vec::new();
    while call_import(&mut cpu, "user32.dll", "PeekMessageA", &[m, 0, 0, 0, 1]) != 0 {
        msgs.push(r32(m + 4));
    }
    assert_eq!(msgs, [0x100, 0x200, 0x201], "WM_KEYDOWN, WM_MOUSEMOVE, WM_LBUTTONDOWN");
}

#[test]
fn quit_posts_the_close_command_like_the_host_close_button() {
    let mut cpu = setup();
    let ev = parse("7:QUIT").unwrap();
    assert_eq!(ev, [(7, Input::Quit)]);
    with_rt(|rt| apply(rt, &ev, 7));
    let m = SCRATCH + 0x200;
    assert_ne!(call_import(&mut cpu, "user32.dll", "PeekMessageA", &[m, 0, 0, 0, 1]), 0);
    assert_eq!((r32(m + 4), r32(m + 8)), (0x112, 0xf060), "WM_SYSCOMMAND(SC_CLOSE)");
}
