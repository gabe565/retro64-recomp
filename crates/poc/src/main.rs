//! Static-recompilation proof of concept for Bugatron Gold v1.20a.
//!
//! Runs translated original machine code natively and compares it with
//! reference behaviour captured from the original executable (Windows VM, Frida):
//!
//!   P1  MSVC CRT start-up (entry 0x42ba98) runs until it calls WinMain.
//!   P2  The game's .lev decoder (0x4076a0 → 0x41cfae) on all 30 files.
//!   P3  The frame-timing function (0x40b720 → 0x424440, x87, COM vtable call,
//!       register-cached Sleep import) replayed with the recorded counter values.
//!   P4  CRT srand/rand through per-thread data (_getptd).
//!   M   Guest memory model checks (original-address globals, pointer translation).
//!
//! Usage: bugatron-poc [--exe Bug3D.exe] [--data DIR] [--reference poc-reference.json] [--out results.json]

use bugatron_runtime::{call_guest, with_rt, Cpu, Runtime};
use serde::Deserialize;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

const ENTRY: u32 = 0x42ba98;
const WINMAIN: u32 = 0x4076e0;
const LEV_DECODE: u32 = 0x4076a0;
const FRAME_TIMING: u32 = 0x40b720;
const CRT_SRAND: u32 = 0x42ab66;
const CRT_RAND: u32 = 0x42ab73;
const CRT_MALLOC: u32 = 0x42ab95;
const CRT_FREE: u32 = 0x42a6f8;
const CRT_GETPTD: u32 = 0x42d583;
const G_DIRECTDRAW4: u32 = 0xa06afc;

#[derive(Deserialize, Clone)]
struct LevRef { name: String, in_len: u32, out_len: u32, crc32: u32 }
#[derive(Deserialize, Clone)]
struct RandRef { seed: u32, first16: Vec<i64>, hash10000: u32, state_after: u32 }
#[derive(Deserialize, Clone)]
struct TimingRow { frame: u32, before: BTreeMap<String, u32>, qpc: Vec<[u32; 2]>, after: BTreeMap<String, u32> }
#[derive(Deserialize, Clone)]
struct Reference { x87_probe: Value, rand: Vec<RandRef>, lev: Vec<LevRef>, timing: Vec<TimingRow>, cw_hist: Value }

const TIMER_GLOBALS: &[(&str, u32)] = &[
    ("qpc_base_lo", 0x9fa928), ("qpc_base_hi", 0x9fa92c), ("tick_base", 0x9f679c), ("tick_now_r3d", 0xa02ad0),
    ("tick_saved", 0xa02a4c), ("rebase_flag", 0x9f5788), ("freq_lo", 0xa06b48), ("freq_hi", 0xa06b4c),
    ("tick_now", 0x8429dc), ("delta", 0x8429e0), ("tick_prev", 0x8429e4), ("delta_copy", 0x8429e8),
    ("delta_f32_bits", 0x81d67c),
];

struct Args { exe: PathBuf, data: PathBuf, reference: PathBuf, out: PathBuf }

fn args() -> Args {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut a = Args {
        exe: std::env::var("BUGATRON_EXE").map(PathBuf::from).unwrap_or_else(|_| root.join("_work/ext/gold120/app/Bug3D.exe")),
        data: root.join("_work/ext/gold120/app"),
        reference: root.join("tests/fixtures/poc-reference.json"),
        out: root.join("target/poc-results.json"),
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let v = PathBuf::from(it.next().unwrap_or_else(|| panic!("{k} needs a value")));
        match k.as_str() {
            "--exe" => a.exe = v,
            "--data" => a.data = v,
            "--reference" => a.reference = v,
            "--out" => a.out = v,
            _ => panic!("unknown argument {k}"),
        }
    }
    a
}

#[derive(Default)]
struct Report {
    sections: Vec<(String, bool, Value)>,
}

