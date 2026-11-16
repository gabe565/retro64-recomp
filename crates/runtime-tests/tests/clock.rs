//! The simulation clock the game observes: refresh-boundary waits at 60 Hz.

use bugatron_runtime::clock::{Cadence, STALL_READS};
use bugatron_runtime::testing::{call_com, call_import};
use bugatron_runtime::with_rt;
use bugatron_runtime_tests::*;

const FREQ: u64 = 24_000_000;

fn qpc(cpu: &mut bugatron_runtime::Cpu) -> u64 {
    let p = SCRATCH + 0x100;
    assert_eq!(call_import(cpu, "kernel32.dll", "QueryPerformanceCounter", &[p]), 1);
    r32(p) as u64 | ((r32(p + 4) as u64) << 32)
}

fn install_cadence() {
    with_rt(|rt| {
        rt.clock.cadence = Some(Cadence::new());
        rt.clock.qpc_frequency = FREQ;
        rt.clock.pace = false;
    });
}

#[test]
fn vblanks_land_exactly_on_refresh_boundaries() {
    let mut c = Cadence::new();
    for _ in 0..60 {
        c.vblank();
    }
    assert_eq!(c.now_ns(), 1_000_000_000, "60 vblanks are exactly one second");
    assert_eq!(c.ticks(FREQ), FREQ);
    // A vblank wait from mid-refresh goes to the next boundary, not a full period later.
    c.sleep(3);
    c.vblank();
    assert_eq!(c.ticks(60), 61);
    // No drift after an hour of refreshes.
    let mut c = Cadence::new();
    for _ in 0..3600 * 60 {
        c.vblank();
    }
    assert_eq!(c.ticks(FREQ), 3600 * FREQ);
    // The game's 1/1024-second ticks per refresh: 17 or 18, summing to 1024 per second.
    let mut c = Cadence::new();
    let (mut prev, mut sum) = (0, 0);
    for _ in 0..60 {
        c.vblank();
        let t = c.ticks(1024);
        assert!(t - prev == 17 || t - prev == 18);
        sum += t - prev;
        prev = t;
    }
    assert_eq!(sum, 1024);
}

#[test]
fn sleep_zero_is_a_short_yield() {
    let mut c = Cadence::new();
    c.sleep(0);
    assert_eq!(c.now_ns(), 1_000);
    c.sleep(10);
    assert_eq!(c.now_ns(), 10_001_000);
}

/// The original's timer calibration (0x41c180): QPC around each of 60
/// WaitForVerticalBlank calls; the median interval is the refresh period.
#[test]
fn timer_calibration_measures_one_refresh() {
    let mut cpu = setup();
    install_cadence();
    let dd4 = dd4(&mut cpu);
    let mut intervals = Vec::new();
    for _ in 0..60 {
        let a = qpc(&mut cpu);
        assert_eq!(call_com(&mut cpu, dd4, 22, &[1, 0]), 0);
        intervals.push(qpc(&mut cpu) - a);
    }
    intervals.sort();
    assert_eq!(intervals[30], FREQ / 60, "the game sets its timer to exactly one 60 Hz refresh");
}

/// The original's per-frame limiter (0x41c180): busy-wait with QPC + Sleep(0)
/// until one period after its previous exit, then Flip(DDFLIP_WAIT).
#[test]
fn limiter_and_flip_keep_one_frame_per_refresh() {
    let mut cpu = setup();
    install_cadence();
    let dd4 = dd4(&mut cpu);
    call_com(&mut cpu, dd4, 20, &[0x0001_0010, 0x13]);
    call_com(&mut cpu, dd4, 21, &[640, 480, 32, 0, 0]);
    let d = surface_desc(0x1000);
    w32(d + 4, 0x21);
    w32(d + 0x14, 1);
    w32(d + 0x68, 0x2218);
    let (_, front) = create_surface(&mut cpu, dd4, d);
    let period = FREQ / 60;
    let mut prev_exit = qpc(&mut cpu);
    let mut frame_starts = Vec::new();
    for _ in 0..120 {
        frame_starts.push(qpc(&mut cpu));
        loop {
            let now = qpc(&mut cpu);
            call_import(&mut cpu, "kernel32.dll", "Sleep", &[0]);
            if now > prev_exit + period {
                prev_exit = now;
                break;
            }
        }
        assert_eq!(call_com(&mut cpu, front, 11, &[0, 1]), 0);
    }
    for w in frame_starts.windows(2).skip(1) {
        assert_eq!(w[1] - w[0], period, "each frame spans exactly one refresh");
    }
    // Message times and timeGetTime read the same clock.
    let ms = call_import(&mut cpu, "winmm.dll", "timeGetTime", &[]);
    assert_eq!(ms as u64, qpc(&mut cpu) * 1000 / FREQ);
}

#[test]
fn busy_wait_without_sleep_falls_back_to_host_time() {
    let mut c = Cadence::new();
    for _ in 0..STALL_READS {
        c.read_ticks(1_000_000_000);
    }
    let t0 = c.now_ns();
    std::thread::sleep(std::time::Duration::from_millis(5));
    c.read_ticks(1_000_000_000);
    assert!(c.now_ns() >= t0 + 5_000_000, "a stalled busy-wait sees time pass");
    c.vblank();
    assert_eq!(c.ticks(600) % 10, 0, "the next wait returns to a refresh boundary");
}
