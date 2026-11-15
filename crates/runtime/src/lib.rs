//! Portable runtime for statically recompiled Bugatron code.
//!
//! Responsibilities (proof-of-concept scope):
//! - reserve the 4 GiB guest address space and load the user's executable at
//!   its original addresses (`mem`, [`Runtime::load`]);
//! - fill the import address table with thunk addresses and implement the
//!   imported Win32 functions the translated code calls (`imports`);
//! - provide COM objects in guest memory whose vtables point at thunks;
//! - call translated functions from Rust ([`call_guest`]) and dispatch calls
//!   from translated code back into Rust (`rt_*` C symbols).
//!
//! The runtime is single-threaded, like the original game.

// The `rt_*` functions are C entry points called only by generated code with
// its live `Cpu` pointer; they cannot be `unsafe fn` and remain FFI-safe.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

pub mod cpu;
mod ddraw;
pub mod dialog;
mod ddraw_caps;
pub mod files;
mod heap;
mod imports;
pub mod mem;
pub mod x87trig;

pub use cpu::Cpu;
pub use mem::GuestMemory;

use bugatron_pe::Pe;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, VecDeque};

// ---- guest memory layout -----------------------------------------------------
pub const STACK_LOW: u32 = 0x0010_0000;
pub const STACK_TOP: u32 = 0x0030_0000;
pub const TEB_ADDR: u32 = 0x7ffd_e000;
pub const HEAP_LOW: u32 = 0x1000_0000;
pub const HEAP_HIGH: u32 = 0x3000_0000;
pub const VALLOC_LOW: u32 = 0x3000_0000;
pub const VALLOC_HIGH: u32 = 0x7000_0000;
/// Import thunks: 0xF0000000 + index * 16.
pub const THUNK_BASE: u32 = 0xF000_0000;
/// COM method thunks: 0xF0100000 + interface * 0x1000 + slot * 16.
pub const COM_THUNK_BASE: u32 = 0xF010_0000;
/// Return address pushed by [`call_guest`]; never executed.
pub const HOST_RETURN: u32 = 0xF0FF_FFF0;

type RecompFn = unsafe extern "C" fn(*mut Cpu);
extern "C" {
    fn recomp_lookup(addr: u32) -> Option<RecompFn>;
    fn recomp_function_count() -> u32;
}

pub type Override = Box<dyn FnMut(&mut Cpu)>;

/// Virtual time sources. Proof-of-concept: replay queues for exact differential tests.
#[derive(Default)]
pub struct Clock {
    pub qpc_queue: VecDeque<u64>,
    pub qpc_frequency: u64,
    pub qpc_underflow: bool,
    pub time_get_time: u32,
    pub sleeps: u64,
    pub monitor_frequency: u32,
}

#[derive(Default, Debug, Clone)]
pub struct Diagnostics {
    pub import_calls: BTreeMap<String, u64>,
    pub com_calls: BTreeMap<String, u64>,
    pub host_to_guest_calls: u64,
    pub guest_indirect_targets: BTreeMap<u32, u64>,
    pub guest_indirect_sites: BTreeMap<u32, std::collections::BTreeSet<u32>>,
}

pub struct Runtime {
    pub(crate) com: ddraw::ComRegistry,
    pub(crate) dialog: Option<dialog::Dialog>,
    pub mem: GuestMemory,
    pub pe: Pe,
    pub(crate) import_names: Vec<(String, String)>,
    pub(crate) heap: heap::Heap,
    pub(crate) valloc: heap::VirtualRegions,
    pub(crate) tls: Vec<u32>,
    pub(crate) last_error: u32,
    pub clock: Clock,
    pub diag: Diagnostics,
    pub command_line: u32,
    pub module_path: String,
    /// Guest file handles over the user's game directory and the save directory.
    pub files: files::Files,
    pub(crate) env_block: u32,
    pub exit_hook: Option<Box<dyn FnMut(u32) -> i32>>,
    overrides: HashMap<u32, Override>,
}

thread_local! {
    static RT: RefCell<Option<Runtime>> = const { RefCell::new(None) };
}

/// Run `f` with the installed runtime. Panics if none is installed.
pub fn with_rt<R>(f: impl FnOnce(&mut Runtime) -> R) -> R {
    RT.with(|r| f(r.borrow_mut().as_mut().expect("runtime not installed")))
}