impl Report {
    fn add(&mut self, name: &str, pass: bool, detail: Value) {
        println!("[{}] {name}", if pass { "PASS" } else { "FAIL" });
        self.sections.push((name.to_string(), pass, detail));
    }
    fn all_pass(&self) -> bool {
        !self.sections.is_empty() && self.sections.iter().all(|s| s.1)
    }
}

fn main() {
    let a = args();
    let exe = std::fs::read(&a.exe).expect("read Bug3D.exe");
    let reference: Reference = serde_json::from_slice(&std::fs::read(&a.reference).expect("read reference")).expect("parse reference");
    let report = Rc::new(RefCell::new(Report::default()));
    REFERENCE_X87.with(|r| *r.borrow_mut() = json!({"probe": reference.x87_probe, "control_word_histogram": reference.cw_hist}));

    // The report lists the guest-to-guest indirect call sites.
    bugatron_runtime::RECORD_INDIRECT.store(true, std::sync::atomic::Ordering::Relaxed);
    let mut rt = Runtime::load(&exe).expect("load");
    println!("guest base {:p}; translated functions: {}", rt.mem.base(), Runtime::function_count());
    pre_run_memory_checks(&rt, &report);

    // P1: the CRT calls WinMain; the override runs P2–P4 and then returns to the CRT.
    {
        let report = report.clone();
        let reference = reference.clone();
        let data = a.data.clone();
        rt.set_override(WINMAIN, Box::new(move |cpu: &mut Cpu| winmain(cpu, &report, &reference, &data)));
    }
    {
        let report = report.clone();
        let out = a.out.clone();
        rt.exit_hook = Some(Box::new(move |code| finish(&report, &out, code)));
    }
    rt.install();
    let mut cpu = with_rt(|rt| rt.initial_cpu());
    call_guest(&mut cpu, ENTRY, &[]);
    // The CRT ends with ExitProcess, which never returns here.
    report.borrow_mut().add("P1 CRT returned without calling ExitProcess", false, json!({}));
    std::process::exit(finish(&report, &a.out, 0xdead));
}

fn pre_run_memory_checks(rt: &Runtime, report: &Rc<RefCell<Report>>) {
    let m = &rt.mem;
    let s_config = m.cstr(0x4375e0);
    let s_prefs = m.cstr(0x4374e0);
    let state = m.r32(0x4370a0);
    let encrypted = m.r32(0x4370a4);
    let host_ok = m.host_ptr(0x4375e0) as usize == m.base() as usize + 0x4375e0;
    // Top of the 32-bit space is readable through the guard region.
    let top = m.r32(0xffff_fffc);
    let pass = s_config == "config.xml" && s_prefs == "prefs" && state == 6 && encrypted == 1 && host_ok && top == 0;
    report.borrow_mut().add("M1 image mapped at original addresses (.rdata strings, .data initial values, host = base + guest)", pass, json!({
        "0x4375e0": s_config, "0x4374e0": s_prefs, "0x4370a0_game_state_initial": state, "0x4370a4_lev_encrypted_flag": encrypted,
        "host_ptr_is_base_plus_guest": host_ok, "read_at_0xfffffffc": top,
        "host_os": std::env::consts::OS, "host_arch": std::env::consts::ARCH,
    }));
}

/// Callee-saved registers per the Win32 calling convention.
fn saved(cpu: &Cpu) -> [u32; 4] { [cpu.ebx, cpu.esi, cpu.edi, cpu.ebp] }
fn restore(cpu: &mut Cpu, s: [u32; 4]) { cpu.ebx = s[0]; cpu.esi = s[1]; cpu.edi = s[2]; cpu.ebp = s[3]; }

