//! Assemble every regression case, recompile it with the real recompiler, and
//! compile the generated C into this crate.

use std::path::PathBuf;

pub const CODE_BASE: u32 = 0x0040_1000;
pub const CASE_STRIDE: u32 = 0x400;

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let rt_c = root.join("crates/runtime/c");
    println!("cargo:rerun-if-changed={}", rt_c.join("recomp.h").display());
    println!("cargo:rerun-if-changed=../x86-cases/src/lib.rs");
    let cases = bugatron_x86_cases::cases();
    let mut snippets = Vec::new();
    for (i, c) in cases.iter().enumerate() {
        let va = CODE_BASE + i as u32 * CASE_STRIDE;
        let bytes = bugatron_x86_cases::assemble(c, va).unwrap_or_else(|e| panic!("assemble {}: {e}", c.name));
        assert!((bytes.len() as u32) < CASE_STRIDE, "case {} too large", c.name);
        snippets.push((va, bytes));
    }
    let out = retro64_recomp::recompile_snippets(&snippets, 64);
    let unimpl: usize = out.stats.unimplemented.values().sum();
    assert_eq!(unimpl, 0, "unimplemented instructions in regression cases: {:?}", out.stats.unimplemented);
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
    build.include(&gen).include(&rt_c).warnings(false);
    // C99 (MSVC has no switch for it; its default mode compiles this code).
    if !build.get_compiler().is_like_msvc() {
        build.std("c99");
    }
    if build.get_compiler().is_like_msvc() {
        build.flag("/fp:precise");
    } else {
        build.flag("-ffp-contract=off").flag("-fno-fast-math");
    }
    build.link_lib_modifier("+whole-archive");
    build.compile("x86_cases_gen");
    let list: Vec<String> = snippets.iter().map(|(va, _)| format!("{va:#x}")).collect();
    std::fs::write(
        PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("case_addrs.rs"),
        format!("pub const CASE_ADDRS: &[u32] = &[{}];\n", list.join(", ")),
    )
    .unwrap();
}
