//! Assemble small guest helper routines (a recording window procedure and
//! recording callbacks), recompile them with the real recompiler, and link the
//! generated C into this crate. Tests then exercise runtime imports and COM
//! methods that call back into guest code.

use bugatron_x86_cases::iced::code_asm::*;
use std::path::PathBuf;

/// Helper entry points, one per 0x100 bytes from CODE_BASE (see src/lib.rs).
const CODE_BASE: u32 = 0x0040_1000;
/// Guest address holding the log write pointer; entries follow at LOG_PTR + 0x10.
const LOG_PTR: u32 = 0x0061_0000;

/// stdcall routine that appends its `n` arguments to the log and returns `value`.
fn recorder(n: u32, value: u32) -> Result<CodeAssembler, IcedError> {
    let mut a = CodeAssembler::new(32)?;
    a.mov(eax, dword_ptr(LOG_PTR))?;
    for i in 0..n {
        a.mov(ecx, dword_ptr(esp + 4 + 4 * i))?;
        a.mov(dword_ptr(eax + 4 * i), ecx)?;
    }
    a.add(eax, 4 * n as i32)?;
    a.mov(dword_ptr(LOG_PTR), eax)?;
    a.mov(eax, value)?;
    a.ret_1(4 * n)?;
    Ok(a)
}

/// Window procedure: logs (msg, wParam, lParam); returns 1 for WM_NCCREATE, else 0.
fn wndproc() -> Result<CodeAssembler, IcedError> {
    let mut a = CodeAssembler::new(32)?;
    a.mov(eax, dword_ptr(LOG_PTR))?;
    for i in 0..3 {
        a.mov(ecx, dword_ptr(esp + 8 + 4 * i))?;
        a.mov(dword_ptr(eax + 4 * i), ecx)?;
    }
    a.add(eax, 12)?;
    a.mov(dword_ptr(LOG_PTR), eax)?;
    a.xor(eax, eax)?;
    a.cmp(dword_ptr(esp + 8), 0x81)?;
    a.sete(al)?;
    a.ret_1(16)?;
    Ok(a)
}

/// EnumDisplayModes callback (lpDDSD2, ctx): copies the 0x7c-byte descriptor to the log, returns 1.
fn mode_copier() -> Result<CodeAssembler, IcedError> {
    let mut a = CodeAssembler::new(32)?;
    a.push(esi)?;
    a.push(edi)?;
    a.mov(esi, dword_ptr(esp + 12))?;
    a.mov(edi, dword_ptr(LOG_PTR))?;
    a.mov(ecx, 0x7c / 4)?;
    a.rep().movsd()?;
    a.mov(dword_ptr(LOG_PTR), edi)?;
    a.pop(edi)?;
    a.pop(esi)?;
    a.mov(eax, 1)?;
    a.ret_1(8)?;
    Ok(a)
}

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let rt_c = root.join("crates/runtime/c");
    println!("cargo:rerun-if-changed={}", rt_c.join("recomp.h").display());
    println!("cargo:rerun-if-changed=build.rs");
    // Order must match the HELPER_* constants in src/lib.rs.
    let helpers: Vec<CodeAssembler> = vec![
        wndproc().unwrap(),
        recorder(2, 1).unwrap(),
        recorder(2, 0).unwrap(),
        recorder(6, 1).unwrap(),
        mode_copier().unwrap(),
        recorder(4, 1).unwrap(),
    ];
    let mut snippets = Vec::new();
    for (i, mut a) in helpers.into_iter().enumerate() {
        let va = CODE_BASE + 0x100 * i as u32;
        let bytes = a.assemble(va as u64).unwrap();
        assert!(bytes.len() < 0x100);
        snippets.push((va, bytes));
    }
    let out = bugatron_recomp::recompile_snippets(&snippets, 64);
    assert_eq!(out.stats.unimplemented.values().sum::<usize>(), 0, "{:?}", out.stats.unimplemented);
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
    // The flag readers' fast paths against flags_get (tests/flags.rs).
    let check = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("c/flags_check.c");
    println!("cargo:rerun-if-changed={}", check.display());
    build.file(&check);
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
    build.compile("runtime_tests_gen");
}
