//! Native oracle for the instruction-semantics regression cases.
//!
//! Build for 32-bit Windows (`i686-pc-windows-gnu` or `i686-pc-windows-msvc`) and
//! run on a real x86 CPU. Each case's bytes (identical to what the recompiler
//! tests translate) are executed natively inside a small harness that sets up
//! registers, flags, the x87 control word and the scratch buffer, then records
//! registers, EFLAGS, the FNSAVE image and the scratch buffer.
//!
//! Usage: bugatron-x86-oracle.exe [output.json]
//! Prints JSON to stdout (and writes the file if given).

#[cfg(target_arch = "x86")]
fn main() {
    imp::run();
}

#[cfg(not(target_arch = "x86"))]
fn main() {
    eprintln!("bugatron-x86-oracle must be built for a 32-bit x86 target (Windows or Linux) and run on a real x86 CPU");
    std::process::exit(2);
}

#[cfg(target_arch = "x86")]
mod imp {
    use bugatron_x86_cases::iced::code_asm::*;
    use bugatron_x86_cases::{cases, R, SCRATCH_SIZE};

    /// 16 KiB of readable, writable, executable memory.
    #[cfg(windows)]
    fn alloc_rwx() -> *mut u8 {
        use windows_sys::Win32::System::Memory::{VirtualAlloc, MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE};
        unsafe { VirtualAlloc(std::ptr::null(), 0x4000, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) as *mut u8 }
    }
    #[cfg(unix)]
    fn alloc_rwx() -> *mut u8 {
        let p = unsafe {
            libc::mmap(std::ptr::null_mut(), 0x4000, libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, -1, 0)
        };
        if p == libc::MAP_FAILED { std::ptr::null_mut() } else { p as *mut u8 }
    }

    #[repr(C)]
    struct State {
        in_regs: [u32; 8],
        in_eflags: u32,
        fcw: u16,
        _pad: u16,
        saved_esp: u32,
        esp_before: u32,
        out_regs: [u32; 8],
        out_eflags: u32,
        fpu: [u8; 108],
    }

