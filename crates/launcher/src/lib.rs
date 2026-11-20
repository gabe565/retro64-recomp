//! The program around a recompiled game: finds the user's copy of the game, sets
//! up the runtime with the game's profile, and runs the translated original from
//! its entry point. The runtime implements the imports the game uses; anything not
//! yet implemented stops the run with its name (see docs/runtime-status.md).
//!
//! Each game's binary calls [`run`] with its [`Title`]. Options: `<game> --help`
//! (generated from [`Args`]).
//!
//! Files the game writes (prefs, high scores) go to SAVE_DIR (default: the game's
//! per-user folder, `bugatron_runtime::files::default_save_dir`); the game
//! directory is only read.

pub mod source;

use bugatron_runtime::files::Pack;
use bugatron_runtime::game::Game;
use bugatron_runtime::{call_guest, display, input_script, with_rt, Runtime};
use clap::{CommandFactory, FromArgMatches, Parser};
use sha2::Digest;
use std::path::{Path, PathBuf};

/// A game's binary: the game and how to find it.
pub struct Title {
    /// The runtime's profile of the game.
    pub game: &'static Game,
    /// The game and version the binary was built for ("Bugatron Gold v1.20a"), and
    /// the binary's name.
    pub title: &'static str,
    pub bin: &'static str,
    /// The environment variable that can name the game (as `--exe`).
    pub exe_env: &'static str,
    /// File names looked for in each search folder, in order: the executable, then
    /// the installer and download zip.
    pub sources: &'static [&'static str],
    /// Folders under Program Files the Windows installer installs to.
    pub install_folders: &'static [&'static str],
    /// The executable's SHA-256 and where the build found the game
    /// (`env!("GAME_EXE_SHA256")` and `env!("GAME_BUILT_FROM")`; bugatron-build).
    pub sha256: &'static str,
    pub built_from: &'static str,
    /// The game folder built into the binary (`embed-game`; empty without it).
    pub pack: Pack,
    /// For BUGATRON_TRACE=delta: the game's frame delta and current time (u32
    /// ticks), printed after every flip.
    pub delta: Option<(u32, u32)>,
}

/// A parsed `--input` script (one value, not a list of them).
#[derive(Clone)]
struct Script(Vec<(u64, input_script::Input)>);

/// A game, statically recompiled.
#[derive(Parser)]
#[command(version, after_help = "Environment: BUGATRON_TRACE=fmod,audio,imports,messages,sdl,gpu,draws,delta,gamepad (or all) prints diagnostics.")]
struct Args {
    /// Your copy of the game (`Title::sources`; help set by `run`)
    #[arg(long, value_name = "FILE")]
    exe: Option<PathBuf>,
    /// Game folder (Audio, Graphics, levels) [default: the executable's folder, or the files in the installer]
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
/// folder of the game this program was built from (keeps `cargo run` working).
fn search_dirs(t: &Title) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    dirs.extend(std::env::current_dir().ok());
    dirs.extend(std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)));
    if cfg!(windows) {
        for folder in t.install_folders {
            for var in ["ProgramFiles(x86)", "ProgramFiles"] {
                dirs.extend(std::env::var_os(var).map(|p| PathBuf::from(p).join(folder)));
            }
            dirs.push(PathBuf::from(r"C:\Program Files (x86)").join(folder));
        }
    }
    dirs.extend(Path::new(t.built_from).parent().map(Path::to_path_buf));
    let mut out: Vec<PathBuf> = Vec::new();
    for d in dirs {
        if !out.contains(&d) {
            out.push(d);
        }
    }
    out
}

/// The first game source (`Title::sources`, in that order) in the search folders.
/// Exits with the folders tried if there is none.
fn find_game(t: &Title) -> PathBuf {
    let dirs = search_dirs(t);
    for d in &dirs {
        if let Some(p) = t.sources.iter().find_map(|n| find_in(d, n)) {
            return p;
        }
    }
    let list: Vec<String> = dirs.iter().map(|d| format!("  {}", d.display())).collect();
    eprintln!(
        "error: {} not found ({}). Looked in:\n{}\nRun from the game folder, or pass --exe (or set {}).",
        t.title,
        t.sources.join(", "),
        list.join("\n"),
        t.exe_env
    );
    std::process::exit(2);
}

/// Run the game: parse the command line, find the game, and run it until it exits.
pub fn run(t: &Title) -> ! {
    let exe_help = format!(
        "Your {}: {} [default: found in the current directory, next to this program, the Windows install \
         folder, or the copy this program was built from]",
        t.title,
        t.sources.join(", ")
    );
    let cmd = Args::command()
        .name(t.bin)
        .bin_name(t.bin)
        .about(format!("{}, statically recompiled.", t.title))
        .mut_arg("exe", |a| a.env(t.exe_env).help(exe_help));
    let args = Args::from_arg_matches(&cmd.get_matches()).unwrap_or_else(|e| e.exit());
    let exe_name = t.game.exe_name;
    // The game: --exe (or its environment variable), else the embedded copy, else found on disk.
    let game = match args.exe {
        Some(p) => source::open(&p, exe_name),
        None => match source::embedded(t.pack, exe_name) {
            Some(g) => Ok(g),
            None => source::open(&find_game(t), exe_name),
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
    eprintln!("{exe_name}: {exe_label}");
    let sha: String = sha2::Sha256::digest(&image).iter().map(|b| format!("{b:02x}")).collect();
    if sha != t.sha256 {
        eprintln!("error: {exe_label} is not {}'s {exe_name} (SHA-256 {sha});\nthis build was translated from {}.", t.title, t.sha256);
        std::process::exit(2);
    }
    let mut rt = Runtime::load(&image).unwrap_or_else(|e| panic!("load {exe_name}: {e}"));
    rt.set_game(t.game);
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
    let save = save.or_else(|| bugatron_runtime::files::default_save_dir(t.game.save_folder)).unwrap_or_else(|| PathBuf::from("save"));
    eprintln!("saves: {}", save.display());
    rt.files.save_dir = save;
    // config.toml (bindings, display, fixes); scripted runs keep the original behaviour.
    if !scripted {
        bugatron_runtime::config::load_into(&mut rt);
        rt.user.host_fullscreen &= rt.display.fullscreen;
    }
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
    let mode = video_mode.or_else(|| rt.display.resolution.filter(|_| !scripted).map(|(w, h)| (w, h, 32))).or_else(|| {
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
    let delta = t.delta.filter(|_| std::env::var("BUGATRON_TRACE").is_ok_and(|t| t.contains("delta")));
    if delta.is_some() || !input.is_empty() {
        rt.frame_hook = Some(Box::new(move |rt, n| {
            if let Some((delta, now)) = delta {
                // The game's frame delta, written by its frame-timing function.
                let (vt_ms, qpc) = rt.clock.cadence.as_ref().map(|c| (c.now_ns() / 1_000_000, c.qpc_reads)).unwrap_or((0, 0));
                eprintln!("[delta] flip {n}: {} (tick_now {}, virtual {vt_ms} ms, qpc reads {qpc})", rt.mem.r32(delta), rt.mem.r32(now));
            }
            input_script::apply(rt, &input, n);
        }));
    }
    eprintln!("translated functions: {}", Runtime::function_count());
    rt.install();
    let (mut cpu, entry) = with_rt(|rt| (rt.initial_cpu(), rt.pe.entry_va));
    call_guest(&mut cpu, entry, &[]);
    eprintln!("CRT entry returned without ExitProcess");
    std::process::exit(1);
}

