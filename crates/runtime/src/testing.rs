//! Entry points for runtime regression tests (crates/runtime-tests).
//!
//! These drive imports and COM methods exactly as recompiled code does: the
//! arguments and a return address are pushed on the guest stack, the same
//! dispatcher runs, and the callee's stdcall stack cleanup is checked.

use crate::{with_rt, Cpu, HOST_RETURN};

fn push_frame(cpu: &mut Cpu, args: &[u32]) -> u32 {
    let esp0 = cpu.esp;
    with_rt(|rt| {
        for &a in args.iter().rev() {
            cpu.esp -= 4;
            rt.mem.w32(cpu.esp, a);
        }
        cpu.esp -= 4;
        rt.mem.w32(cpu.esp, HOST_RETURN);
    });
    esp0
}

fn check_frame(cpu: &Cpu, esp0: u32, what: &str) {
    assert_eq!(cpu.esp, esp0, "{what}: stdcall stack cleanup mismatch");
}

/// Call import `dll!name` (DLL name in lower case, e.g. "user32.dll") with stdcall `args`.
pub fn call_import(cpu: &mut Cpu, dll: &str, name: &str, args: &[u32]) -> u32 {
    let esp0 = push_frame(cpu, args);
    crate::imports::dispatch_named(cpu, dll, name);
    check_frame(cpu, esp0, name);
    cpu.eax
}

/// Call COM method `slot` on guest object `this` with `args` (excluding `this`).
pub fn call_com(cpu: &mut Cpu, this: u32, slot: u32, args: &[u32]) -> u32 {
    let thunk = with_rt(|rt| {
        let vt = rt.mem.r32(this);
        rt.mem.r32(vt + 4 * slot)
    });
    let mut full = vec![this];
    full.extend_from_slice(args);
    let esp0 = push_frame(cpu, &full);
    crate::imports::dispatch_com(cpu, thunk);
    check_frame(cpu, esp0, &format!("COM slot {slot}"));
    cpu.eax
}
