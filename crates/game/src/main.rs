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
use std::path::PathBuf;

const ENTRY: u32 = 0x42ba98;

/// A parsed `--input` script (one value, not a list of them).
#[derive(Clone)]
struct Script(Vec<(u64, input_script::Input)>);

/// Bugatron Gold v1.20a, statically recompiled.
#[derive(Parser)]
#[command(version, about, after_help = "Environment: BUGATRON_TRACE=fmod,audio,imports,messages,sdl,gpu,draws,delta (or all) prints diagnostics.")]
struct Args {
    /// Your Bug3D.exe (Gold v1.20a; checked by SHA-256)
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

fn main() {
    let args = Args::parse();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let exe = args.exe.unwrap_or_else(|| root.join("_work/ext/gold120/app/Bug3D.exe"));
    let data = args.data;
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
    let image = std::fs::read(&exe).unwrap_or_else(|e| {
        eprintln!("error: cannot read {}: {e}\nPoint --exe (or BUGATRON_EXE) at your copy of Bugatron Gold v1.20a's Bug3D.exe.", exe.display());
        std::process::exit(2)
    });
    let sha: String = sha2::Sha256::digest(&image).iter().map(|b| format!("{b:02x}")).collect();
    if sha != env!("BUGATRON_EXE_SHA256") {
        eprintln!("error: {} is not Bugatron Gold v1.20a's Bug3D.exe (SHA-256 {sha});\nthis build was translated from {}.", exe.display(), env!("BUGATRON_EXE_SHA256"));
        std::process::exit(2);
    }
    let mut rt = Runtime::load(&image).expect("load Bug3D.exe");
    rt.user.ignore_host_input = scripted;
    rt.user.host_fullscreen = !windowed && !scripted;
    rt.user.run_in_background = args.run_in_background;
    rt.files.data_dir = data.unwrap_or_else(|| exe.parent().map(PathBuf::from).unwrap_or_default());
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