fn winmain(cpu: &mut Cpu, report: &Rc<RefCell<Report>>, reference: &Reference, data: &PathBuf) {
    // stdcall WinMain(hInstance, hPrevInstance, lpCmdLine, nCmdShow)
    let (hinst, hprev, cmdline, show, cw) = with_rt(|rt| {
        let e = cpu.esp;
        (rt.mem.r32(e + 4), rt.mem.r32(e + 8), rt.mem.r32(e + 12), rt.mem.r32(e + 16), cpu.fcw)
    });
    let (cmd_text, imports) = with_rt(|rt| (rt.mem.cstr(cmdline), rt.import_calls()));
    let p1 = hinst == 0x400000 && hprev == 0 && cw == 0x27f;
    report.borrow_mut().add("P1 translated MSVC CRT start-up reached WinMain", p1, json!({
        "hInstance": format!("{hinst:#x}"), "hPrevInstance": hprev, "lpCmdLine_guest_ptr": format!("{cmdline:#x}"),
        "lpCmdLine": cmd_text, "nCmdShow": show, "x87_control_word_at_winmain": format!("{cw:#x}"),
        "imports_called_during_startup": imports,
    }));

    let regs = saved(cpu);
    p2_lev(cpu, report, reference, data);
    p3_timing(cpu, report, reference);
    p4_rand(cpu, report, reference);
    memory_after(cpu, report);
    restore(cpu, regs);

    // Return 0 from WinMain: pop return address + 4 stdcall arguments.
    cpu.eax = 0;
    cpu.esp += 4 + 16;
}

fn p2_lev(cpu: &mut Cpu, report: &Rc<RefCell<Report>>, reference: &Reference, data: &PathBuf) {
    let mut rows = Vec::new();
    let mut all = true;
    for r in &reference.lev {
        let path = find_case_insensitive(data, &r.name);
        let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let inv: Vec<u8> = raw.iter().map(|b| !b).collect();
        let buf = call_guest(cpu, CRT_MALLOC, &[inv.len() as u32]);
        let plen = call_guest(cpu, CRT_MALLOC, &[4]);
        with_rt(|rt| rt.mem.write(buf, &inv));
        let out = call_guest(cpu, LEV_DECODE, &[buf, plen]); // frees buf
        let (n, crc) = with_rt(|rt| {
            let n = rt.mem.r32(plen);
            (n, crc32fast::hash(rt.mem.slice(out, n as usize)))
        });
        call_guest(cpu, CRT_FREE, &[out]);
        call_guest(cpu, CRT_FREE, &[plen]);
        let ok = raw.len() as u32 == r.in_len && n == r.out_len && crc == r.crc32;
        all &= ok;
        rows.push(json!({"file": r.name, "out_len": n, "crc32": crc, "match": ok}));
    }
    report.borrow_mut().add(&format!("P2 translated .lev decoder matches the original on {}/{} files", rows.iter().filter(|r| r["match"] == true).count(), rows.len()), all, json!(rows));
}

fn find_case_insensitive(dir: &PathBuf, name: &str) -> PathBuf {
    std::fs::read_dir(dir).ok().and_then(|it| it.filter_map(|e| e.ok()).find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name)).map(|e| e.path()))
        .unwrap_or_else(|| dir.join(name))
}

fn set_timer_globals(snap: &BTreeMap<String, u32>) {
    with_rt(|rt| for &(k, a) in TIMER_GLOBALS { rt.mem.w32(a, snap[k]) });
}
fn get_timer_globals() -> BTreeMap<String, u32> {
    with_rt(|rt| TIMER_GLOBALS.iter().map(|&(k, a)| (k.to_string(), rt.mem.r32(a))).collect())
}

