//! Translates the whole of the user's Bug3D.exe at build time and compiles the generated C.
//! (Same pipeline as crates/poc/build.rs, with every function translated.)
//!
//! Inputs (never committed):
//!   BUGATRON_EXE    path to Bug3D.exe (Gold v1.20a, verified by SHA-256)
//!   BUGATRON_HINTS  path to the recompiler hints [default: hints/gold-1.20a.json]

use bugatron_recomp::{recompile, Hints, Options, BUG3D_GOLD_120A_SHA256};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// The whole program, from the CRT entry point.
const ROOTS: &[u32] = &[0x42ba98];
/// Platform-facing functions replaced by the runtime (documented in
/// docs/graphics/r3d-interface.md and src/r3d.rs). Empty until the facade lands.
const OVERRIDES: &[u32] = &[];

fn main() {
    println!("cargo:rerun-if-env-changed=BUGATRON_EXE");
    println!("cargo:rerun-if-env-changed=BUGATRON_HINTS");
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let exe_path = std::env::var("BUGATRON_EXE").map(PathBuf::from).unwrap_or_else(|_| root.join("_work/ext/gold120/app/Bug3D.exe"));
    let hints_path = std::env::var("BUGATRON_HINTS").map(PathBuf::from).unwrap_or_else(|_| root.join("hints/gold-1.20a.json"));
    println!("cargo:rerun-if-changed={}", exe_path.display());
    println!("cargo:rerun-if-changed={}", hints_path.display());
    let rt_c = root.join("crates/runtime/c");
    println!("cargo:rerun-if-changed={}", rt_c.join("recomp.h").display());

    let exe = std::fs::read(&exe_path).unwrap_or_else(|e| panic!(
        "cannot read Bug3D.exe at {} ({e}). Set BUGATRON_EXE to your copy of Bugatron Gold v1.20a.", exe_path.display()));
    let sha = bugatron_recomp::sha256_hex(&exe);
    assert_eq!(sha, BUG3D_GOLD_120A_SHA256, "BUGATRON_EXE is not Bugatron Gold v1.20a");
    // The binary checks the Bug3D.exe it is given at run time against the one it was built from.
    println!("cargo:rustc-env=BUGATRON_EXE_SHA256={BUG3D_GOLD_120A_SHA256}");
    write_pack(&exe_path);
    // Last-resort default for finding Bug3D.exe at run time (keeps `cargo run` working in the repo).
    let built_from = exe_path.canonicalize().unwrap_or_else(|_| exe_path.clone());
    println!("cargo:rustc-env=BUGATRON_BUILT_FROM={}", built_from.display());
    let hints: Hints = serde_json::from_slice(&std::fs::read(&hints_path).unwrap_or_else(|e| panic!(
        "cannot read hints at {} ({e}).", hints_path.display()))).expect("parse hints");

    let opts = Options {
        roots: ROOTS.to_vec(),
        overrides: OVERRIDES.iter().copied().collect::<BTreeSet<_>>(),
        all_functions: true,
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

/// `embed-game`: a table of every file in the game folder, each included with
/// `include_bytes!` (OUT_DIR/pack.rs). Without the feature the table is empty.
fn write_pack(exe_path: &std::path::Path) {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("pack.rs");
    let mut rs = String::from("pub static PACK: &[(&str, &[u8])] = &[\n");
    if std::env::var_os("CARGO_FEATURE_EMBED_GAME").is_some() {
        let dir = exe_path.parent().expect("game folder").canonicalize().expect("game folder");
        println!("cargo:rerun-if-changed={}", dir.display());
        let mut files = Vec::new();
        let mut stack = vec![dir.clone()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).expect("read game folder").flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.file_name().is_some_and(|n| n != ".DS_Store") {
                    files.push(p);
                }
            }
        }
        files.sort();
        for p in files {
            let rel = p.strip_prefix(&dir).unwrap().components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
            rs.push_str(&format!("    ({rel:?}, include_bytes!({:?})),\n", p.display().to_string()));
        }
    }
    rs.push_str("];\n");
    std::fs::write(out, rs).unwrap();
}
