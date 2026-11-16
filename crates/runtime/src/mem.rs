//! Guest address space: one 4 GiB (+guard) reservation; guest address `a` lives
//! at `base + a`. Every u32 address is in bounds, so generated code needs no checks.
//!
//! The reservation is inaccessible until committed, on every OS, so a guest
//! access to memory the original process would not have had mapped faults
//! instead of silently reading zeros. A fault inside the reservation is
//! reported with its guest address (see `install_fault_handler`). The last
//! guest page and the guard are committed up front so unaligned accesses at
//! the top of the space behave like the original (wrap-free) reads.

use std::sync::atomic::{AtomicPtr, Ordering};

pub const GUEST_SPACE: usize = 1 << 32;
pub const GUARD: usize = 1 << 16;

extern "C" {
    static mut recomp_mem: *mut u8;
}

static BASE: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());

pub struct GuestMemory {
    base: *mut u8,
    /// Committed guest pages (4 KiB, as on Windows), one bit each, for
    /// IsBadReadPtr and friends. The OS may commit more (16 KiB host pages).
    committed: std::sync::Mutex<Vec<u64>>,
}

const PAGE: usize = 0x1000;

unsafe impl Send for GuestMemory {}

impl GuestMemory {
    /// Reserve the address space and publish the base pointer to generated code.
    pub fn reserve() -> std::io::Result<Self> {
        let base = os::reserve(GUEST_SPACE + GUARD)?;
        BASE.store(base, Ordering::SeqCst);
        // SAFETY: single assignment before any generated code runs.
        unsafe { recomp_mem = base };
        os::commit(base, GUEST_SPACE - 0x1000, 0x1000 + GUARD)?;
        install_fault_handler();
        let mem = GuestMemory { base, committed: std::sync::Mutex::new(vec![0; GUEST_SPACE / PAGE / 64]) };
        mem.mark(0xffff_f000, 0x1000);
        Ok(mem)
    }

    pub fn base(&self) -> *mut u8 {
        self.base
    }

    /// Make [addr, addr+len) readable and writable (no-op where the OS commits lazily).
    pub fn commit(&self, addr: u32, len: usize) -> std::io::Result<()> {
        os::commit(self.base, addr as usize, len)?;
        self.mark(addr, len);
        Ok(())
    }

    fn mark(&self, addr: u32, len: usize) {
        let mut bits = self.committed.lock().unwrap();
        for p in addr as usize / PAGE..(addr as usize + len).div_ceil(PAGE).min(GUEST_SPACE / PAGE) {
            bits[p / 64] |= 1 << (p % 64);
        }
    }

    /// Whether every byte of [addr, addr+len) is committed (len 0: true, as on Windows).
    pub fn is_committed(&self, addr: u32, len: usize) -> bool {
        if len == 0 {
            return true;
        }
        let end = addr as usize + len;
        if end > GUEST_SPACE {
            return false;
        }
        let bits = self.committed.lock().unwrap();
        (addr as usize / PAGE..end.div_ceil(PAGE)).all(|p| bits[p / 64] & (1 << (p % 64)) != 0)
    }

    /// Host pointer for a guest address. Valid for the life of the reservation.
    pub fn host_ptr(&self, addr: u32) -> *mut u8 {
        // SAFETY: base..base+4GiB+guard is reserved; addr < 4GiB.
        unsafe { self.base.add(addr as usize) }
    }

    pub fn slice(&self, addr: u32, len: usize) -> &[u8] {
        assert!(addr as usize + len <= GUEST_SPACE + GUARD);
        // SAFETY: in-bounds of the reservation; caller committed the range.
        unsafe { std::slice::from_raw_parts(self.host_ptr(addr), len) }
    }

    #[allow(clippy::mut_from_ref)]
    pub fn slice_mut(&self, addr: u32, len: usize) -> &mut [u8] {
        assert!(addr as usize + len <= GUEST_SPACE + GUARD);
        // SAFETY: as above; guest memory is only touched from the single guest thread.
        unsafe { std::slice::from_raw_parts_mut(self.host_ptr(addr), len) }
    }

    pub fn r8(&self, a: u32) -> u8 { self.slice(a, 1)[0] }
    pub fn r16(&self, a: u32) -> u16 { u16::from_le_bytes(self.slice(a, 2).try_into().unwrap()) }
    pub fn r32(&self, a: u32) -> u32 { u32::from_le_bytes(self.slice(a, 4).try_into().unwrap()) }
    pub fn w8(&self, a: u32, v: u8) { self.slice_mut(a, 1)[0] = v }
    pub fn w16(&self, a: u32, v: u16) { self.slice_mut(a, 2).copy_from_slice(&v.to_le_bytes()) }
    pub fn w32(&self, a: u32, v: u32) { self.slice_mut(a, 4).copy_from_slice(&v.to_le_bytes()) }
    pub fn write(&self, a: u32, bytes: &[u8]) { self.slice_mut(a, bytes.len()).copy_from_slice(bytes) }

    pub fn cstr(&self, a: u32) -> String {
        let mut out = Vec::new();
        let mut p = a;
        loop {
            let b = self.r8(p);
            if b == 0 { break; }
            out.push(b);
            p = p.wrapping_add(1);
        }
        String::from_utf8_lossy(&out).into_owned()
    }
}

impl Drop for GuestMemory {
    fn drop(&mut self) {
        os::release(self.base, GUEST_SPACE + GUARD);
    }
}