fn p3_timing(cpu: &mut Cpu, report: &Rc<RefCell<Report>>, reference: &Reference) {
    // Real IDirectDraw4 replaced by a runtime COM object in guest memory.
    with_rt(|rt| {
        let dd = rt.create_com_object(1, 32);
        rt.mem.w32(G_DIRECTDRAW4, dd);
    });
    let mut mismatches = Vec::new();
    let mut sleeps_total = 0u64;
    let notes_before = cpu.x87_notes;
    for row in &reference.timing {
        set_timer_globals(&row.before);
        let sleeps0 = with_rt(|rt| {
            rt.clock.qpc_queue = row.qpc.iter().map(|p| (p[0] as u64) | ((p[1] as u64) << 32)).collect();
            rt.clock.qpc_frequency = (row.before["freq_lo"] as u64) | ((row.before["freq_hi"] as u64) << 32);
            rt.clock.qpc_underflow = false;
            rt.clock.sleeps
        });
        let r = call_guest(cpu, FRAME_TIMING, &[]);
        let (left, under, sleeps1) = with_rt(|rt| (rt.clock.qpc_queue.len(), rt.clock.qpc_underflow, rt.clock.sleeps));
        sleeps_total += sleeps1 - sleeps0;
        let got = get_timer_globals();
        let diffs: Vec<String> = row.after.iter().filter(|(k, v)| got.get(*k) != Some(v)).map(|(k, v)| format!("{k}: got {} want {v}", got[k])).collect();
        if r != 1 || left != 0 || under || !diffs.is_empty() || (sleeps1 - sleeps0) as usize + 1 != row.qpc.len() {
            mismatches.push(json!({"frame": row.frame, "ret": r, "qpc_left": left, "qpc_underflow": under, "diffs": diffs}));
        }
    }
    let n = reference.timing.len();
    report.borrow_mut().add(&format!("P3 translated frame timing reproduces {}/{} recorded frames bit-exactly", n - mismatches.len(), n), mismatches.is_empty(), json!({
        "frames": n, "mismatches": mismatches.iter().take(10).collect::<Vec<_>>(), "sleep_calls": sleeps_total,
        "x87_notes_raised": cpu.x87_notes & !notes_before,
    }));

    // Edge cases from the static analysis: <4 ticks spins with Sleep(1); >512 reuses the previous delta.
    let f: u64 = 1_024_000; // 1000 counts per tick, so tick = qpc / 1000 exactly
    let mut edge = Vec::new();
    for (name, prev_delta, qpc_ticks, want_delta, want_sleeps) in [
        ("spin_until_4_ticks", 17u32, vec![1001u64, 1002, 1003, 1004], 4u32, 3u64),
        ("clamp_over_512_reuses_previous", 17, vec![2000], 17, 0),
        ("exactly_512_allowed", 17, vec![1512], 512, 0),
    ] {
        let mut snap: BTreeMap<String, u32> = TIMER_GLOBALS.iter().map(|&(k, _)| (k.to_string(), 0)).collect();
        snap.insert("freq_lo".into(), f as u32);
        snap.insert("tick_prev".into(), 1000);
        snap.insert("tick_now".into(), 1000);
        snap.insert("delta".into(), prev_delta);
        set_timer_globals(&snap);
        let s0 = with_rt(|rt| {
            rt.clock.qpc_queue = qpc_ticks.iter().map(|t| t * 1000).collect();
            rt.clock.qpc_frequency = f;
            rt.clock.sleeps
        });
        call_guest(cpu, FRAME_TIMING, &[]);
        let g = get_timer_globals();
        let sleeps = with_rt(|rt| rt.clock.sleeps) - s0;
        let f32v = f32::from_bits(g["delta_f32_bits"]);
        let ok = g["delta"] == want_delta && sleeps == want_sleeps && f32v == want_delta as f32 / 1024.0;
        edge.push(json!({"case": name, "delta": g["delta"], "sleeps": sleeps, "delta_f32": f32v, "pass": ok}));
    }
    let ok = edge.iter().all(|e| e["pass"] == true);
    report.borrow_mut().add("P3b frame-timing edge cases match the static analysis (4-tick floor, 512-tick clamp)", ok, json!(edge));
}

fn msvc_rand_ref(seed: u32) -> (Vec<i64>, u32, u32) {
    let mut s = seed;
    let mut first = Vec::new();
    let mut acc = 0u32;
    for i in 0..10000 {
        s = s.wrapping_mul(0x343fd).wrapping_add(0x269ec3);
        let v = (s >> 16) & 0x7fff;
        if i < 16 { first.push(v as i64); }
        acc = acc.wrapping_mul(31).wrapping_add(v);
    }
    (first, acc, s)
}