impl Runtime {
    /// Reserve guest memory and map `exe` at its original image base.
    pub fn load(exe: &[u8]) -> Result<Runtime, String> {
        if !cfg!(target_endian = "little") {
            return Err("host must be little-endian".into());
        }
        let pe = Pe::parse(exe).map_err(|e| e.to_string())?;
        let mem = GuestMemory::reserve().map_err(|e| format!("reserve guest memory: {e}"))?;
        mem.commit(pe.image_base, pe.size_of_image as usize).map_err(|e| e.to_string())?;
        // Headers (first page) and sections at their original addresses.
        let hdr = exe.len().min(0x1000);
        mem.write(pe.image_base, &exe[..hdr]);
        for s in &pe.sections {
            let n = (s.raw_size as usize).min(s.virtual_size.max(s.raw_size) as usize);
            let src = exe.get(s.raw_offset as usize..s.raw_offset as usize + n).ok_or("section data truncated")?;
            mem.write(s.va, src);
        }
        // Import address table → thunk addresses.
        let mut import_names = Vec::new();
        for (i, imp) in pe.imports.iter().enumerate() {
            mem.w32(imp.iat_va, THUNK_BASE + (i as u32) * 16);
            import_names.push((imp.dll.to_ascii_lowercase(), imp.name.clone()));
        }
        // Stack and thread information block.
        mem.commit(STACK_LOW, (STACK_TOP - STACK_LOW) as usize).map_err(|e| e.to_string())?;
        mem.commit(TEB_ADDR, 0x1000).map_err(|e| e.to_string())?;
        mem.w32(TEB_ADDR, 0xffff_ffff); // SEH chain end
        mem.w32(TEB_ADDR + 4, STACK_TOP);
        mem.w32(TEB_ADDR + 8, STACK_LOW);
        mem.w32(TEB_ADDR + 0x18, TEB_ADDR);
        mem.w32(TEB_ADDR + 0x20, 1); // process id
        mem.w32(TEB_ADDR + 0x24, 1); // thread id

        let mut rt = Runtime {
            mem,
            pe,
            import_names,
            heap: heap::Heap::new(HEAP_LOW, HEAP_HIGH),
            valloc: heap::VirtualRegions::new(VALLOC_LOW, VALLOC_HIGH),
            tls: Vec::new(),
            last_error: 0,
            clock: Clock { qpc_frequency: 24_000_000, monitor_frequency: 60, ..Default::default() },
            diag: Diagnostics::default(),
            command_line: 0,
            module_path: "C:\\Bugatron\\Bug3D.exe".into(),
            files: Default::default(),
            env_block: 0,
            exit_hook: None,
            overrides: HashMap::new(),
            com: Default::default(),
            dialog: None,
        };
        rt.command_line = rt.alloc_cstr("\"C:\\Bugatron\\Bug3D.exe\"");
        rt.env_block = rt.heap_alloc(16, true);
        Ok(rt)
    }

    /// A runtime with no executable image: guest memory, stack, TEB and heap only.
    /// Used by instruction-semantics tests.
    pub fn bare() -> Result<Runtime, String> {
        let pe = Pe { image_base: 0, entry_va: 0, size_of_image: 0, sections: Vec::new(), imports: Vec::new() };
        let mem = GuestMemory::reserve().map_err(|e| format!("reserve guest memory: {e}"))?;
        mem.commit(STACK_LOW, (STACK_TOP - STACK_LOW) as usize).map_err(|e| e.to_string())?;
        mem.commit(TEB_ADDR, 0x1000).map_err(|e| e.to_string())?;
        mem.w32(TEB_ADDR, 0xffff_ffff);
        mem.w32(TEB_ADDR + 0x18, TEB_ADDR);
        Ok(Runtime {
            mem,
            pe,
            import_names: Vec::new(),
            heap: heap::Heap::new(HEAP_LOW, HEAP_HIGH),
            valloc: heap::VirtualRegions::new(VALLOC_LOW, VALLOC_HIGH),
            tls: Vec::new(),
            last_error: 0,
            clock: Clock::default(),
            diag: Diagnostics::default(),
            command_line: 0,
            module_path: String::new(),
            files: Default::default(),
            env_block: 0,
            exit_hook: None,
            overrides: HashMap::new(),
            com: Default::default(),
            dialog: None,
        })
    }

    /// Install as the thread's runtime (required before running guest code).
    pub fn install(self) {
        RT.with(|r| *r.borrow_mut() = Some(self));
    }

    pub fn uninstall() -> Option<Runtime> {
        RT.with(|r| r.borrow_mut().take())
    }

    pub fn initial_cpu(&self) -> Cpu {
        Cpu::new(STACK_TOP - 16, TEB_ADDR)
    }

    pub fn set_override(&mut self, addr: u32, f: Override) {
        self.overrides.insert(addr, f);
    }

