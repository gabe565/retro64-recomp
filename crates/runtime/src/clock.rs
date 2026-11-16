//! Simulation clock: the virtual time the recompiled game observes.
//!
//! The original paces itself against the display:
//! - at start-up, 0x41c180 calibrates a refresh period by timing 60
//!   `WaitForVerticalBlank` intervals with QueryPerformanceCounter (median);
//! - every frame, the same function busy-waits (QPC + `Sleep(0)`) until one
//!   period has passed since its previous exit, and `Flip(DDFLIP_WAIT)` then
//!   waits for the vertical blank.
//!
//! On the 60 Hz reference that gives one frame per refresh. The runtime
//! reproduces it with virtual time that advances only when the guest waits:
//! - `Flip(WAIT)` and `WaitForVerticalBlank` → the next 1/60 s refresh boundary;
//! - `Sleep(n)` → +n ms; `Sleep(0)` (a yield) → +1 µs, so busy-waits end;
//! - QPC, `timeGetTime` and message times all read this one exact counter.
//!
//! Calibration therefore measures exactly 1/60 s, every frame starts on a
//! refresh boundary, and the limiter's overshoot drops one frame every
//! 16,667 frames, deterministically, much as accumulated overshoot does on
//! real hardware. [`pace`] waits for host time to catch up after each wait;
//! presentation never drives the cadence.
//!
//! If the game polls the clock many times without waiting (a busy-wait without
//! `Sleep`), virtual time follows host time until its next wait, with a
//! diagnostic printed once.

use std::time::{Duration, Instant};

/// Refresh rate of the reference display.
pub const REFRESH_HZ: u64 = 60;
const NS: u128 = 1_000_000_000;
/// Virtual time is kept in units of 1/REFRESH_HZ ns, so one refresh is exactly `NS` units.
const UNITS_PER_NS: u128 = REFRESH_HZ as u128;
const REFRESH: u128 = NS;
/// Guest time consumed by `Sleep(0)`.
pub const YIELD_NS: u128 = 1_000;
/// Clock reads without a wait before the stall fallback engages.
pub const STALL_READS: u32 = 1_000_000;

#[derive(Debug, Clone)]
pub struct Cadence {
    /// Virtual time since start, in 1/REFRESH_HZ ns.
    t: u128,
    /// Vertical blanks waited for (Flip and WaitForVerticalBlank).
    pub vblanks: u64,
    reads_since_wait: u32,
    stall_warned: bool,
    stall_host_mark: Option<Instant>,
    host_start: Instant,
}

impl Cadence {
    pub fn new() -> Cadence {
        Cadence { t: 0, vblanks: 0, reads_since_wait: 0, stall_warned: false, stall_host_mark: None, host_start: Instant::now() }
    }

    /// Virtual nanoseconds since start (rounded down).
    pub fn now_ns(&self) -> u128 {
        self.t / UNITS_PER_NS
    }

    /// Virtual time in ticks of `freq` per second (rounded down from the exact value).
    pub fn ticks(&self, freq: u64) -> u64 {
        (self.t * freq as u128 / (NS * UNITS_PER_NS)) as u64
    }

    /// A guest read of the clock in ticks of `freq` (QPC: its frequency; timeGetTime: 1000).
    pub fn read_ticks(&mut self, freq: u64) -> u64 {
        self.note_read();
        self.ticks(freq)
    }

    fn note_read(&mut self) {
        self.reads_since_wait = self.reads_since_wait.saturating_add(1);
        if self.reads_since_wait < STALL_READS {
            return;
        }
        let now = Instant::now();
        match self.stall_host_mark {
            None => {
                if !self.stall_warned {
                    eprintln!("[clock] the game polled the clock {STALL_READS} times without waiting; following host time until it waits");
                    self.stall_warned = true;
                }
            }
            Some(mark) => self.t += (now - mark).as_nanos() * UNITS_PER_NS,
        }
        self.stall_host_mark = Some(now);
    }

    fn waited(&mut self) {
        self.reads_since_wait = 0;
        self.stall_host_mark = None;
    }

    /// Wait for the next vertical blank: the next refresh boundary strictly after now.
    pub fn vblank(&mut self) {
        self.t = (self.t / REFRESH + 1) * REFRESH;
        self.vblanks += 1;
        self.waited();
    }

    /// `Sleep(ms)`.
    pub fn sleep(&mut self, ms: u32) {
        let ns = if ms == 0 { YIELD_NS } else { ms as u128 * 1_000_000 };
        self.t += ns * UNITS_PER_NS;
        self.waited();
    }

    /// How long the host must wait for real time to reach virtual time.
    pub fn pace_delay(&self) -> Duration {
        Duration::from_nanos(self.now_ns() as u64).saturating_sub(self.host_start.elapsed())
    }

    /// If the host has fallen more than `max_lag` behind, forgive the backlog
    /// (the simulation slows rather than racing to catch up).
    pub fn forgive_lag(&mut self, max_lag: Duration) {
        let virt = Duration::from_nanos(self.now_ns() as u64);
        let host = self.host_start.elapsed();
        if host > virt + max_lag {
            self.host_start += host - virt - max_lag;
        }
    }
}

impl Default for Cadence {
    fn default() -> Self {
        Cadence::new()
    }
}

/// Wait for real time to catch up with virtual time (simulation pacing).
pub(crate) fn pace(rt: &mut crate::Runtime) {
    let pace = rt.clock.pace;
    if let Some(c) = rt.clock.cadence.as_mut() {
        c.forgive_lag(Duration::from_millis(250));
        let d = c.pace_delay();
        if pace && !d.is_zero() {
            std::thread::sleep(d);
        }
    }
}

/// The guest waits for a vertical blank (`Flip(DDFLIP_WAIT)`, `WaitForVerticalBlank`).
pub(crate) fn vblank_wait(rt: &mut crate::Runtime) {
    if let Some(c) = rt.clock.cadence.as_mut() {
        c.vblank();
        if rt.clock.stop_after_vblanks.is_some_and(|n| c.vblanks >= n) {
            eprintln!("stopping after {} vertical blanks", c.vblanks);
            crate::print_diagnostics(rt);
            std::process::exit(0);
        }
    }
    pace(rt);
}
