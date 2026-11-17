//! Bugatron Gold v1.20a, statically recompiled.
//!
//! Runs the translated original program from its CRT entry point on the
//! portable runtime. The runtime implements the imports the game uses; anything
//! not yet implemented stops the run with its name (see docs/runtime-status.md).
//!
//! Options: `bugatron --help` (generated from [`Args`]).
//!
//! Files the game writes (prefs, high scores) go to SAVE_DIR (default: the per-user
//! folder, `bugatron_runtime::files::default_save_dir`);
//! the game directory is only read.

mod source;
mod unpack;

use bugatron_runtime::{call_guest, display, input_script, with_rt, Runtime};
use clap::{CommandFactory, Parser};
use sha2::Digest;
use std::path::{Path, PathBuf};

const ENTRY: u32 = 0x42ba98;

/// A parsed `--input` script (one value, not a list of them).
#[derive(Clone)]
struct Script(Vec<(u64, input_script::Input)>);

/// Bugatron Gold v1.20a, statically recompiled.
#[derive(Parser)]
#[command(version, about, after_help = "Environment: BUGATRON_TRACE=fmod,audio,imports,messages,sdl,gpu,draws,delta,gamepad (or all) prints diagnostics.")]
struct Args {
    /// Your Bugatron Gold v1.20a: Bug3D.exe, the installer (buggold_120.exe), or the downloaded
    /// zip [default: found in the current directory, next to this program, the Windows install
    /// folder, or the copy this program was built from]
    #[arg(long, value_name = "FILE", env = "BUGATRON_EXE")]
    exe: Option<PathBuf>,
    /// Game folder (Audio, Graphics, levels) [default: Bug3D.exe's folder, or the files in the installer]
    #[arg(long, value_name = "DIR")]
    data: Option<PathBuf>,
    /// Where prefs and high scores are written (game files are never modified) [default: the
    /// per-user data folder, printed at startup]
    #[arg(long, value_name = "DIR")]
    save: Option<PathBuf>,

    /// Start in a window instead of fullscreen (Alt+Enter or Ctrl+Cmd+F toggles)
    #[arg(long, help_heading = "Display")]
    windowed: bool,
    /// Keep running while another application has focus
    #[arg(long, help_heading = "Display")]
    run_in_background: bool,
    /// Guest display mode, e.g. 1600x1200x32 or 640x480x16 [default: the largest 4:3 mode that fits the display, 32 bpp]
    #[arg(long, value_name = "WxHxBPP", value_parser = display::parse_mode, help_heading = "Display")]
    video_mode: Option<(u32, u32, u32)>,

    /// Stop after N vertical blanks and print import/COM call counts
    #[arg(long, value_name = "N", help_heading = "Testing")]
    frames: Option<u64>,
    /// Scripted input at flip numbers, e.g. "600:F1,700:LEFT+,760:LEFT-": KEY tap, KEY+ hold,
    /// KEY- release, move@XxY, click@XxY, rel@DXxDY, QUIT. Scripted runs stay windowed and
    /// ignore the host keyboard and mouse
    #[arg(long, value_name = "SCRIPT", value_parser = |s: &str| input_script::parse(s).map(Script), help_heading = "Testing")]
    input: Option<Script>,
    /// Save the screen after these flips as PNG
    #[arg(long, value_name = "A,B,...", value_delimiter = ',', help_heading = "Testing")]
    dump_frames: Vec<u64>,
    /// Where --dump-frames writes
    #[arg(long, value_name = "DIR", default_value = ".", help_heading = "Testing")]
    dump_dir: PathBuf,
    /// Record the FMOD mix (WAV) from the first song on, for SECONDS
    #[arg(long, num_args = 2, value_names = ["FILE", "SECONDS"], help_heading = "Testing")]
    record_audio: Option<Vec<String>>,
}

/// `name` in `dir`, matching the file name case-insensitively (Linux file systems are not).
fn find_in(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        p.is_file() && p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name))
    })
}

/// Folders searched for the game when --exe/--data are not given: the current
/// directory, the folder of this program, the Windows install folder, and the
/// folder of the Bug3D.exe this program was built from (keeps `cargo run` working).
fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    dirs.extend(std::env::current_dir().ok());
    dirs.extend(std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)));
    if cfg!(windows) {
        for var in ["ProgramFiles(x86)", "ProgramFiles"] {
            dirs.extend(std::env::var_os(var).map(|p| PathBuf::from(p).join("Bugatron Gold")));
        }
        dirs.push(PathBuf::from(r"C:\Program Files (x86)\Bugatron Gold"));
    }
    dirs.extend(Path::new(env!("BUGATRON_BUILT_FROM")).parent().map(Path::to_path_buf));
    let mut out: Vec<PathBuf> = Vec::new();
    for d in dirs {
        if !out.contains(&d) {
            out.push(d);
        }
    }
    out
}

