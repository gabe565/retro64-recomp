//! Translates the whole of the user's BugatronWorlds.exe at build time and
//! compiles the generated C (bugatron-build).
//!
//! Inputs:
//!   BUGATRON_WORLDS_EXE    the user's Bugatron Worlds v1.01a free trial (never
//!                          committed): BugatronWorlds.exe or its installer,
//!                          bugatron_worlds.exe (verified by SHA-256) [default:
//!                          reference/bugatron_worlds.exe]
//!   BUGATRON_WORLDS_HINTS  path to the recompiler hints [default:
//!                          hints/bugatron-worlds-trial.json]

fn main() {
    bugatron_build::build(bugatron_build::Target {
        title: "Bugatron Worlds v1.01a (free trial)",
        exe_name: "BugatronWorlds.exe",
        sha256: bugatron_recomp::BUGATRON_WORLDS_TRIAL_SHA256,
        env_prefix: "BUGATRON_WORLDS_",
        default_sources: &["reference/bugatron_worlds.exe"],
        hints: "hints/bugatron-worlds-trial.json",
        traps: Default::default(),
    });
}
