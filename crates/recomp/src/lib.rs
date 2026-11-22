//! Static recompiler: 32-bit x86 machine code → portable C.
//!
//! Proof-of-concept scope. The pipeline is the real one: hints seed function
//! entries and switch tables, every function is discovered by recursive descent
//! over decoded instructions, and each guest function becomes one C function
//! `void f_XXXXXXXX(Cpu *c)` over the machine model in `runtime/c/recomp.h`.
//! Instructions the emitter does not support become `rt_unimpl()` calls and
//! are counted, so coverage is measurable rather than silently wrong.

mod emit;
pub mod targets;

use bugatron_pe::Pe;
use iced_x86::{Decoder, DecoderOptions, FlowControl, Instruction};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const BUG3D_GOLD_120A_SHA256: &str = "be581a63bccfc7dcc78a5c55cacc02e9f7aab9e5398540d2cc86a7aba758919e";
/// Z-Ball Gold's `Z-Ball.exe` (March 2002).
pub const ZBALL_GOLD_SHA256: &str = "599539b2cbf0b2235921b5ee486be9af4502c715eeb750c1aeed807d2e6f841c";
/// Bugatron Worlds v1.01a's `BugatronWorlds.exe`, the free trial (September 2006).
pub const BUGATRON_WORLDS_TRIAL_SHA256: &str = "08ed700fe1f40804444683a08a20740cdb660bcc7ffcc08eb6f3e804f3ca0e17";

#[derive(Debug, Deserialize)]
pub struct HintFunction {
    pub entry: u32,
    pub name: String,
    #[serde(default)]
    pub thunk: bool,
}

#[derive(Debug, Deserialize)]
pub struct HintJumpTable {
    pub insn: u32,
    pub targets: Vec<u32>,
}

#[derive(Debug, Deserialize)]
pub struct Hints {
    pub program_sha256: String,
    pub functions: Vec<HintFunction>,
    pub jump_tables: Vec<HintJumpTable>,
}

/// What to translate.
#[derive(Debug, Default)]
pub struct Options {
    /// Functions to translate, plus everything they reach through direct calls and tail jumps.
    pub roots: Vec<u32>,
    /// Functions replaced by runtime implementations (`rt_override`). Never followed.
    pub overrides: BTreeSet<u32>,
    /// Translate every hinted function (dry-run coverage / full-program build).
    pub all_functions: bool,
    /// Also root every known function whose address appears in a data section
    /// (initializer tables, exception-handler tables, callback pointers).
    pub include_data_referenced: bool,
    /// Number of functions per generated C file.
    pub functions_per_file: usize,
    /// Instruction address -> reason. Emitted as `rt_trap` (a fatal, explicit
    /// diagnostic) instead of translating the instruction.
    pub traps: BTreeMap<u32, String>,
}

#[derive(Debug, Default, Clone)]
pub struct Stats {
    pub functions: usize,
    pub instructions: usize,
    pub unimplemented: BTreeMap<String, usize>,
    pub tail_calls: usize,
    pub jump_tables: usize,
    pub indirect_calls: usize,
    pub decode_errors: usize,
    pub traps: usize,
}

pub struct Output {
    /// (file name, contents)
    pub files: Vec<(String, String)>,
    pub stats: Stats,
    pub functions: Vec<u32>,
    /// Functions rooted because their address is stored in data.
    pub data_referenced: Vec<u32>,
}

#[derive(Debug)]
pub struct RecompError(pub String);
impl std::fmt::Display for RecompError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for RecompError {}

pub fn sha256_hex(data: &[u8]) -> String {
    let d = Sha256::digest(data);
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// One discovered guest function.
pub(crate) struct Func {
    pub entry: u32,
    pub name: String,
    pub insns: BTreeMap<u32, Instruction>,
    pub labels: BTreeSet<u32>,
    /// Instruction address → resolved switch targets.
    pub tables: BTreeMap<u32, Vec<u32>>,
    /// Branch instruction address → other function it jumps to (tail call).
    pub tail_jump: BTreeMap<u32, u32>,
    /// Instruction address → other function entry it falls through into.
    pub fall_into: BTreeMap<u32, u32>,
    pub callees: BTreeSet<u32>,
    /// Code addresses used as immediate operands (callbacks such as atexit handlers).
    pub addr_taken: BTreeSet<u32>,
    pub decode_errors: BTreeSet<u32>,
}

/// A contiguous range of the guest image backed by bytes.
pub(crate) struct Region<'a> {
    pub va: u32,
    pub bytes: &'a [u8],
    pub exec: bool,
}