#[cfg(unix)]
mod os {
    pub fn reserve(len: usize) -> std::io::Result<*mut u8> {
        // Inaccessible until committed (same semantics as the Windows path).
        let p = unsafe {
            libc::mmap(std::ptr::null_mut(), len, libc::PROT_NONE,
                libc::MAP_PRIVATE | libc::MAP_ANON | libc::MAP_NORESERVE, -1, 0)
        };
        if p == libc::MAP_FAILED { Err(std::io::Error::last_os_error()) } else { Ok(p as *mut u8) }
    }
    pub fn commit(base: *mut u8, off: usize, len: usize) -> std::io::Result<()> {
        // Host page size: 16 KiB on Apple Silicon, 4 KiB on most x86-64/Linux.
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as usize;
        let start = off & !(page - 1);
        let end = (off + len + page - 1) & !(page - 1);
        let r = unsafe { libc::mprotect(base.add(start) as *mut _, end - start, libc::PROT_READ | libc::PROT_WRITE) };
        if r != 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
    }
    pub fn release(base: *mut u8, len: usize) {
        unsafe { libc::munmap(base as *mut _, len) };
    }
}

#[cfg(windows)]
mod os {
    use windows_sys::Win32::System::Memory::*;
    pub fn reserve(len: usize) -> std::io::Result<*mut u8> {
        let p = unsafe { VirtualAlloc(std::ptr::null(), len, MEM_RESERVE, PAGE_NOACCESS) };
        if p.is_null() { Err(std::io::Error::last_os_error()) } else { Ok(p as *mut u8) }
    }
    pub fn commit(base: *mut u8, off: usize, len: usize) -> std::io::Result<()> {
        let page = 0x1000usize;
        let start = off & !(page - 1);
        let end = (off + len + page - 1) & !(page - 1);
        let p = unsafe { VirtualAlloc(base.add(start) as *const _, end - start, MEM_COMMIT, PAGE_READWRITE) };
        if p.is_null() { Err(std::io::Error::last_os_error()) } else { Ok(()) }
    }
    pub fn release(base: *mut u8, _len: usize) {
        unsafe { VirtualFree(base as *mut _, 0, MEM_RELEASE) };
    }
}

/// Base and size of the reservation, for the fault handler.
fn guest_range() -> (usize, usize) {
    (BASE.load(Ordering::SeqCst) as usize, GUEST_SPACE + GUARD)
}

/// Report faults inside the guest reservation with the guest address, then exit
/// with status 102. Faults elsewhere are left to the default handling.
pub fn install_fault_handler() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(fault::install);
}

/// Format "bugatron-runtime fatal: guest memory fault at 0x........ (<what>)\n"
/// without allocating (usable from a signal handler).
fn fault_message(guest: u32, what: &[u8], out: &mut [u8; 128]) -> usize {
    let prefix = b"bugatron-runtime fatal: guest memory fault at 0x";
    let mut n = 0;
    for &b in prefix { out[n] = b; n += 1; }
    for i in (0..8).rev() {
        let d = ((guest >> (i * 4)) & 0xf) as u8;
        out[n] = if d < 10 { b'0' + d } else { b'a' + d - 10 };
        n += 1;
    }
    for &b in b" (" { out[n] = b; n += 1; }
    for &b in what.iter().take(40) { out[n] = b; n += 1; }
    for &b in b": address not committed in the guest space)\n".iter().take(128 - n) { out[n] = b; n += 1; }
    n
}

#[cfg(unix)]
mod fault {
    use super::{fault_message, guest_range};

    extern "C" fn on_fault(sig: libc::c_int, info: *mut libc::siginfo_t, _ctx: *mut libc::c_void) {
        #[cfg(target_os = "linux")]
        let addr = unsafe { (*info).si_addr() } as usize;
        #[cfg(not(target_os = "linux"))]
        let addr = unsafe { (*info).si_addr } as usize;
        let (base, len) = guest_range();
        if addr >= base && addr < base + len {
            let mut buf = [0u8; 128];
            let what: &[u8] = if sig == libc::SIGBUS { b"SIGBUS" } else { b"SIGSEGV" };
            let n = fault_message((addr - base) as u32, what, &mut buf);
            unsafe {
                libc::write(2, buf.as_ptr() as *const _, n);
                libc::_exit(102);
            }
        }
        // Not ours: restore the default action and return so the fault re-raises.
        unsafe {
            libc::signal(sig, libc::SIG_DFL);
        }
    }

    pub fn install() {
        for sig in [libc::SIGSEGV, libc::SIGBUS] {
            unsafe {
                let mut sa: libc::sigaction = std::mem::zeroed();
                sa.sa_sigaction = on_fault as *const () as usize;
                sa.sa_flags = libc::SA_SIGINFO;
                libc::sigemptyset(&mut sa.sa_mask);
                libc::sigaction(sig, &sa, std::ptr::null_mut());
            }
        }
    }
}

#[cfg(windows)]
mod fault {
    use super::{fault_message, guest_range};
    use windows_sys::Win32::System::Diagnostics::Debug::{AddVectoredExceptionHandler, EXCEPTION_POINTERS};

    unsafe extern "system" fn on_fault(info: *mut EXCEPTION_POINTERS) -> i32 {
        let rec = unsafe { &*(*info).ExceptionRecord };
        if rec.ExceptionCode as u32 == 0xC000_0005 && rec.NumberParameters >= 2 {
            let addr = rec.ExceptionInformation[1];
            let (base, len) = guest_range();
            if addr >= base && addr < base + len {
                let mut buf = [0u8; 128];
                let what: &[u8] = if rec.ExceptionInformation[0] == 1 { b"write" } else { b"read" };
                let n = fault_message((addr - base) as u32, what, &mut buf);
                eprint!("{}", String::from_utf8_lossy(&buf[..n]));
                std::process::exit(102);
            }
        }
        0 // EXCEPTION_CONTINUE_SEARCH
    }

    pub fn install() {
        unsafe { AddVectoredExceptionHandler(1, Some(on_fault)) };
    }
}
