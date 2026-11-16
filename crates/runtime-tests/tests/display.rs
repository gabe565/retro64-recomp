//! Host display → guest display-mode policy, and extra modes offered to the game.

use bugatron_runtime::ddraw::MODES;
use bugatron_runtime::display::{auto_mode, fit_4_3, mode_label, parse_mode, window_size, HostDisplay, AUTO_MAX_HEIGHT};
use bugatron_runtime::testing::call_com;
use bugatron_runtime::with_rt;
use bugatron_runtime_tests::*;

fn host(w: u32, h: u32, density: f32) -> HostDisplay {
    HostDisplay { w_points: w, h_points: h, density, usable_points: (w, h - 40) }
}

#[test]
fn largest_4_3_fit_uses_physical_pixels() {
    // MacBook Pro 14": 1512×982 points at 2× → 3024×1964 pixels.
    let mbp = host(1512, 982, 2.0);
    assert_eq!(mbp.pixels(), (3024, 1964));
    assert_eq!(auto_mode(&mbp), (2616, 1962, 32), "not the 1512×982 logical size");
    assert_eq!(auto_mode(&host(1920, 1080, 1.0)), (1440, 1080, 32));
    assert_eq!(auto_mode(&host(1600, 1200, 1.0)), (1600, 1200, 32));
    assert_eq!(auto_mode(&host(1280, 1024, 1.0)), (1280, 960, 32), "5:4 is pillarboxed, not stretched");
    assert_eq!(auto_mode(&host(1280, 800, 1.0)), (1064, 798, 32));
    // 4K and 5K displays are capped (per-frame readback cost); the aspect stays 4:3.
    assert_eq!(auto_mode(&host(1920, 1080, 2.0)), (2880, 2160, 32));
    assert_eq!(auto_mode(&host(2560, 1440, 2.0)), (2880, AUTO_MAX_HEIGHT, 32));
    // Never below the original's 640×480.
    assert_eq!(auto_mode(&host(800, 400, 1.0)), (640, 480, 32));
    for d in [host(1512, 982, 2.0), host(1366, 768, 1.25), host(3440, 1440, 1.0)] {
        let (w, h, _) = auto_mode(&d);
        assert_eq!(w * 3, h * 4, "4:3 for {d:?}");
        assert_eq!(w % 8, 0);
        assert!(w <= d.pixels().0.max(640) && h <= d.pixels().1.max(480));
    }
}

#[test]
fn fit_is_exact_and_capped() {
    assert_eq!(fit_4_3(640, 480, None), (640, 480));
    assert_eq!(fit_4_3(3024, 1964, None), (2616, 1962));
    assert_eq!(fit_4_3(3024, 1964, Some(1200)), (1600, 1200));
    assert_eq!(fit_4_3(1000, 2000, None), (1000, 750), "width-limited");
}

#[test]
fn window_is_4_3_within_the_usable_area() {
    let (w, h) = window_size(&host(1512, 982, 2.0));
    assert_eq!(w * 3, h * 4);
    assert!(w <= 1512 && h <= 942 * 9 / 10 + 1);
}

#[test]
fn video_mode_strings_parse_with_or_without_spaces() {
    assert_eq!(parse_mode("1600x1200x32"), Ok((1600, 1200, 32)));
    assert_eq!(parse_mode("1600 x 1200 x 32"), Ok((1600, 1200, 32)));
    assert_eq!(parse_mode("800X600X16"), Ok((800, 600, 16)));
    assert!(parse_mode("1600x1200").is_err());
    assert!(parse_mode("1600x1200x24").is_err(), "only 16 and 32 bits are offered");
    assert!(parse_mode("99999x1x32").is_err());
    assert_eq!(mode_label((2616, 1962, 32)), "2616 x 1962 x 32", "as the game's dialog lists modes");
}

#[test]
fn extra_modes_are_enumerated_after_the_period_list_and_accepted() {
    let mut cpu = setup();
    with_rt(|rt| rt.com.offer_mode(2616, 1962));
    with_rt(|rt| rt.com.offer_mode(1600, 1200)); // already a period mode: not duplicated
    let dd4 = dd4(&mut cpu);
    call_com(&mut cpu, dd4, 8, &[0, 0, 0, HELPER_MODE_COPIER]);
    let modes: Vec<(u32, u32, u32)> = log_bytes().chunks(0x7c).map(|m| (dw(m, 3), dw(m, 2), dw(m, 21))).collect();
    assert_eq!(&modes[..MODES.len()], MODES, "period mode indices (and prefs files) stay stable");
    assert_eq!(&modes[MODES.len()..], [(2616, 1962, 32), (2616, 1962, 16)]);
    assert_eq!(call_com(&mut cpu, dd4, 21, &[2616, 1962, 32, 0, 0]), 0);
    assert_eq!(with_rt(|rt| rt.user.screen), (2616, 1962));
}
