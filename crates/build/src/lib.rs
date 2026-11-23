//! A game crate's build script: translates the user's copy of the game's executable
//! and compiles the generated C, every function translated.
//!
//! Inputs (environment, prefixed with the game's `env_prefix`, e.g. `BUGATRON_`):
//!   <PREFIX>EXE     the user's game (never committed): the executable, the installer,
//!                   or a zip, verified by SHA-256 [default: `Target::default_sources`]
//!   <PREFIX>HINTS   path to the recompiler hints [default: `Target::hints`]
//!
//! For the program it sets `GAME_EXE_SHA256` (checked at run time) and
//! `GAME_BUILT_FROM` (a last-resort place to find the game, for `cargo run`), and
//! writes `OUT_DIR/pack.rs` (the `embed-game` feature's files; empty without it).

use retro64_recomp::{recompile, Hints, Options};
use bugatron_unpack::{exe_of, unpack, Unpacked};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What to build.
pub struct Target<'a> {
    /// For messages: the game and version ("Bugatron Gold v1.20a").
    pub title: &'a str,
    /// The executable's file name and SHA-256.
    pub exe_name: &'a str,
    pub sha256: &'a str,
    /// The environment variables' prefix ("BUGATRON_").
    pub env_prefix: &'a str,
    /// Where to look for the game without `<PREFIX>EXE`, relative to the repository
    /// root: the first that exists.
    pub default_sources: &'a [&'a str],
    /// The recompiler hints, relative to the repository root.
    pub hints: &'a str,
    /// Instructions reachable only through exception dispatch (recomp targets.rs).
    pub traps: BTreeMap<u32, String>,
}

pub fn build(t: Target) {
    let var = |name: &str| format!("{}{name}", t.env_prefix);
    println!("cargo:rerun-if-env-changed={}", var("EXE"));
    println!("cargo:rerun-if-env-changed={}", var("HINTS"));
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let exe_path = std::env::var(var("EXE")).map(PathBuf::from).unwrap_or_else(|_| {
        let candidates: Vec<PathBuf> = t.default_sources.iter().map(|p| root.join(p)).collect();
        candidates.iter().find(|p| p.exists()).unwrap_or(candidates.last().expect("a default source")).clone()
    });
    let hints_path = std::env::var(var("HINTS")).map(PathBuf::from).unwrap_or_else(|_| root.join(t.hints));
    println!("cargo:rerun-if-changed={}", exe_path.display());
    println!("cargo:rerun-if-changed={}", hints_path.display());
    let rt_c = root.join("crates/runtime/c");
    println!("cargo:rerun-if-changed={}", rt_c.join("recomp.h").display());

    let bytes = std::fs::read(&exe_path).unwrap_or_else(|e| {
        panic!(
            "cannot read {} ({e}). Set {} to your {} ({}, the installer, or the download zip).",
            exe_path.display(),
            var("EXE"),
            t.title,
            t.exe_name
        )
    });
    let unpacked = unpack(bytes, &exe_path.display().to_string(), t.exe_name).unwrap_or_else(|e| panic!("{e}"));
    let exe = match &unpacked {
        Unpacked::Exe(b) => b.clone(),
        Unpacked::Folder(files) => {
            exe_of(files, t.exe_name).unwrap_or_else(|| panic!("{} does not contain {}", exe_path.display(), t.exe_name)).to_vec()
        }
    };
    let sha = retro64_recomp::sha256_hex(&exe);
    assert_eq!(sha, t.sha256, "{} is not {}", var("EXE"), t.title);
    // The binary checks the executable it is given at run time against the one it was built from.
    println!("cargo:rustc-env=GAME_EXE_SHA256={}", t.sha256);
    write_pack(&exe_path, &unpacked);
    // Last-resort default for finding the game at run time (keeps `cargo run` working in the repo).
    let built_from = exe_path.canonicalize().unwrap_or_else(|_| exe_path.clone());
    println!("cargo:rustc-env=GAME_BUILT_FROM={}", built_from.display());
    let hints: Hints = serde_json::from_slice(
        &std::fs::read(&hints_path).unwrap_or_else(|e| panic!("cannot read hints at {} ({e}).", hints_path.display())),
    )
    .expect("parse hints");

    let opts = Options {
        // The whole program, from the PE entry point.
        roots: vec![bugatron_pe::Pe::parse(&exe).expect("parse the executable").entry_va],
        overrides: Default::default(),
        all_functions: true,
        include_data_referenced: true,
        functions_per_file: 32,
        traps: t.traps,
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
    build.include(&gen).include(&rt_c).warnings(false);
    // C99 (MSVC has no switch for it; its default mode compiles this code).
    if !build.get_compiler().is_like_msvc() {
        build.std("c99");
    }
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
    build.compile("game_gen");
    println!("cargo:rustc-env=GAME_GEN_STATS={}", gen.join("stats.json").display());
}

/// `embed-game`: a table of every file in the game folder, each included with
/// `include_bytes!` (OUT_DIR/pack.rs). Without the feature the table is empty.
fn write_pack(exe_path: &Path, unpacked: &Unpacked) {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let out = out_dir.join("pack.rs");
    let mut rs = String::from("pub static PACK: &[(&str, &[u8])] = &[\n");
    if std::env::var_os("CARGO_FEATURE_EMBED_GAME").is_some() {
        // An installer or zip is unpacked into OUT_DIR/game first.
        let dir = match unpacked {
            Unpacked::Exe(_) => exe_path.parent().expect("game folder").canonicalize().expect("game folder"),
            Unpacked::Folder(files) => {
                let dir = out_dir.join("game");
                let _ = std::fs::remove_dir_all(&dir);
                for (rel, data) in files {
                    let p = dir.join(rel);
                    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                    std::fs::write(p, data).unwrap();
                }
                dir.canonicalize().unwrap()
            }
        };
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