fn p4_rand(cpu: &mut Cpu, report: &Rc<RefCell<Report>>, reference: &Reference) {
    let ptd = call_guest(cpu, CRT_GETPTD, &[]);
    let mut rows = Vec::new();
    let mut all = true;
    for r in &reference.rand {
        call_guest(cpu, CRT_SRAND, &[r.seed]);
        let mut first = Vec::new();
        let mut acc = 0u32;
        for i in 0..10000 {
            let v = call_guest(cpu, CRT_RAND, &[]);
            if i < 16 { first.push(v as i64); }
            acc = acc.wrapping_mul(31).wrapping_add(v);
        }
        let state = with_rt(|rt| rt.mem.r32(ptd + 0x14));
        let formula = msvc_rand_ref(r.seed);
        let ok_ref = first == r.first16 && acc == r.hash10000 && state == r.state_after;
        let ok_formula = first == formula.0 && acc == formula.1 && state == formula.2;
        all &= ok_ref && ok_formula;
        rows.push(json!({"seed": r.seed, "matches_original": ok_ref, "matches_msvc_formula": ok_formula, "first4": &first[..4]}));
    }
    report.borrow_mut().add("P4 translated CRT srand/rand matches the original (10,000 values x 4 seeds)", all, json!({"ptd_guest_ptr": format!("{ptd:#x}"), "seeds": rows}));
}

fn memory_after(cpu: &Cpu, report: &Rc<RefCell<Report>>) {
    let (d, com, import_total) = with_rt(|rt| (rt.diag.clone(), rt.com_calls(), rt.import_calls().values().sum::<u64>()));
    let guest_indirect: Vec<String> = d.guest_indirect_sites.iter()
        .map(|(site, ts)| format!("{site:#010x} -> {}", ts.iter().map(|t| format!("{t:#010x}")).collect::<Vec<_>>().join(",")))
        .collect();
    let ok = !guest_indirect.is_empty() && com.values().sum::<u64>() > 0 && d.host_to_guest_calls > 0;
    report.borrow_mut().add("I1 real indirect calls executed: guest->guest function pointers, COM vtable, IAT imports, host->guest calls", ok, json!({
        "guest_to_guest_indirect_sites": guest_indirect,
        "com_vtable_calls": com,
        "import_calls_total": import_total,
        "host_to_guest_calls": d.host_to_guest_calls,
        "final_esp": format!("{:#x}", cpu.esp),
    }));
}

thread_local! {
    static REFERENCE_X87: RefCell<Value> = const { RefCell::new(Value::Null) };
}

fn finish(report: &Rc<RefCell<Report>>, out: &PathBuf, exit_code: u32) -> i32 {
    let r = report.borrow();
    let stats: Value = std::fs::read(env!("BUGATRON_GEN_STATS")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
    let doc = json!({
        "host": {"os": std::env::consts::OS, "arch": std::env::consts::ARCH},
        "guest_exit_code": exit_code,
        "recompiler_stats": stats,
        "reference_x87_probe_vm": REFERENCE_X87.with(|r| r.borrow().clone()),
        "all_pass": r.all_pass(),
        "results": r.sections.iter().map(|(n, p, d)| json!({"name": n, "pass": p, "detail": d})).collect::<Vec<_>>(),
    });
    if let Some(dir) = out.parent() { let _ = std::fs::create_dir_all(dir); }
    std::fs::write(out, serde_json::to_string_pretty(&doc).unwrap()).expect("write results");
    println!("results written to {}", out.display());
    println!("guest exit code {exit_code:#x}; overall: {}", if r.all_pass() { "PASS" } else { "FAIL" });
    if r.all_pass() { 0 } else { 1 }
}