    pub fn heap_alloc(&mut self, size: u32, zero: bool) -> u32 {
        let a = self.heap.alloc(size.max(1)).expect("guest heap exhausted");
        // Commit the block's full capacity: HeapReAlloc may grow in place up to it.
        let cap = (size.max(8) + 7) & !7;
        self.mem.commit(a, cap as usize).expect("commit heap");
        if zero {
            self.mem.slice_mut(a, size as usize).fill(0);
        }
        a
    }

    pub fn alloc_cstr(&mut self, s: &str) -> u32 {
        let a = self.heap_alloc(s.len() as u32 + 1, true);
        self.mem.write(a, s.as_bytes());
        a
    }

    /// Create a COM object in guest memory: [vtable ptr] followed by the vtable.
    pub fn create_com_object(&mut self, interface: u32, slots: u32) -> u32 {
        let obj = self.heap_alloc(8 + slots * 4, true);
        let vt = obj + 8;
        self.mem.w32(obj, vt);
        for s in 0..slots {
            self.mem.w32(vt + s * 4, COM_THUNK_BASE + interface * 0x1000 + s * 16);
        }
        obj
    }

    pub fn function_count() -> u32 {
        // SAFETY: provided by generated C.
        unsafe { recomp_function_count() }
    }
}

/// Call translated guest code at `addr` with the cdecl convention.
/// Verifies the guest stack is balanced on return.
pub fn call_guest(cpu: &mut Cpu, addr: u32, args: &[u32]) -> u32 {
    // SAFETY: lookup is a pure table search in generated code.
    let f = unsafe { recomp_lookup(addr) }.unwrap_or_else(|| fatal(&format!("call_guest: no translated function at {addr:#010x}")));
    let esp0 = cpu.esp;
    let mem = with_rt(|rt| {
        rt.diag.host_to_guest_calls += 1;
        rt.mem.base()
    });
    let _ = mem;
    for &a in args.iter().rev() {
        cpu.esp -= 4;
        with_rt(|rt| rt.mem.w32(cpu.esp, a));
    }
    cpu.esp -= 4;
    with_rt(|rt| rt.mem.w32(cpu.esp, HOST_RETURN));
    // SAFETY: f is a generated function taking a valid Cpu pointer.
    unsafe { f(cpu as *mut Cpu) };
    // cdecl: callee popped the return address; caller pops arguments.
    cpu.esp += 4 * args.len() as u32;
    if cpu.esp != esp0 {
        fatal(&format!("call_guest({addr:#010x}): stack imbalance, esp {:#x} expected {esp0:#x}", cpu.esp));
    }
    cpu.eax
}

/// Call translated guest code at `addr` with the stdcall convention (the callee
/// pops its arguments), e.g. an enumeration callback. Must not be called while
/// the runtime is borrowed through [`with_rt`]. Verifies the guest stack.
pub fn call_guest_stdcall(cpu: &mut Cpu, addr: u32, args: &[u32]) -> u32 {
    // SAFETY: lookup is a pure table search in generated code.
    let f = unsafe { recomp_lookup(addr) }.unwrap_or_else(|| fatal(&format!("call_guest_stdcall: no translated function at {addr:#010x}")));
    let esp0 = cpu.esp;
    with_rt(|rt| {
        rt.diag.host_to_guest_calls += 1;
        for &a in args.iter().rev() {
            cpu.esp -= 4;
            rt.mem.w32(cpu.esp, a);
        }
        cpu.esp -= 4;
        rt.mem.w32(cpu.esp, HOST_RETURN);
    });
    // SAFETY: f is a generated function taking a valid Cpu pointer.
    unsafe { f(cpu as *mut Cpu) };
    if cpu.esp != esp0 {
        fatal(&format!("call_guest_stdcall({addr:#010x}): stack imbalance, esp {:#x} expected {esp0:#x}", cpu.esp));
    }
    cpu.eax
}

pub fn fatal(msg: &str) -> ! {
    eprintln!("bugatron-runtime fatal: {msg}");
    std::process::exit(101)
}

// ---- symbols called from generated C ----------------------------------------------

#[no_mangle]
pub extern "C" fn rt_import(c: *mut Cpu, thunk: u32) {
    // SAFETY: generated code passes its live Cpu.
    let cpu = unsafe { &mut *c };
    if thunk >= COM_THUNK_BASE {
        imports::dispatch_com(cpu, thunk);
    } else {
        imports::dispatch_import(cpu, ((thunk - THUNK_BASE) / 16) as usize);
    }
}