/// The first game source (Bug3D.exe, the installer, or the download zip, in that
/// order) in the search folders. Exits with the folders tried if there is none.
fn find_game() -> PathBuf {
    let dirs = search_dirs();
    for d in &dirs {
        if let Some(p) = source::NAMES.iter().find_map(|n| find_in(d, n)) {
            return p;
        }
    }
    let list: Vec<String> = dirs.iter().map(|d| format!("  {}", d.display())).collect();
    eprintln!(
        "error: Bugatron Gold v1.20a not found ({}). Looked in:\n{}\nRun from the game folder, or pass --exe (or set BUGATRON_EXE).",
        source::NAMES.join(", "),
        list.join("\n")
    );
    std::process::exit(2);
}

/// The game folder built into this program (`--features embed-game`; empty without it).
mod embedded {
    include!(concat!(env!("OUT_DIR"), "/pack.rs"));
}

fn main() {
    let args = Args::parse();
    // The game: --exe (or BUGATRON_EXE), else the embedded copy, else found on disk.
    let game = match args.exe {
        Some(p) => source::open(&p),
        None => match source::embedded(embedded::PACK) {
            Some(g) => Ok(g),
            None => source::open(&find_game()),
        },
    }
    .unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(2)
    });
    // Game files: --data, else those that came with the game.
    let (data, pack) = match args.data {
        Some(d) => (Some(d), None),
        None => (game.dir.clone(), game.pack),
    };
    let save = args.save;
    let frames = args.frames;
    let dumps = args.dump_frames;
    let dump_dir = args.dump_dir;
    let video_mode = args.video_mode;
    let scripted = args.input.is_some();
    let input = args.input.map(|s| s.0).unwrap_or_default();
    let windowed = args.windowed;
    if !dumps.is_empty() {
        let _ = std::fs::create_dir_all(&dump_dir);
    }
    let record = args.record_audio.map(|v| {
        let secs = v[1].parse::<f64>().unwrap_or_else(|_| {
            Args::command().error(clap::error::ErrorKind::ValueValidation, format!("--record-audio: bad SECONDS {:?}", v[1])).exit()
        });
        let path = PathBuf::from(&v[0]);
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            let _ = std::fs::create_dir_all(parent);
        }
        (path, secs)
    });
    let (image, exe_label) = (game.exe, game.label);
    eprintln!("Bug3D.exe: {exe_label}");
    let sha: String = sha2::Sha256::digest(&image).iter().map(|b| format!("{b:02x}")).collect();
    if sha != env!("BUGATRON_EXE_SHA256") {
        eprintln!("error: {exe_label} is not Bugatron Gold v1.20a's Bug3D.exe (SHA-256 {sha});\nthis build was translated from {}.", env!("BUGATRON_EXE_SHA256"));
        std::process::exit(2);
    }
    let mut rt = Runtime::load(&image).expect("load Bug3D.exe");
    rt.user.ignore_host_input = scripted;
    rt.user.host_fullscreen = !windowed && !scripted;
    rt.user.run_in_background = args.run_in_background;
    match (&data, pack) {
        (Some(d), _) => eprintln!("game folder: {}", d.display()),
        (None, Some(p)) => eprintln!("game folder: in memory ({} files)", p.len()),
        (None, None) => {}
    }
    rt.files.data_dir = data.unwrap_or_default();
    rt.files.pack = pack;
    let save = save.or_else(bugatron_runtime::files::default_save_dir).unwrap_or_else(|| PathBuf::from("save"));
    eprintln!("saves: {}", save.display());
    rt.files.save_dir = save;
    rt.clock.cadence = Some(bugatron_runtime::clock::Cadence::new());
    rt.clock.pace = true;
    rt.clock.stop_after_vblanks = frames;
    rt.capture_frames = dumps;
    rt.capture_dir = dump_dir;
    if let Some((path, secs)) = record {
        rt.fmod.mixer().recording = Some(bugatron_runtime::audio::mixer::Recording { seconds: secs, ..Default::default() });
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
                let (vt_ms, qpc) = rt.clock.cadence.as_ref().map(|c| (c.now_ns() / 1_000_000, c.qpc_reads)).unwrap_or((0, 0));
                eprintln!("[delta] flip {n}: {} (tick_now {}, virtual {vt_ms} ms, qpc reads {qpc})", rt.mem.r32(0x8429e0), rt.mem.r32(0x8429dc));
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

