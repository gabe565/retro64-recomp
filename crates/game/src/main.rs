//! Bugatron Gold v1.20a, statically recompiled.
//!
//! Runs the translated original program from its CRT entry point on the
//! portable runtime. The runtime implements the imports the game uses; anything
//! not yet implemented stops the run with its name (see docs/runtime-status.md).
//!
//! Usage: bugatron [--exe Bug3D.exe] [--data GAME_DIR] [--save SAVE_DIR] [--frames N]
//!
//! `--frames N` stops after N vertical blanks and prints import/COM call counts.
//! `--video-mode "W x H x BPP"` picks that mode in the game's video dialog
//! (saved to prefs by the game, as when a user picks it).
//! `--input "600:F1,700:LEFT+,760:LEFT-"` injects keys at flip numbers: `KEY`
//! taps (released two flips later), `KEY+` holds, `KEY-` releases. Keys:
//! A–Z, 0–9, F1–F12, ESC, ENTER, SPACE, TAB, LEFT, RIGHT, UP, DOWN, CTRL,
//! SHIFT, ALT, PGUP, PGDN, HOME, END. Mouse: `move@XxY`, `click@XxY` (left
//! button, released two flips later), in guest screen coordinates.
//! `--dump-frames A,B,...` saves the front buffer after those flips as PNG in
//! `--dump-dir` (default `.`).
//!
//! Files the game writes (prefs, high scores) go to SAVE_DIR (default `./save`);
//! the game directory is only read.

use bugatron_runtime::{call_guest, user, with_rt, Runtime};
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
    let mut input: Vec<(u64, Input)> = Vec::new();
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let v = PathBuf::from(it.next().unwrap_or_else(|| panic!("{k} needs a value")));
        match k.as_str() {
            "--exe" => exe = v,
            "--data" => data = v,
            "--save" => save = v,
            "--frames" => frames = Some(v.to_string_lossy().parse::<u64>().expect("--frames N")),
            "--dump-frames" => dumps = v.to_string_lossy().split(',').map(|x| x.parse::<u64>().expect("--dump-frames A,B")).collect(),
            "--dump-dir" => dump_dir = v,
            "--video-mode" => video_mode = Some(v.to_string_lossy().into_owned()),
            "--input" => input = parse_input(&v.to_string_lossy()),
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
    rt.video_mode = video_mode;
    let trace_delta = std::env::var("BUGATRON_TRACE").is_ok_and(|t| t.contains("delta"));
    if trace_delta || !input.is_empty() {
        rt.frame_hook = Some(Box::new(move |rt, n| {
            if trace_delta {
                // The game's frame delta (1/1024 s ticks), written by its frame-timing function.
                eprintln!("[delta] flip {n}: {}", rt.mem.r32(0x8429e0));
            }
            for (_, ev) in input.iter().filter(|e| e.0 == n) {
                match *ev {
                    Input::Key(sc, down) => user::key_input_rt(rt, sc, down, false, false),
                    Input::Move(p) => user::mouse_move_rt(rt, p),
                    Input::Button(down, p) => user::mouse_button_rt(rt, 1, down, p),
                }
            }
        }));
    }
    eprintln!("translated functions: {}", Runtime::function_count());
    rt.install();
    let mut cpu = with_rt(|rt| rt.initial_cpu());
    call_guest(&mut cpu, ENTRY, &[]);
    eprintln!("CRT entry returned without ExitProcess");
    std::process::exit(1);
}

/// SDL scancode for a key name used by `--input`.
fn scancode(name: &str) -> i32 {
    let n = name.to_ascii_uppercase();
    let b = n.as_bytes();
    match n.as_str() {
        _ if b.len() == 1 && b[0].is_ascii_uppercase() => 4 + (b[0] - b'A') as i32,
        _ if b.len() == 1 && b[0] == b'0' => 39,
        _ if b.len() == 1 && b[0].is_ascii_digit() => 30 + (b[0] - b'1') as i32,
        _ if n.starts_with('F') && n[1..].parse::<i32>().is_ok_and(|k| (1..=12).contains(&k)) => 57 + n[1..].parse::<i32>().unwrap(),
        "ENTER" => 40,
        "ESC" => 41,
        "TAB" => 43,
        "SPACE" => 44,
        "HOME" => 74,
        "PGUP" => 75,
        "END" => 77,
        "PGDN" => 78,
        "RIGHT" => 79,
        "LEFT" => 80,
        "DOWN" => 81,
        "UP" => 82,
        "CTRL" => 224,
        "SHIFT" => 225,
        "ALT" => 226,
        _ => panic!("--input: unknown key {name}"),
    }
}

#[derive(Clone, Copy, Debug)]
enum Input {
    Key(i32, bool),
    Move((i32, i32)),
    Button(bool, (i32, i32)),
}

/// Parse `--input` into (flip, event) pairs.
fn parse_input(spec: &str) -> Vec<(u64, Input)> {
    let mut ev = Vec::new();
    let point = |s: &str| -> (i32, i32) {
        let (x, y) = s.split_once('x').unwrap_or_else(|| panic!("--input: expected XxY, got {s}"));
        (x.parse().unwrap(), y.parse().unwrap())
    };
    for item in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (frame, key) = item.split_once(':').unwrap_or_else(|| panic!("--input: expected FRAME:KEY, got {item}"));
        let f: u64 = frame.parse().unwrap_or_else(|_| panic!("--input: bad frame in {item}"));
        if let Some(p) = key.strip_prefix("move@") {
            ev.push((f, Input::Move(point(p))));
        } else if let Some(p) = key.strip_prefix("click@") {
            let p = point(p);
            ev.push((f, Input::Move(p)));
            ev.push((f, Input::Button(true, p)));
            ev.push((f + 2, Input::Button(false, p)));
        } else if let Some(k) = key.strip_suffix('+') {
            ev.push((f, Input::Key(scancode(k), true)));
        } else if let Some(k) = key.strip_suffix('-') {
            ev.push((f, Input::Key(scancode(k), false)));
        } else {
            ev.push((f, Input::Key(scancode(key), true)));
            ev.push((f + 2, Input::Key(scancode(key), false)));
        }
    }
    ev
}