#[no_mangle]
pub extern "C" fn rt_override(c: *mut Cpu, addr: u32) {
    let cpu = unsafe { &mut *c };
    // Take the override out so it can call back into the runtime and guest code.
    let mut f = with_rt(|rt| rt.overrides.remove(&addr))
        .unwrap_or_else(|| fatal(&format!("no override registered for {addr:#010x}")));
    f(cpu);
    with_rt(|rt| rt.overrides.insert(addr, f));
}

#[no_mangle]
pub extern "C" fn rt_unresolved_call(_c: *mut Cpu, target: u32, site: u32) {
    fatal(&format!("indirect call/jump at {site:#010x} to untranslated address {target:#010x}"));
}

#[no_mangle]
pub extern "C" fn rt_unimpl(_c: *mut Cpu, addr: u32, what: *const std::os::raw::c_char) {
    let s = unsafe { std::ffi::CStr::from_ptr(what) }.to_string_lossy();
    fatal(&format!("unimplemented instruction at {addr:#010x}: {s}"));
}

#[no_mangle]
pub extern "C" fn rt_bad_jump(_c: *mut Cpu, site: u32, target: u32) {
    fatal(&format!("switch at {site:#010x} jumped to unknown target {target:#010x}"));
}

/// An instruction the recompiler replaced with an explicit trap (see
/// `bugatron_recomp::targets`). Never approximated: execution stops.
#[no_mangle]
pub extern "C" fn rt_trap(_c: *mut Cpu, site: u32, reason: *const std::os::raw::c_char) {
    let s = unsafe { std::ffi::CStr::from_ptr(reason) }.to_string_lossy();
    fatal(&format!("trap at {site:#010x}: {s}"));
}

#[no_mangle]
pub extern "C" fn rt_div_error(_c: *mut Cpu, site: u32) {
    fatal(&format!("integer divide error (#DE) at {site:#010x}"));
}

/// Guest-to-guest indirect call (function pointer, vtable, or tail jump) reached
/// translated code. Recorded so tests can show real indirect dispatch.
#[no_mangle]
pub extern "C" fn rt_note_indirect(target: u32, site: u32) {
    RT.with(|r| {
        if let Ok(mut g) = r.try_borrow_mut() {
            if let Some(rt) = g.as_mut() {
                *rt.diag.guest_indirect_targets.entry(target).or_default() += 1;
                rt.diag.guest_indirect_sites.entry(site).or_default().insert(target);
            }
        }
    });
}

/// x87 behaviour the binary64 machine model cannot reproduce exactly
/// (precision control other than 53 bits, non-nearest rounding in arithmetic,
/// stack overflow, FPATAN via libm, 80-bit memory operands, and FSIN/FCOS/FPTAN
/// results whose 64-bit significand is not representable in binary64).
/// Recorded in `Cpu::x87_notes`; with `BUGATRON_X87_STRICT=1` the first
/// occurrence of each kind stops execution with a diagnostic.
#[no_mangle]
pub extern "C" fn rt_x87_note(c: *mut Cpu, kind: u32) {
    let cpu = unsafe { &mut *c };
    if cpu.x87_notes & kind == 0 && x87_strict() {
        fatal(&format!("x87 approximation (kind {kind:#x}: {}) under BUGATRON_X87_STRICT", x87_note_name(kind)));
    }
    cpu.x87_notes |= kind;
}

fn x87_strict() -> bool {
    static STRICT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *STRICT.get_or_init(|| std::env::var("BUGATRON_X87_STRICT").is_ok_and(|v| v == "1"))
}

// ---- x87 FSIN / FCOS / FPTAN (x87-compatible argument reduction) -----------------

const FSW_IE: u32 = 0x0001;
const FSW_PE: u32 = 0x0020;
const FSW_C1: u32 = 0x0200;
const FSW_C2: u32 = 0x0400;

