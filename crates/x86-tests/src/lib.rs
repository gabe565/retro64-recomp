//! Runs regression cases through the recompiled code and the runtime.

use bugatron_runtime::{call_guest, with_rt, Cpu, Runtime};
use bugatron_x86_cases::{Case, Outcome, SCRATCH_SIZE};

include!(concat!(env!("OUT_DIR"), "/case_addrs.rs"));

/// Guest address of the scratch buffer handed to every case in EBX.
pub const SCRATCH: u32 = 0x0060_0000;

/// Install a bare runtime (no executable image) for snippet tests.
pub fn install_runtime() {
    let rt = Runtime::bare().expect("bare runtime");
    rt.mem.commit(SCRATCH, SCRATCH_SIZE).expect("commit scratch");
    rt.install();
}

/// Run case `i` through the recompiled code and return its outcome, plus the
/// x87 notes the machine model raised.
pub fn run_model(i: usize, case: &Case) -> (Outcome, u32) {
    let mut cpu: Cpu = with_rt(|rt| rt.initial_cpu());
    let esp0 = cpu.esp;
    with_rt(|rt| {
        let m = rt.mem.slice_mut(SCRATCH, SCRATCH_SIZE);
        m.fill(0);
        for (off, b) in &case.mem {
            m[*off..*off + b.len()].copy_from_slice(b);
        }
    });
    let r = case.regs;
    cpu.eax = r[0];
    cpu.ecx = r[1];
    cpu.edx = r[2];
    cpu.ebx = SCRATCH;
    cpu.ebp = r[5];
    cpu.esi = r[6];
    cpu.edi = r[7];
    cpu.fl_kind = 0; // explicit flags
    cpu.fl_res = case.eflags & 0x8d5;
    cpu.df = 0;
    cpu.fcw = case.fcw as u32;
    call_guest(&mut cpu, CASE_ADDRS[i], &[]);
    // Read back architectural flags through the C model.
    let flags = model_flags(&cpu);
    let fsw = ((cpu.fsw & !0x3800) | ((cpu.ftop & 7) << 11)) as u16;
    let mem = with_rt(|rt| rt.mem.slice(SCRATCH, SCRATCH_SIZE).to_vec());
    let mut regs = [cpu.eax, cpu.ecx, cpu.edx, cpu.ebx, cpu.esp.wrapping_sub(esp0), cpu.ebp, cpu.esi, cpu.edi];
    for p in case.ptr_regs {
        regs[*p as usize] = regs[*p as usize].wrapping_sub(SCRATCH);
    }
    if !case.ptr_regs.contains(&bugatron_x86_cases::R::Ebx) && regs[3] == SCRATCH {
        regs[3] = 0; // EBX unchanged: report as scratch-relative 0
    }
    (Outcome { regs, eflags: flags | (cpu.df << 10), fcw: cpu.fcw as u16, fsw, mem }, cpu.x87_notes)
}

extern "C" {
    fn recomp_flags_get(c: *const Cpu) -> u32;
}

fn model_flags(cpu: &Cpu) -> u32 {
    // SAFETY: helper compiled from recomp.h in the generated library.
    unsafe { recomp_flags_get(cpu as *const Cpu) }
}