/// Code and data the recompiler reads: built from a PE image or from raw snippets.
pub(crate) struct Image<'a> {
    pub regions: Vec<Region<'a>>,
}

impl<'a> Image<'a> {
    pub fn from_pe(pe: &Pe, data: &'a [u8]) -> Self {
        let regions = pe
            .sections
            .iter()
            .filter_map(|s| {
                let start = s.raw_offset as usize;
                let end = (s.raw_offset + s.raw_size) as usize;
                data.get(start..end.min(data.len())).map(|bytes| Region { va: s.va, bytes, exec: s.is_executable() })
            })
            .collect();
        Image { regions }
    }

    fn region(&self, va: u32) -> Option<&Region<'a>> {
        self.regions.iter().find(|r| va >= r.va && ((va - r.va) as usize) < r.bytes.len())
    }

    pub fn byte_at(&self, va: u32) -> Option<u8> {
        self.region(va).map(|r| r.bytes[(va - r.va) as usize])
    }

    pub fn is_exec_start(&self, va: u32) -> bool {
        self.region(va).is_some_and(|r| r.exec && r.va == va)
    }

    fn decode_at(&self, va: u32) -> Option<Instruction> {
        let r = self.region(va)?;
        if !r.exec {
            return None;
        }
        let off = (va - r.va) as usize;
        let bytes = &r.bytes[off..r.bytes.len().min(off + 16)];
        let mut dec = Decoder::with_ip(32, bytes, va as u64, DecoderOptions::NONE);
        let ins = dec.decode();
        if ins.is_invalid() {
            None
        } else {
            Some(ins)
        }
    }
}

fn discover(img: &Image, entry: u32, name: &str, entries: &BTreeSet<u32>, tables: &BTreeMap<u32, Vec<u32>>) -> Func {
    let mut f = Func {
        entry,
        name: name.to_string(),
        insns: BTreeMap::new(),
        labels: BTreeSet::new(),
        tables: BTreeMap::new(),
        tail_jump: BTreeMap::new(),
        fall_into: BTreeMap::new(),
        callees: BTreeSet::new(),
        addr_taken: BTreeSet::new(),
        decode_errors: BTreeSet::new(),
    };
    let mut work = VecDeque::from([entry]);
    let is_other_entry = |a: u32| a != entry && entries.contains(&a);
    while let Some(va) = work.pop_front() {
        if f.insns.contains_key(&va) || f.decode_errors.contains(&va) {
            continue;
        }
        let Some(ins) = img.decode_at(va) else {
            f.decode_errors.insert(va);
            continue;
        };
        f.insns.insert(va, ins);
        for i in 0..ins.op_count() {
            if ins.op_kind(i) == iced_x86::OpKind::Immediate32 {
                let v = ins.immediate32();
                if entries.contains(&v) || is_code_start_candidate(img, v) {
                    f.addr_taken.insert(v);
                }
            }
        }
        let next = ins.next_ip32();
        let follow = |t: u32, f: &mut Func, work: &mut VecDeque<u32>, from: u32, jump: bool| {
            if is_other_entry(t) {
                if jump { f.tail_jump.insert(from, t); } else { f.fall_into.insert(from, t); }
            } else {
                if jump {
                    f.labels.insert(t);
                }
                work.push_back(t);
            }
        };
        match ins.flow_control() {
            FlowControl::Next | FlowControl::Call | FlowControl::IndirectCall => {
                if ins.flow_control() == FlowControl::Call {
                    f.callees.insert(ins.near_branch32());
                }
                follow(next, &mut f, &mut work, va, false);
            }
            FlowControl::ConditionalBranch => {
                follow(ins.near_branch32(), &mut f, &mut work, va, true);
                follow(next, &mut f, &mut work, va, false);
            }
            FlowControl::UnconditionalBranch => {
                follow(ins.near_branch32(), &mut f, &mut work, va, true);
            }
            FlowControl::IndirectBranch => {
                if let Some(ts) = tables.get(&va) {
                    for &t in ts {
                        f.labels.insert(t);
                        work.push_back(t);
                    }
                    f.tables.insert(va, ts.clone());
                }
            }
            FlowControl::Return | FlowControl::Interrupt | FlowControl::Exception | FlowControl::XbeginXabortXend => {}
        }
    }
    f
}

fn is_code_start_candidate(img: &Image, v: u32) -> bool {
    if img.decode_at(v).is_none() {
        return false;
    }
    if img.is_exec_start(v) {
        return true; // first byte of a code section
    }
    // Preceded by `ret`, `int3`/`nop` padding, or `ret imm16` (C2 xx xx).
    matches!(img.byte_at(v.wrapping_sub(1)), Some(0xc3 | 0xcc | 0x90)) || img.byte_at(v.wrapping_sub(3)) == Some(0xc2)
}

