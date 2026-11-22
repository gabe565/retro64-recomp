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
fn sleep_zero_is_a_pure_yield_and_qpc_reads_take_one_tick() {
    let mut c = Cadence::new();
    c.sleep(0);
    assert_eq!(c.now_ns(), 0);
    c.sleep(10);
    assert_eq!(c.now_ns(), 10_000_000);
    let a = c.read_qpc(FREQ);
    let b = c.read_qpc(FREQ);
    assert_eq!(b, a + 1, "successive QPC reads differ by exactly one tick");
    assert_eq!(c.ticks(FREQ), b + 1);
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
    // The QPC read before each wait takes a tick, as real loop overhead does.
    assert_eq!(intervals[30], FREQ / 60 - 1, "the game measures one 60 Hz refresh, minus the read");
}

/// The original's per-frame limiter (0x41c180): calibrate, then each frame
/// busy-wait with QPC + Sleep(0) until one measured period after the previous
/// exit, then Flip(DDFLIP_WAIT). Over 20,000 frames (over 5 minutes) every
/// frame must span exactly one refresh: no drift, no dropped frames.
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
    // Calibration as the game does it (median of 60 vblank intervals).
    let mut iv = Vec::new();
    for _ in 0..60 {
        let a = qpc(&mut cpu);
        call_com(&mut cpu, dd4, 22, &[1, 0]);
        iv.push(qpc(&mut cpu) - a);
    }
    iv.sort();
    let period = iv[30];
    let mut prev_exit = qpc(&mut cpu);
    let mut frame_starts = Vec::new();
    let mut spins = 0;
    for frame in 0..20_000 {
        frame_starts.push(qpc(&mut cpu));
        loop {
            let now = qpc(&mut cpu);
            call_import(&mut cpu, "kernel32.dll", "Sleep", &[0]);
            // The first frame has no preceding Flip and waits a whole period.
            if frame > 0 {
                spins += 1;
            }
            if now > prev_exit + period {
                prev_exit = now;
                break;
            }
        }
        assert_eq!(call_com(&mut cpu, front, 11, &[0, 1]), 0);
    }
    for w in frame_starts.windows(2).skip(1) {
        assert_eq!(w[1] - w[0], FREQ / 60, "each frame spans exactly one refresh");
    }
    assert!(spins < 20_000 * 4, "the busy-wait ends after a few polls ({spins})");
    // Message times and timeGetTime read the same clock.
    let ms = call_import(&mut cpu, "winmm.dll", "timeGetTime", &[]) as u64;
    assert_eq!(ms, qpc(&mut cpu) * 1000 / FREQ);
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

/// The frame wait's closed form gives the value and clock that reading in a loop
/// gives (game.rs FrameWait).
#[test]
fn reading_past_a_value_matches_reading_in_a_loop() {
    for freq in [24_000_000u64, 3_579_545, 1_193_182, 1000] {
        for (start_reads, ahead) in [(0u64, 0u64), (1, 1), (5, 399_999), (1234, 400_000), (99, 12_345_678)] {
            let mut looped = Cadence::new();
            let mut closed = Cadence::new();
            for _ in 0..start_reads {
                looped.read_qpc(freq);
                closed.read_qpc(freq);
            }
            let end = looped.ticks(freq) + ahead;
            let mut v = looped.read_qpc(freq);
            let mut reads = 1;
            while v <= end {
                v = looped.read_qpc(freq);
                reads += 1;
            }
            let first = closed.read_qpc(freq);
            let (w, skipped) = if first <= end { closed.read_qpc_past(freq, end) } else { (first, 0) };
            let closed_reads = if first <= end { skipped + 2 } else { 1 };
            assert_eq!((w, closed_reads, closed.ticks(freq)), (v, reads, looped.ticks(freq)), "freq {freq}, start {start_reads}, ahead {ahead}");
        }
    }
}
