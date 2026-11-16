//! Bugatron Gold v1.20a, statically recompiled.
//!
//! Runs the translated original program from its CRT entry point on the
//! portable runtime. The runtime implements the imports the game uses; anything
//! not yet implemented stops the run with its name (see docs/runtime-status.md).
//!
//! Usage: bugatron [--exe Bug3D.exe] [--data GAME_DIR] [--save SAVE_DIR] [--frames N]
//!
//! `--frames N` stops after N vertical blanks and prints import/COM call counts.
//! `--dump-frames A,B,...` saves the front buffer after those flips as PNG in
//! `--dump-dir` (default `.`).
//!
//! Files the game writes (prefs, high scores) go to SAVE_DIR (default `./save`);
//! the game directory is only read.

use bugatron_runtime::{call_guest, with_rt, Runtime};
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
    eprintln!("translated functions: {}", Runtime::function_count());
    rt.install();
    let mut cpu = with_rt(|rt| rt.initial_cpu());
    call_guest(&mut cpu, ENTRY, &[]);
    eprintln!("CRT entry returned without ExitProcess");
    std::process::exit(1);
}