fn x87_trig_insn(c: *mut Cpu, op: x87trig::TrigOp) {
    use x87trig::TrigOutcome;
    let cpu = unsafe { &mut *c };
    let top = (cpu.ftop & 7) as usize;
    if (cpu.ftag >> (2 * top)) & 3 == 3 {
        rt_x87_note(c, 8); // stack underflow: operand register is empty
    }
    let x = cpu.st[top];
    match x87trig::x87_trig(op, x) {
        TrigOutcome::OutOfRange => {
            // |x| >= 2^63: C2 = 1, operand unchanged, nothing pushed.
            cpu.fsw = (cpu.fsw & !FSW_C1) | FSW_C2;
            return;
        }
        TrigOutcome::Value { value, c1, ext_inexact, inexact } => {
            cpu.st[top] = value;
            cpu.fsw = (cpu.fsw & !(FSW_C1 | FSW_C2)) | if c1 { FSW_C1 } else { 0 } | if inexact { FSW_PE } else { 0 };
            if ext_inexact {
                rt_x87_note(c, 64);
            }
        }
        TrigOutcome::Invalid => {
            // Masked invalid operation (±∞): result is the default QNaN.
            cpu.st[top] = f64::from_bits(x87trig::INDEFINITE);
            cpu.fsw = (cpu.fsw & !(FSW_C1 | FSW_C2)) | FSW_IE;
        }
        TrigOutcome::Nan(v) => {
            cpu.st[top] = v;
            cpu.fsw &= !(FSW_C1 | FSW_C2);
        }
    }
    if op == x87trig::TrigOp::Tan {
        // FPTAN pushes 1.0 after a completed operation.
        cpu.ftop = (cpu.ftop.wrapping_sub(1)) & 7;
        let t = cpu.ftop as usize;
        if (cpu.ftag >> (2 * t)) & 3 != 3 {
            rt_x87_note(c, 8); // stack overflow
        }
        cpu.st[t] = 1.0;
        cpu.ftag &= !(3 << (2 * t));
    }
}

#[no_mangle]
pub extern "C" fn rt_x87_fsin(c: *mut Cpu) {
    x87_trig_insn(c, x87trig::TrigOp::Sin)
}
#[no_mangle]
pub extern "C" fn rt_x87_fcos(c: *mut Cpu) {
    x87_trig_insn(c, x87trig::TrigOp::Cos)
}
#[no_mangle]
pub extern "C" fn rt_x87_fptan(c: *mut Cpu) {
    x87_trig_insn(c, x87trig::TrigOp::Tan)
}

pub fn x87_note_name(kind: u32) -> &'static str {
    match kind {
        1 => "24-bit precision control",
        2 => "64-bit precision control (needs 80-bit arithmetic)",
        4 => "non-nearest rounding control in arithmetic",
        8 => "x87 register stack overflow",
        16 => "FPATAN (host libm result)",
        32 => "80-bit memory operand",
        64 => "FSIN/FCOS/FPTAN result held as binary64 instead of a 64-bit significand",
        _ => "unknown",
    }
}

/// 80-bit extended → binary64 (round to nearest even). Used only where the
/// original stores or loads a TBYTE; flagged so differences can be audited.
#[no_mangle]
pub extern "C" fn rt_f80_load(c: *mut Cpu, addr: u32) -> f64 {
    rt_x87_note(c, 32);
    let (mant, se) = with_rt(|rt| {
        let b = rt.mem.slice(addr, 10);
        (u64::from_le_bytes(b[0..8].try_into().unwrap()), u16::from_le_bytes([b[8], b[9]]))
    });
    f80_to_f64(mant, se)
}

#[no_mangle]
pub extern "C" fn rt_f80_store(c: *mut Cpu, addr: u32, v: f64) {
    rt_x87_note(c, 32);
    let (mant, se) = f64_to_f80(v);
    with_rt(|rt| {
        let b = rt.mem.slice_mut(addr, 10);
        b[0..8].copy_from_slice(&mant.to_le_bytes());
        b[8..10].copy_from_slice(&se.to_le_bytes());
    });
}

pub fn f80_to_f64(mant: u64, se: u16) -> f64 {
    let sign = if se & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = (se & 0x7fff) as i32;
    if exp == 0 && mant == 0 {
        return 0.0 * sign;
    }
    if exp == 0x7fff {
        return if mant << 1 == 0 { sign * f64::INFINITY } else { f64::NAN };
    }
    // value = mant * 2^(exp - 16383 - 63); mant → f64 rounds once (nearest even).
    let m = mant as f64;
    sign * m * 2f64.powi(exp - 16383 - 63)
}

pub fn f64_to_f80(v: f64) -> (u64, u16) {
    let bits = v.to_bits();
    let sign = ((bits >> 63) as u16) << 15;
    let exp = ((bits >> 52) & 0x7ff) as i32;
    let frac = bits & ((1u64 << 52) - 1);
    if exp == 0 && frac == 0 {
        return (0, sign);
    }
    if exp == 0x7ff {
        return (if frac == 0 { 1u64 << 63 } else { (1u64 << 63) | (frac << 11) }, sign | 0x7fff);
    }
    if exp == 0 {
        // subnormal: normalise
        let shift = frac.leading_zeros() - 11;
        let m = frac << (shift + 11);
        let e = 1 - 1023 - shift as i32 + 16383;
        return (m, sign | e as u16);
    }
    ((1u64 << 63) | (frac << 11), sign | (exp - 1023 + 16383) as u16)
}
