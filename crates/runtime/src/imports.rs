//! Implementations of the Win32 imports the game actually calls.
//!
//! Only functions observed being called are implemented. Every other import in
//! the executable's import table resolves to a stub that stops the run with the
//! function's name, so the implemented surface grows from evidence.

use crate::{fatal, with_rt, Cpu, Runtime};

/// stdcall argument `i` (0-based) of the import being called.
fn arg(rt: &Runtime, cpu: &Cpu, i: u32) -> u32 {
    rt.mem.r32(cpu.esp + 4 + 4 * i)
}

/// Return from a stdcall import: pop return address and `nargs` arguments.
fn ret(cpu: &mut Cpu, value: u32, nargs: u32) {
    cpu.eax = value;
    cpu.esp += 4 + 4 * nargs;
}

const FAKE_STDIN: u32 = 0x100;
const FAKE_STDOUT: u32 = 0x101;
const FAKE_STDERR: u32 = 0x102;
const KERNEL32_HANDLE: u32 = 0x7c80_0000;
const DDRAW_HANDLE: u32 = 0x7380_0000;

pub(crate) fn dispatch_import(cpu: &mut Cpu, idx: usize) {
    let (dll, name) = with_rt(|rt| rt.import_names.get(idx).cloned().unwrap_or_else(|| fatal(&format!("bad import thunk {idx}"))));
    dispatch_named(cpu, &dll, &name);
}

/// Dispatch an import by DLL and function name (lower-case DLL name).
pub(crate) fn dispatch_named(cpu: &mut Cpu, dll: &str, name: &str) {
    with_rt(|rt| *rt.diag.import_calls.entry(format!("{dll}!{name}")).or_default() += 1);
    let (dll, name) = (dll.to_string(), name.to_string());
    // DDRAW imports may call back into guest code, so they run unborrowed.
    if dll.eq_ignore_ascii_case("ddraw.dll") && crate::ddraw::import(cpu, &name) {
        return;
    }
    if dll.eq_ignore_ascii_case("user32.dll") && crate::dialog::import(cpu, &name) {
        return;
    }
    if (dll.eq_ignore_ascii_case("user32.dll") || dll.eq_ignore_ascii_case("gdi32.dll")) && crate::user::import(cpu, &name) {
        return;
    }
    with_rt(|rt| call(rt, cpu, &dll, &name));
}

