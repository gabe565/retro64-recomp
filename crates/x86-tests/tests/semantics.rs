//! Instruction-semantics regression test.
//!
//! For every case in `bugatron-x86-cases`:
//! 1. run the recompiled snippet on the runtime;
//! 2. check hand-derived expectations (Intel SDM semantics, defined flags only);
//! 3. if `tests/fixtures/x86-oracle-*.json` exists (produced by running
//!    `bugatron-x86-oracle` natively on a real x86 CPU), compare against it on
//!    registers, defined flags, scratch memory and masked x87 status bits.
//!
//! Model outcomes are written to `target/x86-model-results.json` for review.

use bugatron_x86_cases::{cases, Kind, Outcome};
use bugatron_x86_tests::{install_runtime, run_model};
use serde_json::{json, Value};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn load_oracles() -> Vec<(String, Value)> {
    let dir = root().join("tests/fixtures");
    let mut out = Vec::new();
    if let Ok(it) = std::fs::read_dir(&dir) {
        for e in it.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with("x86-oracle-") && n.ends_with(".json") {
                let v: Value = serde_json::from_slice(&std::fs::read(e.path()).unwrap()).unwrap();
                out.push((n, v));
            }
        }
    }
    out
}

fn oracle_outcome(v: &Value) -> Outcome {
    let regs: Vec<u32> = v["regs"].as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as u32).collect();
    Outcome {
        regs: regs.try_into().unwrap(),
        eflags: v["eflags"].as_u64().unwrap() as u32,
        fcw: v["fcw"].as_u64().unwrap() as u16,
        fsw: v["fsw"].as_u64().unwrap() as u16,
        mem: unhex(v["mem"].as_str().unwrap()),
    }
}

#[test]
fn instruction_semantics() {
    install_runtime();
    let oracles = load_oracles();
    let mut failures = Vec::new();
    let mut report = Vec::new();
    for (i, case) in cases().iter().enumerate() {
        let (got, notes) = run_model(i, case);
        let mut problems: Vec<String> = Vec::new();

        // Hand-derived expectations.
        match case.kind {
            Kind::KnownDiscrepancy(why) => {
                if notes == 0 {
                    problems.push(format!("known discrepancy ({why}) but the model raised no x87 note"));
                }
            }
            Kind::ReportOnly => {}
            Kind::Strict => {
                for (r, v) in case.expect.regs {
                    if got.regs[*r as usize] != *v {
                        problems.push(format!("{r:?}: got {:#x} want {v:#x}", got.regs[*r as usize]));
                    }
                }
                if let Some(f) = case.expect.flags {
                    if got.eflags & case.flag_mask != f & case.flag_mask {
                        problems.push(format!("flags: got {:#05x} want {:#05x} (mask {:#05x})", got.eflags & case.flag_mask, f & case.flag_mask, case.flag_mask));
                    }
                }
                for (off, b) in &case.expect.mem {
                    if got.mem[*off..*off + b.len()] != b[..] {
                        problems.push(format!("mem[{off}..]: got {} want {}", hex(&got.mem[*off..*off + b.len()]), hex(b)));
                    }
                }
            }
        }

        // Native oracle results.
        let mut oracle_cmp = Vec::new();
        for (file, o) in &oracles {
            let Some(entry) = o["cases"].as_array().and_then(|a| a.iter().find(|e| e["name"] == case.name)) else {
                continue;
            };
            let want = oracle_outcome(entry);
            let mut diffs = Vec::new();
            for r in 0..8 {
                if got.regs[r] != want.regs[r] {
                    diffs.push(format!("reg{r}: got {:#x} oracle {:#x}", got.regs[r], want.regs[r]));
                }
            }
            if got.eflags & case.flag_mask != want.eflags & case.flag_mask {
                diffs.push(format!("flags: got {:#05x} oracle {:#05x}", got.eflags & case.flag_mask, want.eflags & case.flag_mask));
            }
            if got.mem != want.mem {
                let first = got.mem.iter().zip(&want.mem).position(|(a, b)| a != b).unwrap_or(0);
                diffs.push(format!("mem differs from offset {first}: got {} oracle {}", hex(&got.mem[first..(first + 16).min(got.mem.len())]), hex(&want.mem[first..(first + 16).min(want.mem.len())])));
            }
            let fm = case.fsw_mask | 0x3800;
            if got.fsw & fm != want.fsw & fm {
                diffs.push(format!("fsw: got {:#06x} oracle {:#06x} (mask {fm:#06x})", got.fsw & fm, want.fsw & fm));
            }
            if got.fcw != want.fcw {
                diffs.push(format!("fcw: got {:#06x} oracle {:#06x}", got.fcw, want.fcw));
            }
            if case.kind == Kind::Strict && !diffs.is_empty() {
                problems.push(format!("{file}: {}", diffs.join("; ")));
            }
            oracle_cmp.push(json!({"oracle": file, "differences": diffs}));
        }

        if !problems.is_empty() {
            failures.push(format!("{}: {}", case.name, problems.join(" | ")));
        }
        report.push(json!({
            "name": case.name,
            "kind": format!("{:?}", case.kind),
            "regs": got.regs, "eflags": got.eflags, "fcw": got.fcw, "fsw": got.fsw, "mem": hex(&got.mem),
            "x87_notes": notes, "problems": problems, "oracle": oracle_cmp,
        }));
    }
    let out = root().join("target/x86-model-results.json");
    let _ = std::fs::create_dir_all(out.parent().unwrap());
    std::fs::write(&out, serde_json::to_string_pretty(&json!({"oracles": oracles.iter().map(|o| &o.0).collect::<Vec<_>>(), "cases": report})).unwrap()).unwrap();
    println!("{} cases, {} oracle file(s); results in {}", report.len(), oracles.len(), out.display());
    assert!(failures.is_empty(), "instruction-semantics failures:\n  {}", failures.join("\n  "));
}
