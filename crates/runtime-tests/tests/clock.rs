//! The simulation clock the game observes: a fixed 60 Hz cadence per Flip.

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
fn cadence_is_exact_and_drift_free() {
    let mut c = Cadence::new();
    assert_eq!(c.now_ns(), 0);
    for _ in 0..60 {
        c.frame();
    }
    assert_eq!(c.now_ns(), 1_000_000_000, "60 frames are exactly one second");
    for _ in 0..3600 * 60 - 60 {
        c.frame();
    }
    assert_eq!(c.now_ns(), 3600 * 1_000_000_000, "no drift after an hour");
    c.sleep(5);
    assert_eq!(c.now_ns(), 3600 * 1_000_000_000 + 5_000_000);
    assert_eq!(c.ticks(24_000_000), 3600 * 24_000_000 + 120_000);
    // 1/60 s in the game's 1/1024-second ticks: deltas of 17 or 18 summing to 1024 per second.
    let mut c = Cadence::new();
    let mut prev = 0u64;
    let mut sum = 0;
    for _ in 0..60 {
        c.frame();
        let t = c.ticks(1024);
        let d = t - prev;
        assert!(d == 17 || d == 18, "delta {d}");
        sum += d;
        prev = t;
    }
    assert_eq!(sum, 1024);
}

#[test]
fn guest_time_is_constant_within_a_frame_and_advances_per_flip() {
    let mut cpu = setup();
    install_cadence();
    let freq = SCRATCH + 0x200;
    call_import(&mut cpu, "kernel32.dll", "QueryPerformanceFrequency", &[freq]);
    assert_eq!(r32(freq) as u64, FREQ);
    let t0 = qpc(&mut cpu);
    let ms0 = call_import(&mut cpu, "winmm.dll", "timeGetTime", &[]);
    assert_eq!(qpc(&mut cpu), t0, "no time passes without a frame boundary");
    // A Flip is the frame boundary.
    let dd4 = dd4(&mut cpu);
    call_com(&mut cpu, dd4, 20, &[0x0001_0010, 0x13]);
    call_com(&mut cpu, dd4, 21, &[640, 480, 32, 0, 0]);
    let d = surface_desc(0x1000);
    w32(d + 4, 0x21);
    w32(d + 0x14, 1);
    w32(d + 0x68, 0x2218);
    let (_, front) = create_surface(&mut cpu, dd4, d);
    for n in 1..=3u64 {
        assert_eq!(call_com(&mut cpu, front, 11, &[0, 1]), 0);
        assert_eq!(qpc(&mut cpu), t0 + n * FREQ / 60, "frame {n}");
    }
    assert_eq!(call_import(&mut cpu, "winmm.dll", "timeGetTime", &[]), ms0 + 50, "3 frames = 50 ms");
    // Sleep advances guest time by its argument.
    call_import(&mut cpu, "kernel32.dll", "Sleep", &[10]);
    assert_eq!(call_import(&mut cpu, "winmm.dll", "timeGetTime", &[]), ms0 + 60);
}

#[test]
fn busy_wait_falls_back_to_host_time() {
    let mut c = Cadence::new();
    for _ in 0..STALL_READS {
        assert_eq!(c.read_ns(), 0);
    }
    std::thread::sleep(std::time::Duration::from_millis(5));
    assert!(c.read_ns() >= 5_000_000, "a stalled busy-wait sees time pass");
    let before = c.now_ns();
    c.frame();
    assert_eq!(c.read_ns(), before + 1_000_000_000 / 60, "the next frame returns to the cadence");
}
