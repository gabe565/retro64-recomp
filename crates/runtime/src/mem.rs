//! Guest address space: one 4 GiB (+guard) reservation; guest address `a` lives
//! at `base + a`. Every u32 address is in bounds, so generated code needs no checks.

use std::sync::atomic::{AtomicPtr, Ordering};

pub const GUEST_SPACE: usize = 1 << 32;
pub const GUARD: usize = 1 << 16;

extern "C" {
    static mut recomp_mem: *mut u8;
}

static BASE: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());

pub struct GuestMemory {
    base: *mut u8,
}

unsafe impl Send for GuestMemory {}

impl GuestMemory {
    /// Reserve the address space and publish the base pointer to generated code.
    pub fn reserve() -> std::io::Result<Self> {
        let base = os::reserve(GUEST_SPACE + GUARD)?;
        BASE.store(base, Ordering::SeqCst);
        // SAFETY: single assignment before any generated code runs.
        unsafe { recomp_mem = base };
        Ok(GuestMemory { base })
    }

    pub fn base(&self) -> *mut u8 {
        self.base
    }

    /// Make [addr, addr+len) readable and writable (no-op where the OS commits lazily).
    pub fn commit(&self, addr: u32, len: usize) -> std::io::Result<()> {
        os::commit(self.base, addr as usize, len)
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
        // MAP_NORESERVE: pages are committed lazily on first touch (macOS and Linux).
        let p = unsafe {
            libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANON | libc::MAP_NORESERVE, -1, 0)
        };
        if p == libc::MAP_FAILED { Err(std::io::Error::last_os_error()) } else { Ok(p as *mut u8) }
    }
    pub fn commit(_base: *mut u8, _off: usize, _len: usize) -> std::io::Result<()> { Ok(()) }
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
