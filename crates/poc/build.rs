//! Translates the user's Bug3D.exe at build time and compiles the generated C.
//!
//! Inputs (never committed):
//!   BUGATRON_EXE    Bug3D.exe, the installer, or the download zip (Gold v1.20a, verified
//!                   by SHA-256) [default: as crates/game/build.rs]
//!   BUGATRON_HINTS  path to the recompiler hints [default: hints/gold-1.20a.json]

#[path = "../game/src/unpack.rs"]
#[allow(dead_code)]
mod unpack;

use bugatron_recomp::{recompile, Hints, Options, BUG3D_GOLD_120A_SHA256};
use unpack::{unpack, Unpacked};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Proof-of-concept roots: CRT entry, the .lev decoder wrapper, the frame
/// timing function, CRT srand/rand/malloc/free, and _getptd.
const ROOTS: &[u32] = &[0x42ba98, 0x4076a0, 0x40b720, 0x42ab66, 0x42ab73, 0x42ab95, 0x42a6f8, 0x42d583];
/// WinMain is replaced by the proof-of-concept test driver (the CRT still calls it).
const OVERRIDES: &[u32] = &[0x4076e0];

fn main() {
    println!("cargo:rerun-if-env-changed=BUGATRON_EXE");
    println!("cargo:rerun-if-env-changed=BUGATRON_HINTS");
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let exe_path = std::env::var("BUGATRON_EXE").map(PathBuf::from).unwrap_or_else(|_| {
        let extracted = root.join("_work/ext/gold120/app/Bug3D.exe");
        if extracted.exists() { extracted } else { root.join("reference/Bugatron_Win_EN_Gold_120.zip") }
    });
    let hints_path = std::env::var("BUGATRON_HINTS").map(PathBuf::from).unwrap_or_else(|_| root.join("hints/gold-1.20a.json"));
    println!("cargo:rerun-if-changed={}", exe_path.display());
    println!("cargo:rerun-if-changed={}", hints_path.display());
    let rt_c = root.join("crates/runtime/c");
    println!("cargo:rerun-if-changed={}", rt_c.join("recomp.h").display());

    let bytes = std::fs::read(&exe_path).unwrap_or_else(|e| panic!(
        "cannot read {} ({e}). Put Bugatron_Win_EN_Gold_120.zip in reference/, or set BUGATRON_EXE.", exe_path.display()));
    let exe = match unpack(bytes, &exe_path.display().to_string()).unwrap_or_else(|e| panic!("{e}")) {
        Unpacked::Exe(b) => b,
        Unpacked::Folder(files) => unpack::exe_of(&files).expect("Bug3D.exe in the installer").to_vec(),
    };
    let sha = bugatron_recomp::sha256_hex(&exe);
    assert_eq!(sha, BUG3D_GOLD_120A_SHA256, "BUGATRON_EXE is not Bugatron Gold v1.20a");
    let hints: Hints = serde_json::from_slice(&std::fs::read(&hints_path).unwrap_or_else(|e| panic!(
        "cannot read hints at {} ({e}).", hints_path.display()))).expect("parse hints");

    let opts = Options {
        roots: ROOTS.to_vec(),
        overrides: OVERRIDES.iter().copied().collect::<BTreeSet<_>>(),
        all_functions: false,
        include_data_referenced: true,
        functions_per_file: 32,
        traps: bugatron_recomp::targets::gold_120a_exception_traps(),
    };
    let out = recompile(&exe, &hints, &opts).expect("recompile");
    let gen = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("gen");
    std::fs::create_dir_all(&gen).unwrap();
    let mut build = cc::Build::new();
    for (name, body) in &out.files {
        let p = gen.join(name);
        std::fs::write(&p, body).unwrap();
        if name.ends_with(".c") {
            build.file(&p);
        }
    }
    let s = &out.stats;
    let data_refs: Vec<String> = out.data_referenced.iter().map(|a| format!("\"{a:#010x}\"")).collect();
    std::fs::write(gen.join("stats.json"), format!(
        "{{\"data_referenced_roots\":[{}],\"functions\":{},\"instructions\":{},\"jump_tables\":{},\"indirect_calls\":{},\"indirect_tail_jumps\":{},\"decode_errors\":{},\"unimplemented\":{}}}",
        data_refs.join(","), s.functions, s.instructions, s.jump_tables, s.indirect_calls, s.tail_calls, s.decode_errors,
        s.unimplemented.values().sum::<usize>())).unwrap();
    build.include(&gen).include(&rt_c).std("c99").warnings(false);
    // Preserve x86 floating-point semantics: no FMA contraction, no fast-math.
    let tool = build.get_compiler();
    if tool.is_like_msvc() {
        build.flag("/fp:precise");
    } else {
        build.flag("-ffp-contract=off").flag("-fno-fast-math");
    }
    // Generated code and the Rust runtime reference each other; link the whole
    // archive so order-sensitive linkers (GNU ld) resolve both directions.
    build.link_lib_modifier("+whole-archive");
    build.compile("bugatron_gen");
    println!("cargo:rustc-env=BUGATRON_GEN_STATS={}", gen.join("stats.json").display());
}