fn call(rt: &mut Runtime, cpu: &mut Cpu, dll: &str, name: &str) {
    macro_rules! a { ($i:expr) => { arg(rt, cpu, $i) }; }
    if dll.eq_ignore_ascii_case("fmod.dll") {
        let args: Vec<u32> = (0..4).map(|i| arg(rt, cpu, i)).collect();
        if let Some((v, n)) = crate::fmod::call(rt, name, &|i| args[i as usize]) {
            ret(cpu, v, n);
            return;
        }
    }
    match (dll, name) {
        // ---- process / module -------------------------------------------------
        ("kernel32.dll", "GetVersion") => ret(cpu, 0x0a28_0105, 0), // Windows XP SP-era 5.1.2600
        ("kernel32.dll", "GetCommandLineA") => ret(cpu, rt.command_line, 0),
        ("kernel32.dll", "GetStartupInfoA") => {
            let p = a!(0);
            rt.mem.slice_mut(p, 68).fill(0);
            rt.mem.w32(p, 68);
            ret(cpu, 0, 1);
        }
        ("kernel32.dll", "GetModuleHandleA") => {
            let p = a!(0);
            let h = if p == 0 {
                rt.pe.image_base
            } else if rt.mem.cstr(p).eq_ignore_ascii_case("kernel32") || rt.mem.cstr(p).eq_ignore_ascii_case("kernel32.dll") {
                KERNEL32_HANDLE
            } else if rt.mem.cstr(p).eq_ignore_ascii_case("ddraw.dll") {
                DDRAW_HANDLE // loaded through the import table, as on Windows
            } else {
                0
            };
            ret(cpu, h, 1);
        }
        ("kernel32.dll", "GetModuleFileNameA") => {
            let (h, buf, size) = (a!(0), a!(1), a!(2));
            let _ = h;
            let mut s = rt.module_path.clone().into_bytes();
            s.truncate(size.saturating_sub(1) as usize);
            let n = s.len() as u32;
            rt.mem.write(buf, &s);
            rt.mem.w8(buf + n, 0);
            ret(cpu, n, 3);
        }
        ("kernel32.dll", "GetProcAddress") => {
            // Nothing optional is provided. For DDRAW!DirectDrawEnumerateExA this
            // selects the game's own DirectDrawEnumerateA fallback (0x428a90), as on
            // pre-DirectX-6 systems; both report the same primary display driver.
            ret(cpu, 0, 2)
        }
        ("kernel32.dll", "GetCurrentThreadId") => ret(cpu, 1, 0),
        ("kernel32.dll", "GetCurrentProcess") => ret(cpu, 0xffff_ffff, 0),
        ("kernel32.dll", "GetCurrentThread") => ret(cpu, 0xffff_fffe, 0),
        ("kernel32.dll", "GetLastError") => ret(cpu, rt.last_error, 0),
        ("kernel32.dll", "SetLastError") => { rt.last_error = a!(0); ret(cpu, 0, 1); }
        ("kernel32.dll", "SetUnhandledExceptionFilter") => ret(cpu, 0, 1),
        ("kernel32.dll", "ExitProcess") => {
            let code = a!(0);
            let status = match rt.exit_hook.as_mut() { Some(h) => h(code), None => code as i32 };
            std::process::exit(status);
        }
        // ---- user interface -----------------------------------------------------
        ("user32.dll", "MessageBoxA") => {
            // Shown on stderr for now; MB_OK-style boxes are all the game uses.
            let (text, caption) = (rt.mem.cstr(a!(1)), rt.mem.cstr(a!(2)));
            eprintln!("[MessageBoxA] {caption}: {text}");
            ret(cpu, 1, 4); // IDOK
        }
        // ---- environment ------------------------------------------------------
        ("kernel32.dll", "GetEnvironmentStrings") | ("kernel32.dll", "GetEnvironmentStringsW") => ret(cpu, rt.env_block, 0),
        ("kernel32.dll", "FreeEnvironmentStringsA") | ("kernel32.dll", "FreeEnvironmentStringsW") => ret(cpu, 1, 1),
        // ---- code pages and strings ----------------------------------------------
        ("kernel32.dll", "GetACP") => ret(cpu, 1252, 0),
        ("kernel32.dll", "GetOEMCP") => ret(cpu, 437, 0),
        ("kernel32.dll", "GetCPInfo") => {
            let p = a!(1);
            rt.mem.slice_mut(p, 20).fill(0);
            rt.mem.w32(p, 1); // MaxCharSize
            rt.mem.w8(p + 4, b'?');
            ret(cpu, 1, 2);
        }
        ("kernel32.dll", "MultiByteToWideChar") => {
            let (src, srclen, dst, dstlen) = (a!(2), a!(3), a!(4), a!(5));
            let bytes: Vec<u8> = if srclen == 0xffff_ffff {
                let mut v = rt.mem.cstr(src).into_bytes();
                v.push(0);
                v
            } else {
                rt.mem.slice(src, srclen as usize).to_vec()
            };
            if dstlen != 0 {
                if (dstlen as usize) < bytes.len() { rt.last_error = 122; ret(cpu, 0, 6); return; }
                for (i, &b) in bytes.iter().enumerate() {
                    rt.mem.w16(dst + 2 * i as u32, b as u16); // Latin-1 / cp1252 subset
                }
            }
            ret(cpu, bytes.len() as u32, 6);
        }
        ("kernel32.dll", "WideCharToMultiByte") => {
            let (src, srclen, dst, dstlen) = (a!(2), a!(3), a!(4), a!(5));
            let mut chars = Vec::new();
            let mut i = 0;
            loop {
                if srclen != 0xffff_ffff && i >= srclen { break; }
                let ch = rt.mem.r16(src + 2 * i);
                chars.push(if ch < 256 { ch as u8 } else { b'?' });
                i += 1;
                if srclen == 0xffff_ffff && ch == 0 { break; }
            }
            if dstlen != 0 {
                if (dstlen as usize) < chars.len() { rt.last_error = 122; ret(cpu, 0, 8); return; }
                rt.mem.write(dst, &chars);
            }
            ret(cpu, chars.len() as u32, 8);
        }
        ("kernel32.dll", "GetStringTypeW") => {
            // (dwInfoType, lpSrcStr, cchSrc, lpCharType) — CT_CTYPE1 only.
            let (info, src, cch, out) = (a!(0), a!(1), a!(2), a!(3));
            if info != 1 { ret(cpu, 0, 4); return; }
            let n = if cch == 0xffff_ffff { (0..).take_while(|&i| rt.mem.r16(src + 2 * i) != 0).count() as u32 + 1 } else { cch };
            for i in 0..n {
                let ch = rt.mem.r16(src + 2 * i);
                rt.mem.w16(out + 2 * i, ctype1(ch));
            }
            ret(cpu, 1, 4);
        }
        ("kernel32.dll", "GetStringTypeA") => {
            // (Locale, dwInfoType, lpSrcStr, cchSrc, lpCharType)
            let (info, src, cch, out) = (a!(1), a!(2), a!(3), a!(4));
            if info != 1 { ret(cpu, 0, 5); return; }
            let n = if cch == 0xffff_ffff { rt.mem.cstr(src).len() as u32 + 1 } else { cch };
            for i in 0..n {
                let ch = rt.mem.r8(src + i) as u16;
                rt.mem.w16(out + 2 * i, ctype1(ch));
            }
            ret(cpu, 1, 5);
        }
        ("kernel32.dll", "LCMapStringW") | ("kernel32.dll", "LCMapStringA") => {
            // (Locale, dwMapFlags, lpSrcStr, cchSrc, lpDestStr, cchDest); LCMAP_LOWERCASE/UPPERCASE only.
            let wide = name.ends_with('W');
            let (flags, src, cch, dst, dcch) = (a!(1), a!(2), a!(3), a!(4), a!(5));
            let unit = if wide { 2 } else { 1 };
            let read = |i: u32| if wide { rt.mem.r16(src + 2 * i) } else { rt.mem.r8(src + i) as u16 };
            let n = if cch == 0xffff_ffff { (0..).take_while(|&i| read(i) != 0).count() as u32 + 1 } else { cch };
            if flags & !0x0300 != 0 { fatal(&format!("{name}: unsupported flags {flags:#x}")); }
            if dcch == 0 { ret(cpu, n, 6); return; }
            if dcch < n { rt.last_error = 122; ret(cpu, 0, 6); return; }
            let vals: Vec<u16> = (0..n).map(|i| {
                let ch = read(i);
                if ch < 128 {
                    let b = ch as u8;
                    (if flags & 0x100 != 0 { b.to_ascii_lowercase() } else if flags & 0x200 != 0 { b.to_ascii_uppercase() } else { b }) as u16
                } else { ch }
            }).collect();
            for (i, v) in vals.iter().enumerate() {
                if wide { rt.mem.w16(dst + 2 * i as u32, *v) } else { rt.mem.w8(dst + i as u32, *v as u8) }
            }
            let _ = unit;
            ret(cpu, n, 6);
        }
        // ---- heap -------------------------------------------------------------
        ("kernel32.dll", "HeapCreate") => ret(cpu, 0x0050_0000, 3),
        ("kernel32.dll", "HeapDestroy") => ret(cpu, 1, 1),
        ("kernel32.dll", "HeapAlloc") => {
            let (flags, size) = (a!(1), a!(2));
            let p = rt.heap_alloc(size, flags & 0x8 != 0);
            ret(cpu, p, 3);
        }
        ("kernel32.dll", "HeapFree") => {
            let ok = rt.heap.free(a!(2));
            ret(cpu, ok as u32, 3);
        }
        ("kernel32.dll", "HeapSize") => {
            let s = rt.heap.size(a!(2)).unwrap_or(0xffff_ffff);
            ret(cpu, s, 3);
        }
        ("kernel32.dll", "HeapReAlloc") => {
            let (flags, p, size) = (a!(1), a!(2), a!(3));
            let old = rt.heap.size(p).unwrap_or_else(|| fatal(&format!("HeapReAlloc of unknown block {p:#x}")));
            if rt.heap.resize_in_place(p, size) {
                if flags & 0x8 != 0 && size > old {
                    rt.mem.slice_mut(p + old, (size - old) as usize).fill(0);
                }
                ret(cpu, p, 4);
            } else if flags & 0x10 != 0 {
                ret(cpu, 0, 4); // HEAP_REALLOC_IN_PLACE_ONLY
            } else {
                let n = rt.heap_alloc(size, flags & 0x8 != 0);
                let data = rt.mem.slice(p, old.min(size) as usize).to_vec();
                rt.mem.write(n, &data);
                rt.heap.free(p);
                ret(cpu, n, 4);
            }
        }
        ("kernel32.dll", "VirtualAlloc") => {
            let (addr, size, ty) = (a!(0), a!(1), a!(2));
            let r = if addr == 0 {
                rt.valloc.reserve(size).unwrap_or(0)
            } else if rt.valloc.contains(addr, size) {
                addr & !0xfff
            } else {
                0
            };
            if r != 0 && ty & 0x1000 != 0 {
                rt.mem.commit(r, size as usize).expect("commit");
            }
            ret(cpu, r, 4);
        }
        ("kernel32.dll", "VirtualFree") => {
            let (addr, size, ty) = (a!(0), a!(1), a!(2));
            if ty & 0x4000 != 0 && size != 0 {
                // MEM_DECOMMIT: recommitted pages read as zero.
                rt.mem.slice_mut(addr, size as usize).fill(0);
            }
            ret(cpu, 1, 3);
        }
        // ---- thread-local storage / sync ---------------------------------------
        ("kernel32.dll", "TlsAlloc") => { rt.tls.push(0); ret(cpu, rt.tls.len() as u32 - 1, 0); }
        ("kernel32.dll", "TlsSetValue") => {
            let i = a!(0) as usize;
            if i < rt.tls.len() { rt.tls[i] = a!(1); ret(cpu, 1, 2) } else { ret(cpu, 0, 2) }
        }
        ("kernel32.dll", "TlsGetValue") => {
            let v = rt.tls.get(a!(0) as usize).copied().unwrap_or(0);
            rt.last_error = 0;
            ret(cpu, v, 1);
        }
        ("kernel32.dll", "InitializeCriticalSection") | ("kernel32.dll", "DeleteCriticalSection")
        | ("kernel32.dll", "EnterCriticalSection") | ("kernel32.dll", "LeaveCriticalSection") => ret(cpu, 0, 1),
        ("kernel32.dll", "InterlockedIncrement") | ("kernel32.dll", "InterlockedDecrement") => {
            let p = a!(0);
            let v = if name == "InterlockedIncrement" { rt.mem.r32(p).wrapping_add(1) } else { rt.mem.r32(p).wrapping_sub(1) };
            rt.mem.w32(p, v);
            ret(cpu, v, 1);
        }
        // ---- standard handles ---------------------------------------------------
        ("kernel32.dll", "GetStdHandle") => {
            let h = match a!(0) { 0xffff_fff6 => FAKE_STDIN, 0xffff_fff5 => FAKE_STDOUT, 0xffff_fff4 => FAKE_STDERR, _ => 0xffff_ffff };
            ret(cpu, h, 1);
        }
        ("kernel32.dll", "GetFileType") => {
            let h = a!(0);
            let t = if matches!(h, FAKE_STDIN | FAKE_STDOUT | FAKE_STDERR) { 2 } else if rt.files.is_file(h) { 1 } else { 0 };
            ret(cpu, t, 1);
        }
        ("kernel32.dll", "SetHandleCount") => ret(cpu, a!(0), 1),
        // ---- files ----------------------------------------------------------------
        ("kernel32.dll", "CreateFileA") => {
            let (name, access, disp) = (rt.mem.cstr(a!(0)), a!(1), a!(4));
            match rt.files.create(&name, access, disp) {
                Ok(h) => { rt.last_error = 0; ret(cpu, h, 7) }
                Err(e) => { rt.last_error = e; ret(cpu, crate::files::INVALID_HANDLE_VALUE, 7) }
            }
        }
        ("kernel32.dll", "FindFirstFileA") => {
            let (pat, out) = (rt.mem.cstr(a!(0)), a!(1));
            match rt.files.find_first(&pat) {
                Ok((h, e)) => { write_find_data(rt, out, &e); ret(cpu, h, 2) }
                Err(e) => { rt.last_error = e; ret(cpu, crate::files::INVALID_HANDLE_VALUE, 2) }
            }
        }
        ("kernel32.dll", "FindNextFileA") => {
            let (h, out) = (a!(0), a!(1));
            match rt.files.find_next(h) {
                Some(Some(e)) => { write_find_data(rt, out, &e); ret(cpu, 1, 2) }
                Some(None) => { rt.last_error = 18; ret(cpu, 0, 2) } // ERROR_NO_MORE_FILES
                None => { rt.last_error = 6; ret(cpu, 0, 2) }
            }
        }
        ("kernel32.dll", "FindClose") => { let ok = rt.files.find_close(a!(0)); ret(cpu, ok as u32, 1) }
        ("kernel32.dll", "SetCurrentDirectoryA") => {
            let path = rt.mem.cstr(a!(0));
            match rt.files.set_cwd(&path) {
                Ok(()) => ret(cpu, 1, 1),
                Err(e) => { rt.last_error = e; ret(cpu, 0, 1) }
            }
        }
        ("kernel32.dll", "GetCurrentDirectoryA") => {
            let (size, buf) = (a!(0), a!(1));
            let p = rt.files.cwd_path();
            let n = p.len() as u32;
            if size > n {
                rt.mem.write(buf, p.as_bytes());
                rt.mem.w8(buf + n, 0);
                ret(cpu, n, 2);
            } else {
                ret(cpu, n + 1, 2); // required size including the terminator
            }
        }
        ("kernel32.dll", "ReadFile") => {
            let (h, buf, n, read) = (a!(0), a!(1), a!(2), a!(3));
            let mut tmp = vec![0u8; n as usize];
            match rt.files.read(h, &mut tmp) {
                Some(k) => {
                    rt.mem.write(buf, &tmp[..k]);
                    if read != 0 { rt.mem.w32(read, k as u32); }
                    ret(cpu, 1, 5);
                }
                None => { rt.last_error = 6; ret(cpu, 0, 5) } // ERROR_INVALID_HANDLE
            }
        }
        ("kernel32.dll", "SetFilePointer") => {
            let (h, dist, high, method) = (a!(0), a!(1), a!(2), a!(3));
            if high != 0 && rt.mem.r32(high) != if (dist as i32) < 0 { 0xffff_ffff } else { 0 } {
                fatal("SetFilePointer: 64-bit distances are not used by the game");
            }
            match rt.files.seek(h, dist as i32, method) {
                Some(p) => {
                    if high != 0 { rt.mem.w32(high, (p >> 32) as u32); }
                    ret(cpu, p as u32, 4)
                }
                None => { rt.last_error = 131; ret(cpu, 0xffff_ffff, 4) } // ERROR_NEGATIVE_SEEK
            }
        }
        ("kernel32.dll", "GetFileSize") => {
            let (h, high) = (a!(0), a!(1));
            let s = rt.files.size(h).unwrap_or(0xffff_ffff);
            if high != 0 { rt.mem.w32(high, (s >> 32) as u32); }
            ret(cpu, s as u32, 2);
        }
        ("kernel32.dll", "CloseHandle") => {
            let ok = rt.files.close(a!(0));
            ret(cpu, ok as u32, 1);
        }
        ("kernel32.dll", "WriteFile") if rt.files.is_file(a!(0)) => {
            let (h, buf, n, written) = (a!(0), a!(1), a!(2), a!(3));
            let data = rt.mem.slice(buf, n as usize).to_vec();
            let k = rt.files.write(h, &data).unwrap_or(0);
            if written != 0 { rt.mem.w32(written, k as u32); }
            ret(cpu, (k == n as usize) as u32, 5);
        }
        ("kernel32.dll", "WriteFile") => {
            let (h, buf, n, written) = (a!(0), a!(1), a!(2), a!(3));
            let text = String::from_utf8_lossy(rt.mem.slice(buf, n as usize)).into_owned();
            if h == FAKE_STDOUT { print!("{text}") } else { eprint!("[guest] {text}") }
            if written != 0 { rt.mem.w32(written, n); }
            ret(cpu, 1, 5);
        }
        // ---- time ---------------------------------------------------------------
        ("kernel32.dll", "QueryPerformanceFrequency") => {
            let p = a!(0);
            let f = rt.clock.qpc_frequency;
            rt.mem.w32(p, f as u32);
            rt.mem.w32(p + 4, (f >> 32) as u32);
            ret(cpu, 1, 1);
        }
        ("kernel32.dll", "QueryPerformanceCounter") => {
            let p = a!(0);
            let freq = rt.clock.qpc_frequency;
            let v = match (rt.clock.qpc_queue.pop_front(), rt.clock.cadence.as_mut()) {
                (Some(v), _) => v,
                (None, Some(c)) => c.read_ticks(freq),
                (None, None) => { rt.clock.qpc_underflow = true; 0 }
            };
            rt.mem.w32(p, v as u32);
            rt.mem.w32(p + 4, (v >> 32) as u32);
            ret(cpu, 1, 1);
        }
        ("kernel32.dll", "Sleep") => {
            rt.clock.sleeps += 1;
            let ms = a!(0);
            if let Some(c) = rt.clock.cadence.as_mut() {
                c.sleep(ms);
                if ms > 0 {
                    crate::clock::pace(rt);
                }
            }
            ret(cpu, 0, 1);
        }
        ("winmm.dll", "timeGetTime") => ret(cpu, rt.clock_ms(), 0),
        // No joystick is attached (controller support is a later enhancement).
        ("winmm.dll", "joyGetPos") => ret(cpu, 167, 2), // JOYERR_UNPLUGGED
        // The timer-calibration code (0x41c180) raises its priority; scheduling is not emulated.
        ("kernel32.dll", "SetPriorityClass") => ret(cpu, 1, 2),
        ("kernel32.dll", "SetThreadPriority") => ret(cpu, 1, 2),
        ("kernel32.dll", "OutputDebugStringA") => {
            eprintln!("[debug] {}", rt.mem.cstr(a!(0)).trim_end());
            ret(cpu, 0, 1);
        }
        ("kernel32.dll", "RaiseException") | ("kernel32.dll", "RtlUnwind") | ("kernel32.dll", "UnhandledExceptionFilter") => {
            fatal(&format!("exception dispatch is not supported: {name} called (C++ throw, SEH raise, or unhandled exception); see docs/recompilation-feasibility.md D8"))
        }
        _ => fatal(&format!("unimplemented import {dll}!{name} (called from {:#010x}, esp={:#x}); implement it in crates/runtime/src/imports.rs only if the original game calls it", rt.mem.r32(cpu.esp), cpu.esp)),
    }
}