    fn cpu_brand() -> String {
        use core::arch::x86::__cpuid;
        let mut s = Vec::new();
        {
            if __cpuid(0x8000_0000).eax >= 0x8000_0004 {
                for leaf in 0x8000_0002u32..=0x8000_0004 {
                    let r = __cpuid(leaf);
                    for v in [r.eax, r.ebx, r.ecx, r.edx] {
                        s.extend_from_slice(&v.to_le_bytes());
                    }
                }
            }
        }
        String::from_utf8_lossy(&s).trim_matches(char::from(0)).trim().to_string()
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    pub fn run() {
        let mut out = String::new();
        out.push_str(&format!("{{\n  \"host_cpu\": \"{}\",\n  \"tool\": \"bugatron-x86-oracle {}\",\n  \"cases\": [\n", cpu_brand(), env!("CARGO_PKG_VERSION")));
        let all = cases();
        for (n, case) in all.iter().enumerate() {
            // SAFETY: State is plain old data; all-zero is a valid value.
            let mut st: Box<State> = Box::new(unsafe { std::mem::zeroed() });
            let mut scratch = vec![0u8; SCRATCH_SIZE].into_boxed_slice();
            for (off, b) in &case.mem {
                scratch[*off..*off + b.len()].copy_from_slice(b);
            }
            st.in_regs = case.regs;
            st.in_eflags = (case.eflags & 0x8d5) | 0x202;
            st.fcw = case.fcw;
            let s = &*st as *const State as u32;
            let scratch_addr = scratch.as_ptr() as u32;

            let code = alloc_rwx();
            assert!(!code.is_null(), "executable memory allocation failed");
            let snippet_va = code as u32 + 0x2000;
            let snippet = bugatron_x86_cases::assemble(case, snippet_va).expect("assemble case");

            let field = |off: u32| s + off;
            macro_rules! off { ($f:ident) => { std::mem::offset_of!(State, $f) as u32 } }
            let harness = (|| -> Result<Vec<u8>, bugatron_x86_cases::iced::IcedError> {
                let mut a = CodeAssembler::new(32)?;
                a.pushad()?;
                a.mov(dword_ptr(field(off!(saved_esp))), esp)?;
                a.fninit()?;
                a.fldcw(word_ptr(field(off!(fcw))))?;
                a.push(dword_ptr(field(off!(in_eflags))))?;
                a.popfd()?;
                a.mov(ecx, dword_ptr(field(off!(in_regs) + 4)))?;
                a.mov(edx, dword_ptr(field(off!(in_regs) + 8)))?;
                a.mov(ebx, scratch_addr)?;
                a.mov(ebp, dword_ptr(field(off!(in_regs) + 20)))?;
                a.mov(esi, dword_ptr(field(off!(in_regs) + 24)))?;
                a.mov(edi, dword_ptr(field(off!(in_regs) + 28)))?;
                a.mov(eax, dword_ptr(field(off!(in_regs))))?;
                a.mov(dword_ptr(field(off!(esp_before))), esp)?;
                a.call(snippet_va as u64)?;
                a.mov(dword_ptr(field(off!(out_regs))), eax)?;
                a.mov(dword_ptr(field(off!(out_regs) + 4)), ecx)?;
                a.mov(dword_ptr(field(off!(out_regs) + 8)), edx)?;
                a.mov(dword_ptr(field(off!(out_regs) + 12)), ebx)?;
                a.mov(dword_ptr(field(off!(out_regs) + 16)), esp)?;
                a.mov(dword_ptr(field(off!(out_regs) + 20)), ebp)?;
                a.mov(dword_ptr(field(off!(out_regs) + 24)), esi)?;
                a.mov(dword_ptr(field(off!(out_regs) + 28)), edi)?;
                a.pushfd()?;
                a.pop(dword_ptr(field(off!(out_eflags))))?;
                a.fnsave(ptr(field(off!(fpu))))?;
                a.fninit()?;
                a.mov(esp, dword_ptr(field(off!(saved_esp))))?;
                a.popad()?;
                a.ret()?;
                a.assemble(code as u64)
            })()
            .expect("assemble harness");
            unsafe {
                std::ptr::copy_nonoverlapping(harness.as_ptr(), code, harness.len());
                std::ptr::copy_nonoverlapping(snippet.as_ptr(), snippet_va as *mut u8, snippet.len());
                let f: extern "C" fn() = std::mem::transmute(code);
                f();
            }

            let mut regs = st.out_regs;
            regs[4] = regs[4].wrapping_sub(st.esp_before);
            for p in case.ptr_regs {
                regs[*p as usize] = regs[*p as usize].wrapping_sub(scratch_addr);
            }
            if !case.ptr_regs.contains(&R::Ebx) && regs[3] == scratch_addr {
                regs[3] = 0;
            }
            let fcw = u16::from_le_bytes([st.fpu[0], st.fpu[1]]);
            let fsw = u16::from_le_bytes([st.fpu[4], st.fpu[5]]);
            out.push_str(&format!(
                "    {{\"name\": \"{}\", \"regs\": [{}], \"eflags\": {}, \"fcw\": {}, \"fsw\": {}, \"mem\": \"{}\", \"fnsave\": \"{}\"}}{}\n",
                case.name,
                regs.iter().map(|r| r.to_string()).collect::<Vec<_>>().join(", "),
                st.out_eflags & 0x0cd5, // CF PF AF ZF SF DF OF
                fcw, fsw, hex(&scratch), hex(&st.fpu),
                if n + 1 < all.len() { "," } else { "" }
            ));
        }
        out.push_str("  ]\n}\n");
        print!("{out}");
        if let Some(path) = std::env::args().nth(1) {
            std::fs::write(&path, &out).expect("write output");
            eprintln!("wrote {path}");
        }
    }
}
