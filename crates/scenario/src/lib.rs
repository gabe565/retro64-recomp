//! Scripted play scenarios, run by each game crate's tests (`tests/scenarios.rs`).
//!
//! A scenario runs the game program with scripted input (`--input`), muted,
//! unpaced (game time is virtual, so it plays the same as in real time) and
//! offscreen (no window), in a temporary save folder, and checks that it exits cleanly and that frames,
//! game memory (`--peek`) and recorded audio are as expected.
//!
//! Frame hashes are kept per platform in the game crate's `tests/frames.json`
//! (frames are the game's artwork, so only their SHA-256 is committed; the GPU
//! backends round differently). A missing hash is printed and recorded with
//! `SCENARIO_UPDATE=1`; a recorded one that differs fails.

use sha2::Digest;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Scenario<'a> {
    pub name: &'a str,
    /// The input script (`--input`).
    pub input: &'a str,
    /// Vertical blanks to run (`--frames`).
    pub frames: u64,
    /// More arguments (for example `--video-mode`).
    pub args: &'a [&'a str],
    pub checks: &'a [Check],
}

pub enum Check {
    /// The frame after this flip has this size.
    FrameSize(u64, u32, u32),
    /// The frame after this flip matches its recorded hash (`tests/frames.json`).
    FrameHash(u64),
    /// The u32 at an address after a flip is (or is not) a value.
    Peek(u64, u32, u32),
    PeekNot(u64, u32, u32),
    /// The u32s at two addresses are equal after a flip.
    PeekSame(u64, u32, u32),
    /// The recording of this many seconds (`--record-audio`) isn't silent.
    Audio(f64),
}

/// Run `s` with the game program `exe`; the crate's directory holds `tests/frames.json`.
pub fn run(exe: &str, crate_dir: &str, s: &Scenario) {
    let dir = std::env::temp_dir().join(format!("scenario-{}-{}-{}", Path::new(exe).file_stem().unwrap().to_string_lossy(), s.name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut flips: Vec<u64> = s.checks.iter().filter_map(|c| match c { Check::FrameSize(f, ..) | Check::FrameHash(f) => Some(*f), _ => None }).collect();
    flips.sort();
    flips.dedup();
    let peeks: Vec<String> = s
        .checks
        .iter()
        .flat_map(|c| match *c {
            Check::Peek(f, a, _) | Check::PeekNot(f, a, _) => vec![(f, a)],
            Check::PeekSame(f, a, b) => vec![(f, a), (f, b)],
            _ => vec![],
        })
        .map(|(f, a)| format!("{f}:{a:#x}"))
        .collect();
    let audio = s.checks.iter().find_map(|c| if let Check::Audio(secs) = c { Some(*secs) } else { None });
    let mut cmd = Command::new(exe);
    cmd.args(["--mute", "--unpaced", "--offscreen", "--save"]).arg(dir.join("save"));
    cmd.args(["--input", s.input, "--frames", &s.frames.to_string()]).args(s.args);
    if !flips.is_empty() {
        let list: Vec<String> = flips.iter().map(u64::to_string).collect();
        cmd.args(["--dump-frames", &list.join(","), "--dump-dir"]).arg(dir.join("frames"));
    }
    if !peeks.is_empty() {
        cmd.args(["--peek", &peeks.join(",")]);
    }
    if let Some(secs) = audio {
        cmd.arg("--record-audio").arg(dir.join("audio.wav")).arg(secs.to_string());
    }
    let out = cmd.output().unwrap_or_else(|e| panic!("{}: cannot run {exe}: {e}", s.name));
    let log = String::from_utf8_lossy(&out.stderr);
    let fail = |what: String| -> ! { panic!("{}: {what}\n--- log (last 30 lines) ---\n{}", s.name, tail(&log, 30)) };
    if !out.status.success() || log.contains("fatal") || log.contains("panicked") {
        fail(format!("exited with {}", out.status));
    }
    let peek = |f: u64, a: u32| -> u32 {
        let key = format!("[peek] flip {f} {a:#x} = ");
        let line = log.lines().find_map(|l| l.strip_prefix(&key)).unwrap_or_else(|| fail(format!("no peek of {a:#x} after flip {f} (did the run reach it?)")));
        u32::from_str_radix(line.trim().trim_start_matches("0x"), 16).unwrap()
    };
    let frame = |f: u64| -> Vec<u8> {
        std::fs::read(dir.join("frames").join(format!("frame-{f:05}.png"))).unwrap_or_else(|_| fail(format!("no frame after flip {f} (did the run reach it?)")))
    };
    // Without a GPU (some CI runners) the game runs but draws nothing: frame
    // checks are skipped, the rest still apply.
    let no_gpu = log.contains("no SDL3 GPU device available");
    if no_gpu {
        eprintln!("{}: no GPU device; frame checks skipped", s.name);
    }
    let hashes_path = PathBuf::from(crate_dir).join("tests/frames.json");
    let platform = std::env::consts::OS;
    let read_hashes = || -> serde_json::Value { std::fs::read(&hashes_path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(serde_json::json!({})) };
    let hashes = read_hashes();
    let mut record = Vec::new();
    for c in s.checks {
        match *c {
            Check::FrameSize(..) | Check::FrameHash(_) if no_gpu => {}
            Check::FrameSize(f, w, h) => {
                let png = frame(f);
                let size = (u32::from_be_bytes(png[16..20].try_into().unwrap()), u32::from_be_bytes(png[20..24].try_into().unwrap()));
                if size != (w, h) {
                    fail(format!("frame after flip {f} is {}x{}, expected {w}x{h}", size.0, size.1));
                }
            }
            Check::FrameHash(f) => {
                let sha: String = sha2::Sha256::digest(frame(f)).iter().map(|b| format!("{b:02x}")).collect();
                let key = format!("{}/{f}", s.name);
                match hashes[platform][&key].as_str() {
                    Some(want) if want == sha => {}
                    Some(want) => fail(format!("frame after flip {f} changed on {platform}: {sha}, recorded {want}")),
                    None => {
                        eprintln!("{}: no recorded hash for frame {f} on {platform}: {sha} (SCENARIO_UPDATE=1 records it)", s.name);
                        record.push((key, sha));
                    }
                }
            }
            Check::Peek(f, a, v) if peek(f, a) != v => fail(format!("{a:#x} after flip {f} is {:#x}, expected {v:#x}", peek(f, a))),
            Check::PeekNot(f, a, v) if peek(f, a) == v => fail(format!("{a:#x} after flip {f} is still {v:#x}")),
            Check::PeekSame(f, a, b) if peek(f, a) != peek(f, b) => fail(format!("{a:#x} and {b:#x} differ after flip {f}")),
            Check::Audio(_) => {
                let wav = std::fs::read(dir.join("audio.wav")).unwrap_or_else(|_| fail("no audio recording".into()));
                let peak = wav[44..].as_chunks::<2>().0.iter().map(|&b| i16::from_le_bytes(b).unsigned_abs()).max().unwrap_or(0);
                if peak < 64 {
                    fail(format!("the recorded audio is silent (peak {peak})"));
                }
            }
            _ => {}
        }
    }
    if !record.is_empty() && std::env::var_os("SCENARIO_UPDATE").is_some() {
        // Scenarios run in parallel: one read-modify-write at a time.
        static FILE: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = FILE.lock().unwrap_or_else(|e| e.into_inner());
        let mut hashes = read_hashes();
        for (k, v) in record {
            hashes[platform][k] = serde_json::Value::String(v);
        }
        std::fs::write(&hashes_path, serde_json::to_string_pretty(&hashes).unwrap() + "\n").unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn tail(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}
