//! Bugatron Gold v1.20a, statically recompiled.
//!
//! Runs the translated original program from its CRT entry point on the
//! portable runtime. The runtime implements the imports the game uses; anything
//! not yet implemented stops the run with its name (see docs/runtime-status.md).
//!
//! Options: `bugatron --help` (generated from [`Args`]).
//!
//! Files the game writes (prefs, high scores) go to SAVE_DIR (default `./save`);
//! the game directory is only read.

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
#[command(version, about, after_help = "Environment: BUGATRON_TRACE=fmod,audio,imports,messages,sdl,gpu,draws,delta (or all) prints diagnostics.")]
struct Args {
    /// Your Bug3D.exe (Gold v1.20a; checked by SHA-256) [default: found in the current directory,
    /// next to this program, the Windows install folder, or the copy this program was built from]
    #[arg(long, value_name = "FILE", env = "BUGATRON_EXE")]
    exe: Option<PathBuf>,
    /// Game folder (Audio, Graphics, levels) [default: Bug3D.exe's folder]
    #[arg(long, value_name = "DIR")]
    data: Option<PathBuf>,
    /// Where prefs and high scores are written (game files are never modified)
    #[arg(long, value_name = "DIR", default_value = "save")]
    save: PathBuf,

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

/// The first search folder holding Bug3D.exe, or (when the exe is embedded and only
/// the assets are needed) the game's Audio and Graphics folders. Exits with the
/// list of folders tried if none does.
fn find_game(need_exe: bool) -> (Option<PathBuf>, PathBuf) {
    let dirs = search_dirs();
    for d in &dirs {
        if let Some(exe) = find_in(d, "Bug3D.exe") {
            return (Some(exe), d.clone());
        }
        let has_dir = |n: &str| std::fs::read_dir(d).ok().into_iter().flatten().flatten()
            .any(|e| e.path().is_dir() && e.file_name().to_string_lossy().eq_ignore_ascii_case(n));
        if !need_exe && has_dir("Audio") && has_dir("Graphics") {
            return (None, d.clone());
        }
    }
    let list: Vec<String> = dirs.iter().map(|d| format!("  {}", d.display())).collect();
    let what = if need_exe { "Bug3D.exe (Bugatron Gold v1.20a)" } else { "the Bugatron Gold game folder (Audio, Graphics)" };
    eprintln!("error: {what} not found. Looked in:\n{}\nRun from the game folder, or pass --exe/--data (or set BUGATRON_EXE).", list.join("\n"));
    std::process::exit(2);
}

/// The game folder built into this program (`--features embed-game`; empty without it).
mod embedded {
    include!(concat!(env!("OUT_DIR"), "/pack.rs"));
}

/// The embedded game folder and its Bug3D.exe, if the program was built with them.
fn embedded() -> Option<(bugatron_runtime::files::Pack, &'static [u8])> {
    let exe = embedded::PACK.iter().find(|(p, _)| p.eq_ignore_ascii_case("Bug3D.exe"))?.1;
    Some((embedded::PACK, exe))
}

fn main() {
    let args = Args::parse();
    // Bug3D.exe: --exe (or BUGATRON_EXE), else the embedded copy, else found on disk.
    // Game files: --data, else the embedded folder (with the embedded exe), else the
    // folder of the Bug3D.exe used, else (embedded exe only) a game folder found on disk.
    let embedded = embedded();
    let (exe, game_dir) = match (args.exe, embedded) {
        (Some(p), _) => {
            let dir = p.parent().map(PathBuf::from).unwrap_or_default();
            (Some(p), Some(dir))
        }
        (None, Some(_)) => (None, None),
        (None, None) => {
            let (exe, dir) = find_game(true);
            (exe, Some(dir))
        }
    };
    let pack = if exe.is_none() && args.data.is_none() { embedded.map(|e| e.0) } else { None };
    let data = args.data.or(game_dir).or_else(|| pack.is_none().then(|| find_game(false).1));
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
    let (image, exe_label) = match &exe {
        Some(p) => {
            let image = std::fs::read(p).unwrap_or_else(|e| {
                eprintln!("error: cannot read {}: {e}\nPoint --exe (or BUGATRON_EXE) at your copy of Bugatron Gold v1.20a's Bug3D.exe.", p.display());
                std::process::exit(2)
            });
            (image, p.display().to_string())
        }
        None => (embedded.expect("embedded Bug3D.exe").1.to_vec(), "embedded".to_string()),
    };
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
        (None, Some(p)) => eprintln!("game folder: embedded ({} files)", p.len()),
        (None, None) => {}
    }
    rt.files.data_dir = data.unwrap_or_default();
    rt.files.pack = pack;
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