pub fn recompile(exe: &[u8], hints: &Hints, opts: &Options) -> Result<Output, RecompError> {
    let sha = sha256_hex(exe);
    if hints.program_sha256 != sha {
        return Err(RecompError(format!("hints are for {}, executable is {}", hints.program_sha256, sha)));
    }
    let pe = Pe::parse(exe).map_err(|e| RecompError(e.to_string()))?;
    let img = Image::from_pe(&pe, exe);

    let mut names: BTreeMap<u32, String> = BTreeMap::new();
    for h in &hints.functions {
        names.insert(h.entry, h.name.clone());
    }
    // Validate switch-table hints: every target must be decodable code in an
    // executable section. Ghidra also reports external references for
    // `jmp [IAT]` tail jumps; those are not tables and are dropped here.
    let tables: BTreeMap<u32, Vec<u32>> = hints
        .jump_tables
        .iter()
        .filter_map(|t| {
            let ok: Vec<u32> = t.targets.iter().copied().filter(|&x| img.decode_at(x).is_some()).collect();
            (!ok.is_empty() && ok.len() == t.targets.len()).then_some((t.insn, ok))
        })
        .collect();

    // Known entries: hints plus roots, extended with call targets as they are found.
    let mut entries: BTreeSet<u32> = names.keys().copied().collect();
    entries.extend(opts.roots.iter().copied());

    // Functions whose addresses are stored in non-executable sections are
    // reachable through pointers (e.g. CRT __xi/__xc initializer tables).
    let mut data_refs: BTreeSet<u32> = BTreeSet::new();
    if opts.include_data_referenced {
        for region in img.regions.iter().filter(|r| !r.exec) {
            let bytes = region.bytes;
            for w in bytes.as_chunks::<4>().0 {
                let v = u32::from_le_bytes(*w);
                if names.contains_key(&v) {
                    data_refs.insert(v);
                } else if is_code_start_candidate(&img, v) {
                    // Not a Ghidra function, but a pointer to code that starts right
                    // after a function end (ret/int3/nop padding) — e.g. a C++ static
                    // initializer reachable only through the CRT's __xc table.
                    data_refs.insert(v);
                    entries.insert(v);
                }
            }
        }
    }
    // Data-referenced code is reachable in either mode (the CRT initializer
    // tables are only ever reached through pointers).
    let queue: VecDeque<u32> = if opts.all_functions {
        names.keys().copied().chain(opts.roots.iter().copied()).chain(data_refs.iter().copied()).collect()
    } else {
        opts.roots.iter().copied().chain(data_refs.iter().copied()).collect()
    };
    let done = discover_all(&img, &names, &tables, &mut entries, queue, &opts.overrides);
    let (files, stats, functions) = generate(&done, &opts.overrides, opts.functions_per_file, &opts.traps);
    Ok(Output { files, stats, functions, data_referenced: data_refs.into_iter().collect() })
}

/// Discover every function reachable from `queue` (direct calls, tail jumps,
/// fallthrough into other entries, and immediate code addresses).
fn discover_all(
    img: &Image,
    names: &BTreeMap<u32, String>,
    tables: &BTreeMap<u32, Vec<u32>>,
    entries: &mut BTreeSet<u32>,
    mut queue: VecDeque<u32>,
    overrides: &BTreeSet<u32>,
) -> BTreeMap<u32, Func> {
    let mut done: BTreeMap<u32, Func> = BTreeMap::new();
    while let Some(e) = queue.pop_front() {
        if done.contains_key(&e) {
            continue;
        }
        let name = names.get(&e).cloned().unwrap_or_else(|| format!("sub_{e:08x}"));
        if overrides.contains(&e) {
            done.insert(e, Func {
                entry: e, name, insns: BTreeMap::new(), labels: BTreeSet::new(), tables: BTreeMap::new(),
                tail_jump: BTreeMap::new(), fall_into: BTreeMap::new(), callees: BTreeSet::new(), addr_taken: BTreeSet::new(), decode_errors: BTreeSet::new(),
            });
            continue;
        }
        let f = discover(img, e, &name, entries, tables);
        for &c in f.callees.iter().chain(f.tail_jump.values()).chain(f.fall_into.values()).chain(f.addr_taken.iter()) {
            if entries.insert(c) {
                // New entry discovered via a call: earlier functions may have
                // decoded straight through it. They stay correct (duplicated code),
                // and the new function gets its own body.
            }
            if !done.contains_key(&c) {
                queue.push_back(c);
            }
        }
        done.insert(e, f);
    }

    done
}

