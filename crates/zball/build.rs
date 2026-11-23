//! Translates the whole of the user's Z-Ball.exe at build time and compiles the
//! generated C (bugatron-build).
//!
//! Inputs:
//!   ZBALL_EXE    the user's Z-Ball Gold (never committed): Z-Ball.exe, the installer,
//!                or a zip of either (verified by SHA-256) [default:
//!                _work/others/Z-Ball_Win_EN_Full-Rip/Z-Ball Gold/Z-Ball.exe, else
//!                reference/Z-Ball_Win_EN_Full-Rip.zip]
//!   ZBALL_HINTS  path to the recompiler hints [default: hints/zball-gold.json]

fn main() {
    bugatron_build::build(bugatron_build::Target {
        title: "Z-Ball Gold",
        exe_name: "Z-Ball.exe",
        sha256: retro64_recomp::ZBALL_GOLD_SHA256,
        env_prefix: "ZBALL_",
        default_sources: &["_work/others/Z-Ball_Win_EN_Full-Rip/Z-Ball Gold/Z-Ball.exe", "reference/Z-Ball_Win_EN_Full-Rip.zip"],
        hints: "hints/zball-gold.json",
        traps: retro64_recomp::targets::zball_gold_exception_traps(),
    });
}
