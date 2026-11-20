//! Translates the whole of the user's Bug3D.exe at build time and compiles the
//! generated C (bugatron-build).
//!
//! Inputs:
//!   BUGATRON_EXE    the user's Bugatron Gold v1.20a (never committed): Bug3D.exe, the
//!                   installer, or the download zip (verified by SHA-256) [default:
//!                   _work/ext/gold120/app/Bug3D.exe, else reference/Bugatron_Win_EN_Gold_120.zip]
//!   BUGATRON_HINTS  path to the recompiler hints [default: hints/gold-1.20a.json]

fn main() {
    bugatron_build::build(bugatron_build::Target {
        title: "Bugatron Gold v1.20a",
        exe_name: "Bug3D.exe",
        sha256: bugatron_recomp::BUG3D_GOLD_120A_SHA256,
        env_prefix: "BUGATRON_",
        default_sources: &["_work/ext/gold120/app/Bug3D.exe", "reference/Bugatron_Win_EN_Gold_120.zip"],
        hints: "hints/gold-1.20a.json",
        traps: bugatron_recomp::targets::gold_120a_exception_traps(),
    });
}