/// Emit C files, the declaration header, and the dispatch table.
fn generate(done: &BTreeMap<u32, Func>, overrides: &BTreeSet<u32>, functions_per_file: usize, traps: &BTreeMap<u32, String>) -> (Vec<(String, String)>, Stats, Vec<u32>) {
    let mut stats = Stats::default();
    let funcs: Vec<&Func> = done.values().collect();
    let per_file = functions_per_file.max(1);
    let mut files = Vec::new();

    let mut decls = String::from("/* Generated by bugatron-recomp. Do not edit or commit. */\n#ifndef BUGATRON_GEN_DECLS_H\n#define BUGATRON_GEN_DECLS_H\n#include \"recomp.h\"\n");
    for f in &funcs {
        decls.push_str(&format!("void f_{:08x}(Cpu *c); /* {} */\n", f.entry, f.name));
    }
    decls.push_str("#endif\n");
    files.push(("gen_decls.h".to_string(), decls));

    for (i, chunk) in funcs.chunks(per_file).enumerate() {
        let mut src = String::from("/* Generated by bugatron-recomp from the user's Bug3D.exe. Do not edit or commit. */\n#include \"gen_decls.h\"\n\n");
        for f in chunk {
            let is_override = overrides.contains(&f.entry);
            src.push_str(&emit::emit_function(f, is_override, &mut stats, traps));
            src.push('\n');
        }
        files.push((format!("gen_{i:03}.c"), src));
    }

    let mut table = String::from("/* Generated dispatch table. */\n#include \"gen_decls.h\"\n\nuint8_t *recomp_mem;\n\ntypedef struct { uint32_t addr; RecompFn fn; } RecompEntry;\nstatic const RecompEntry recomp_table[] = {\n");
    for f in &funcs {
        table.push_str(&format!("    {{ 0x{:08x}u, f_{:08x} }},\n", f.entry, f.entry));
    }
    table.push_str(&format!(
        "}};\nstatic const uint32_t recomp_table_len = {}u;\n\n\
RecompFn recomp_lookup(uint32_t addr) {{\n\
    uint32_t lo = 0, hi = recomp_table_len;\n\
    while (lo < hi) {{\n\
        uint32_t mid = lo + (hi - lo) / 2u;\n\
        if (recomp_table[mid].addr < addr) lo = mid + 1u; else hi = mid;\n\
    }}\n\
    if (lo < recomp_table_len && recomp_table[lo].addr == addr) return recomp_table[lo].fn;\n\
    return 0;\n\
}}\n\n\
void recomp_call_indirect(Cpu *c, uint32_t target, uint32_t site) {{\n\
    if (target >= RT_THUNK_BASE && target < RT_THUNK_END) {{ rt_import(c, target); return; }}\n\
    RecompFn fn = recomp_lookup(target);\n\
    if (fn) {{ rt_note_indirect(target, site); fn(c); return; }}\n\
    rt_unresolved_call(c, target, site);\n\
}}\n\n\
uint32_t recomp_function_count(void) {{ return recomp_table_len; }}\n\nuint32_t recomp_flags_get(const Cpu *c) {{ return flags_get((Cpu *)c); }}\n",
        funcs.len()
    ));
    files.push(("gen_dispatch.c".to_string(), table));

    stats.functions = funcs.len();
    (files, stats, funcs.iter().map(|f| f.entry).collect())
}

/// Recompile standalone code snippets (regression tests). Each snippet is a
/// root function at its address; no hints or overrides are used.
pub fn recompile_snippets(snippets: &[(u32, Vec<u8>)], functions_per_file: usize) -> Output {
    let img = Image { regions: snippets.iter().map(|(va, b)| Region { va: *va, bytes: b.as_slice(), exec: true }).collect() };
    let names: BTreeMap<u32, String> = snippets.iter().map(|(va, _)| (*va, format!("snippet_{va:08x}"))).collect();
    let mut entries: BTreeSet<u32> = names.keys().copied().collect();
    let queue: VecDeque<u32> = names.keys().copied().collect();
    let done = discover_all(&img, &names, &BTreeMap::new(), &mut entries, queue, &BTreeSet::new());
    let (files, stats, functions) = generate(&done, &BTreeSet::new(), functions_per_file, &BTreeMap::new());
    Output { files, stats, functions, data_referenced: Vec::new() }
}
