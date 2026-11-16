//! Bugatron Gold v1.20a, statically recompiled.
//!
//! Runs the translated original program from its CRT entry point on the
//! portable runtime. The runtime implements the imports the game uses; anything
//! not yet implemented stops the run with its name (see docs/runtime-status.md).
//!
//! Usage: bugatron [--exe Bug3D.exe] [--data GAME_DIR] [--save SAVE_DIR] [--frames N]
//!
//! `--frames N` stops after N vertical blanks and prints import/COM call counts.
//! `--video-mode WxHxBPP` (or "W x H x BPP") picks that mode in the game's video
//! dialog (saved to prefs by the game, as when a user picks it). Without it, the
//! largest 4:3 mode that fits the display at 32 bits is chosen (display.rs).
//! `--input "600:F1,700:LEFT+,760:LEFT-"` injects keys at flip numbers: `KEY`
//! taps (released two flips later), `KEY+` holds, `KEY-` releases. Keys:
//! A–Z, 0–9, F1–F12, ESC, ENTER, SPACE, TAB, LEFT, RIGHT, UP, DOWN, CTRL,
//! SHIFT, ALT, PGUP, PGDN, HOME, END. Mouse: `move@XxY`, `click@XxY` (left
//! button, released two flips later), in guest screen coordinates.
//! `--record-audio FILE.wav SECONDS` records the FMOD output from the first song
//! on (written when `--frames` stops the run).
//! `--dump-frames A,B,...` saves the front buffer after those flips as PNG in
//! `--dump-dir` (default `.`).
//!
//! Files the game writes (prefs, high scores) go to SAVE_DIR (default `./save`);
//! the game directory is only read.

use bugatron_runtime::{call_guest, display, input_script, with_rt, Runtime};
use std::path::PathBuf;

const ENTRY: u32 = 0x42ba98;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut exe = std::env::var("BUGATRON_EXE").map(PathBuf::from).unwrap_or_else(|_| root.join("_work/ext/gold120/app/Bug3D.exe"));
    let mut data = exe.parent().map(PathBuf::from).unwrap_or_default();
    let mut save = PathBuf::from("save");
    let mut frames = None;
    let mut dumps = Vec::new();
    let mut dump_dir = PathBuf::from(".");
    let mut video_mode = None;
    let mut record: Option<(PathBuf, f64)> = None;
    let mut input: Vec<(u64, input_script::Input)> = Vec::new();
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let v = PathBuf::from(it.next().unwrap_or_else(|| panic!("{k} needs a value")));
        match k.as_str() {
            "--exe" => exe = v,
            "--data" => data = v,
            "--save" => save = v,
            "--frames" => frames = Some(v.to_string_lossy().parse::<u64>().expect("--frames N")),
            "--dump-frames" => dumps = v.to_string_lossy().split(',').map(str::trim).filter(|x| !x.is_empty()).map(|x| x.parse::<u64>().expect("--dump-frames A,B")).collect(),
            "--dump-dir" => dump_dir = v,
            "--video-mode" => video_mode = Some(display::parse_mode(&v.to_string_lossy()).unwrap_or_else(|e| panic!("--video-mode: {e}"))),
            "--record-audio" => {
                let secs = it.next().and_then(|s| s.parse::<f64>().ok()).expect("--record-audio FILE SECONDS");
                record = Some((v, secs));
            }
            "--input" => input = input_script::parse(&v.to_string_lossy()).unwrap_or_else(|e| panic!("--input: {e}")),
            _ => panic!("unknown argument {k}"),
        }
    }
    let image = std::fs::read(&exe).unwrap_or_else(|e| panic!("cannot read {}: {e}", exe.display()));
    let mut rt = Runtime::load(&image).expect("load Bug3D.exe");
    rt.files.data_dir = data;
    rt.files.save_dir = save;
    rt.clock.cadence = Some(bugatron_runtime::clock::Cadence::new());
    rt.clock.pace = true;
    rt.clock.stop_after_vblanks = frames;
    rt.capture_frames = dumps;
    rt.capture_dir = dump_dir;
    if let Some((path, secs)) = record {
        rt.fmod.mixer().recording = Some(bugatron_runtime::audio::mixer::Recording { limit_frames: (secs * 48000.0) as usize, ..Default::default() });
        rt.audio_record_path = Some(path);
    }
    // Display mode: explicit, else sized to the host display; offered to the game
    // as a driver mode and picked in its dialog.
    let mode = video_mode.or_else(|| {
        let d = display::query()?;
        let m = display::auto_mode(&d);
        eprintln!("display {}x{} points at {}x density ({}x{} pixels): rendering at {}", d.w_points, d.h_points, d.density, d.pixels().0, d.pixels().1, display::mode_label(m));
        Some(m)
    });
    if let Some(m) = mode {
        if (m.0 * 3 != m.1 * 4) && !bugatron_runtime::ddraw::MODES.contains(&m) {
            eprintln!("note: {}x{} is not 4:3; the original's fullscreen projection assumes 4:3 and stretches", m.0, m.1);
        }
        rt.com.offer_mode(m.0, m.1);
        rt.video_mode = Some(display::mode_label(m));
    }
    let trace_delta = std::env::var("BUGATRON_TRACE").is_ok_and(|t| t.contains("delta"));
    if trace_delta || !input.is_empty() {
        rt.frame_hook = Some(Box::new(move |rt, n| {
            if trace_delta {
                // The game's frame delta (1/1024 s ticks), written by its frame-timing function.
                let vt_ms = rt.clock.cadence.as_ref().map(|c| c.now_ns() / 1_000_000).unwrap_or(0);
                eprintln!("[delta] flip {n}: {} (tick_now {}, virtual {vt_ms} ms)", rt.mem.r32(0x8429e0), rt.mem.r32(0x8429dc));
            }
            input_script::apply(rt, &input, n);
        }));
    }
    eprintln!("translated functions: {}", Runtime::function_count());
    rt.install();
    let mut cpu = with_rt(|rt| rt.initial_cpu());
    call_guest(&mut cpu, ENTRY, &[]);
    eprintln!("CRT entry returned without ExitProcess");
    std::process::exit(1);
}

