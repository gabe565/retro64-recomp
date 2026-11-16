//! Simulation clock: the virtual time the recompiled game observes.
//!
//! The original runs a variable-step update timed with QueryPerformanceCounter
//! and presented with a vsync'd `Flip`, so on its reference hardware each loop
//! iteration spans one 60 Hz refresh. The runtime reproduces that cadence
//! exactly and independently of the host display:
//!
//! - virtual time = `frames` × 1/60 s + the total of the game's `Sleep` calls,
//!   computed from integers (no drift);
//! - every guest time source (QPC, `timeGetTime`, message times) derives from it,
//!   so it is constant within a frame and advances by one refresh per `Flip`;
//! - [`pace`] then waits until host time catches up with virtual time.
//!   Presentation never drives the cadence, so a later higher-rate renderer can
//!   present more often without changing simulation speed.
//!
//! If the game polls the clock many times without a frame boundary or `Sleep`
//! (a busy-wait), virtual time follows host time from then on until the next
//! boundary, and a diagnostic is printed once.

use std::time::{Duration, Instant};

/// Simulation frame rate of the reference hardware.
pub const FRAME_RATE: u64 = 60;
const NS: u128 = 1_000_000_000;
/// Clock reads without an advance before the stall fallback engages.
pub const STALL_READS: u32 = 1_000_000;

#[derive(Debug, Clone)]
pub struct Cadence {
    pub frames: u64,
    /// Total of `Sleep` durations, in nanoseconds.
    pub slept_ns: u128,
    /// Extra time from the stall fallback, in nanoseconds.
    pub stalled_ns: u128,
    reads_since_advance: u32,
    stall_warned: bool,
    host_start: Instant,
    stall_host_mark: Option<Instant>,
}

impl Cadence {
    pub fn new() -> Cadence {
        Cadence { frames: 0, slept_ns: 0, stalled_ns: 0, reads_since_advance: 0, stall_warned: false, host_start: Instant::now(), stall_host_mark: None }
    }

    /// Virtual time since start, exactly, in units of 1/FRAME_RATE nanoseconds.
    fn scaled(&self) -> u128 {
        self.frames as u128 * NS + (self.slept_ns + self.stalled_ns) * FRAME_RATE as u128
    }

    /// Virtual nanoseconds since start (rounded down).
    pub fn now_ns(&self) -> u128 {
        self.scaled() / FRAME_RATE as u128
    }

    /// Virtual time in ticks of `freq` per second (rounded down from the exact value).
    pub fn ticks(&self, freq: u64) -> u64 {
        (self.scaled() * freq as u128 / (NS * FRAME_RATE as u128)) as u64
    }

    /// A guest read of the clock in ticks of `freq` (QPC: its frequency; timeGetTime: 1000).
    pub fn read_ticks(&mut self, freq: u64) -> u64 {
        self.read_ns();
        self.ticks(freq)
    }

    /// Note a guest read of the clock; engages the stall fallback when needed.
    pub fn read_ns(&mut self) -> u128 {
        self.reads_since_advance = self.reads_since_advance.saturating_add(1);
        if self.reads_since_advance >= STALL_READS {
            let now = Instant::now();
            match self.stall_host_mark {
                None => {
                    if !self.stall_warned {
                        eprintln!("[clock] the game polled the clock {STALL_READS} times without a Flip or Sleep; following host time until the next frame");
                        self.stall_warned = true;
                    }
                    self.stall_host_mark = Some(now);
                }
                Some(mark) => {
                    self.stalled_ns += (now - mark).as_nanos();
                    self.stall_host_mark = Some(now);
                }
            }
        }
        self.now_ns()
    }

    fn advanced(&mut self) {
        self.reads_since_advance = 0;
        self.stall_host_mark = None;
    }

    /// A frame boundary of the original loop (`Flip`).
    pub fn frame(&mut self) {
        self.frames += 1;
        self.advanced();
    }

    /// `Sleep(ms)`: the guest's time passes by `ms`.
    pub fn sleep(&mut self, ms: u32) {
        self.slept_ns += ms as u128 * 1_000_000;
        self.advanced();
    }

    /// How long the host must wait for real time to reach virtual time.
    pub fn pace_delay(&self) -> Duration {
        let target = Duration::from_nanos(self.now_ns() as u64);
        target.saturating_sub(self.host_start.elapsed())
    }

    /// If the host has fallen more than `max_lag` behind, forgive the backlog
    /// (the simulation slows rather than racing to catch up).
    pub fn forgive_lag(&mut self, max_lag: Duration) {
        let virt = Duration::from_nanos(self.now_ns() as u64);
        let host = self.host_start.elapsed();
        if host > virt + max_lag {
            self.host_start += host - virt;
        }
    }
}

impl Default for Cadence {
    fn default() -> Self {
        Cadence::new()
    }
}

/// A frame boundary of the original game loop (`Flip`): advance virtual time
/// and wait for real time to catch up.
pub(crate) fn frame_boundary() {
    let delay = crate::with_rt(|rt| {
        rt.clock.cadence.as_mut().map(|c| {
            c.frame();
            c.forgive_lag(Duration::from_millis(250));
            c.pace_delay()
        })
    });
    if let Some(d) = delay {
        if crate::with_rt(|rt| rt.clock.pace) && !d.is_zero() {
            std::thread::sleep(d);
        }
    }
}