// ---- COM ---------------------------------------------------------------------------

pub(crate) fn dispatch_com(cpu: &mut Cpu, thunk: u32) {
    let off = thunk - crate::COM_THUNK_BASE;
    crate::ddraw::dispatch(cpu, off / 0x1000, (off % 0x1000) / 16);
}

/// Fill a `WIN32_FIND_DATAA` (0x140 bytes). Timestamps are zero.
fn write_find_data(rt: &Runtime, p: u32, e: &crate::files::FindEntry) {
    rt.mem.slice_mut(p, 0x140).fill(0);
    rt.mem.w32(p, if e.dir { 0x10 } else { 0x20 }); // FILE_ATTRIBUTE_DIRECTORY / ARCHIVE
    rt.mem.w32(p + 0x1c, (e.size >> 32) as u32);
    rt.mem.w32(p + 0x20, e.size as u32);
    let n = e.name.as_bytes();
    let n = &n[..n.len().min(259)];
    rt.mem.write(p + 0x2c, n);
    if n.len() <= 12 {
        rt.mem.write(p + 0x130, &n.to_ascii_uppercase());
    }
}

/// CT_CTYPE1 classification for the "C" locale (ASCII); other code units get 0.
fn ctype1(ch: u16) -> u16 {
    const UPPER: u16 = 0x1; const LOWER: u16 = 0x2; const DIGIT: u16 = 0x4; const SPACE: u16 = 0x8;
    const PUNCT: u16 = 0x10; const CNTRL: u16 = 0x20; const BLANK: u16 = 0x40; const XDIGIT: u16 = 0x80; const ALPHA: u16 = 0x100;
    if ch >= 128 { return 0; }
    let b = ch as u8;
    let mut t = 0;
    if b.is_ascii_uppercase() { t |= UPPER | ALPHA; }
    if b.is_ascii_lowercase() { t |= LOWER | ALPHA; }
    if b.is_ascii_digit() { t |= DIGIT; }
    if b.is_ascii_hexdigit() { t |= XDIGIT; }
    if matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') { t |= SPACE; }
    if b == b' ' || b == b'\t' { t |= BLANK; }
    if b.is_ascii_punctuation() { t |= PUNCT; }
    if b.is_ascii_control() { t |= CNTRL; }
    t
}
