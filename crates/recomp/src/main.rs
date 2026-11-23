//! `retro64-recomp`: translate a game's executable (for example Bug3D.exe) into C.
//! The game programs' build scripts do this themselves (bugatron-build); this tool
//! is for looking at a translation by hand.
//!
//! Usage:
//!   retro64-recomp --exe Bug3D.exe --hints recomp_hints.json --out DIR
//!                   [--root 0xADDR]... [--override 0xADDR]... [--all] [--stats-only]

use retro64_recomp::{recompile, Hints, Options, BUG3D_GOLD_120A_SHA256};
use std::path::PathBuf;

fn parse_addr(s: &str) -> u32 {
    let t = s.trim_start_matches("0x");
    u32::from_str_radix(t, 16).unwrap_or_else(|_| panic!("bad address {s}"))
}

fn main() {
    let mut exe = None;
    let mut hints = None;
    let mut out = None;
    let mut opts = Options { functions_per_file: 64, ..Default::default() };
    let mut stats_only = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--exe" => exe = args.next().map(PathBuf::from),
            "--hints" => hints = args.next().map(PathBuf::from),
            "--out" => out = args.next().map(PathBuf::from),
            "--root" => opts.roots.push(parse_addr(&args.next().expect("--root needs an address"))),
            "--override" => { opts.overrides.insert(parse_addr(&args.next().expect("--override needs an address"))); }
            "--all" => opts.all_functions = true,
            "--data-refs" => opts.include_data_referenced = true,
            "--gold-traps" => opts.traps = retro64_recomp::targets::gold_120a_exception_traps(),
            "--stats-only" => stats_only = true,
            other => panic!("unknown argument {other}"),
        }
    }
    let exe = std::fs::read(exe.expect("--exe required")).expect("read exe");
    let sha = retro64_recomp::sha256_hex(&exe);
    let known = [
        (BUG3D_GOLD_120A_SHA256, "Bugatron Gold v1.20a"),
        (retro64_recomp::ZBALL_GOLD_SHA256, "Z-Ball Gold"),
        (retro64_recomp::BUGATRON_WORLDS_TRIAL_SHA256, "Bugatron Worlds v1.01a (free trial)"),
    ];
    match known.iter().find(|(h, _)| *h == sha) {
        Some((_, name)) => eprintln!("executable: {name}"),
        None => eprintln!("warning: executable sha256 {sha} is not one of the known games"),
    }
    let hints: Hints = serde_json::from_slice(&std::fs::read(hints.expect("--hints required")).expect("read hints")).expect("parse hints");
    let result = recompile(&exe, &hints, &opts).unwrap_or_else(|e| { eprintln!("{e}"); std::process::exit(1) });
    let s = &result.stats;
    println!("functions {}  instructions {}  jump_tables {}  indirect_calls {}  indirect_tail_jumps {}  decode_errors {}  traps {}",
        s.functions, s.instructions, s.jump_tables, s.indirect_calls, s.tail_calls, s.decode_errors, s.traps);
    let total: usize = s.unimplemented.values().sum();
    println!("unimplemented instruction sites: {total}");
    for (k, v) in &s.unimplemented {
        println!("  {v:6}  {k}");
    }
    if !stats_only {
        let out = out.expect("--out required");
        std::fs::create_dir_all(&out).expect("create out dir");
        for (name, body) in &result.files {
            std::fs::write(out.join(name), body).expect("write file");
        }
    }
}
