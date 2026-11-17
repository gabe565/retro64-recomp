# Static recompilation feasibility — Bugatron Gold v1.20a

Date: 2026-11-15. Target: `Bug3D.exe`, SHA-256 `be581a63…758919e`.

Confidence labels:

- **[V] Verified**: from code, data, or runtime observation.
- **[S] Survey**: counted by `tools/ghidra/RecompSurvey.java` over Ghidra's analysis. The counts depend on Ghidra's function and flow discovery.
- **[H] Hypothesis**.

## 1. Verdict

**Static recompilation of `Bug3D.exe` is feasible, and the binary is an unusually good candidate.**

- It is small. There are about 65,000 instructions, 212 KB of `.text`, and 678 functions [S].
- It is compiled by MSVC 6 with no packing, obfuscation, or self-modifying code. `.text` is read-and-execute only (characteristics `0x60000020`) [V].
- It uses no MMX or SSE. The instruction mix is plain integer code plus x87 [S].
- It is single-threaded. `Bug3D.exe` imports no thread-creation API [V].
- Control flow is almost entirely static. Of 69 indirect jumps, 65 are resolved switch tables. The 4 unresolved ones are C-runtime exception helpers [S].
- Game code has essentially no function pointers. Its indirect calls are COM method calls into DirectDraw and Direct3D, or Windows API pointers cached in registers [V, by sampling].
- The platform surface is bounded: 120 imported functions, about 28 distinct COM method slots, and 13 FMOD functions [V/S].

The hard parts are the x87 floating-point model, the DirectDraw/Direct3D 6 replacement, and validation. None of them blocks the approach.

## 2. Code inventory [S]

| Region | Address range | Functions | Bytes | Treatment |
|---|---|---|---|---|
| Game logic | 0x401000–0x41e94f | 276 | 107,957 | Recompile. Never override. |
| Direct3D wrapper (`r3d_*`) | 0x41e950–0x42a3ff | 90 | 45,439 | Recompile, or override at the `r3d_*` API (see §4.7) |
| MSVC 6 C runtime + import thunks | 0x42a400–0x434fff | 312 | 40,342 | Recompile. Its Win32 calls go to the runtime. |

| Measure | Count |
|---|---|
| Instructions in `.text` | 65,134 |
| x87 instructions | 4,739, in 113 functions |
| `fldcw` (control-word writes) | 4 |
| `fs:` accesses (SEH chain, thread data) | 98 |
| Indirect jumps | 69, of which 65 resolved switch tables |
| Indirect calls | 472: 274 through import thunks, 198 others |
| Function addresses taken as values | 44: window procedure, enumeration callbacks, SEH funclets, C-runtime handlers |
| `.text` bytes outside any Ghidra function | 19,254: 7,410 decode as instructions, 1,098 are defined data, the rest is padding or alignment. Needs review. |

The instruction vocabulary is small: about 100 distinct mnemonics. They include `MOV`, `PUSH`, `CMP`, `Jcc`, `SBB`/`ADC` (MSVC boolean idioms), `REP MOVS`/`STOS`, `REPNE SCASB` (inlined `strlen`/`strcpy`), `IMUL`/`IDIV`, and the x87 set. The x87 set includes `FSIN` (22 uses), `FCOS` (7), `FSQRT` (8), `FPATAN` (3), `FPTAN` (1), and `FRNDINT` (1).

## 3. Proposed architecture

```
user's Bug3D.exe ──► recompiler (Rust) ──► generated code (C, per function)  ─┐
       ▲                ▲                                                     ├─► native executable
       │          analysis inputs:                runtime (Rust) ─────────────┘   (Windows/macOS/Linux)
   hash check     functions, jump tables,           • guest memory + loader
                  callbacks (from Ghidra export,    • x87 / flags helpers
                  checked by the recompiler)        • Win32 subset, DDraw/D3D6 or r3d, FMOD
                                                    • SDL3 window/GPU/input/audio, libxmp
```

- **Decoder**: the `iced-x86` crate, which decodes the full x86-32 instruction set.
- **Generated code: C, recommended.** Lifted control flow is irreducible `goto` soup, and the largest function, `FUN_00403730`, is about 3,000 instructions. C expresses `goto` directly and compiles hundreds of thousands of lines quickly through the `cc` crate. Generated Rust would need loop-plus-match state machines, with much slower compiles and worse code. `AGENTS.md` allows either; the recompiler and runtime stay in Rust.
- **Execution model.** Every guest function becomes `void f_XXXXXXXX(Cpu *cpu)`.
  - `Cpu` holds the eight general-purpose registers, lazily computed flags, the x87 register stack and its tag, control, and status words, and the guest memory base.
  - Direct calls become native calls.
  - An indirect call goes through `call_indirect(addr)`. That is a table lookup from guest address to native function. Guest addresses inside the runtime's thunk range dispatch to Rust implementations.

## 4. Major technical challenges

### 4.1 Guest memory model

The game is a 32-bit program running on 64-bit hosts. The plan:

- Reserve a 4 GiB virtual region per process and access guest memory as `base + (u32)addr`. All three operating systems allow this, and it is how WebAssembly engines do it. No bounds checks are needed, because every u32 offset lands inside the reservation.
- Map the user's executable sections at their original addresses, image base `0x400000`. `.data` alone is 6 MB, mostly zero-initialized.
- Place the guest stack, the heap behind `HeapAlloc`/`VirtualAlloc`, and a fake thread information block inside the region.
- Host pointers never enter guest memory. The runtime copies or translates structures such as `DDSURFACEDESC2`, `MSG`, and `WIN32_FIND_DATA` at the API boundary.
- Runtime evidence supports fixed addresses: the image loads at `0x400000` with no relocation [V].

### 4.2 Code discovery and control flow

- Function boundaries, jump tables, and the callback list come from a Ghidra export. The recompiler then runs its own recursive-descent pass to confirm each function and flag disagreements.
- The 4 unresolved indirect jumps (`0x42a76b`, `0x42a779`, `0x42a780`, `0x42a9a0`) are in C-runtime exception-unwinding helpers (`__local_unwind2`, `__NLG_*`, `_CallSettingFrame`). Each needs a hand-written runtime implementation of the helper's semantics, or a generic `jump_indirect` to recompiled code.
- The 7.4 KB of instruction-like bytes outside functions must be classified as reachable code that Ghidra missed, or dead code.
- Host-to-guest callbacks need native entry points: the window procedure, the video-dialog procedure `0x408280`, DirectDraw enumeration callbacks, texture-format enumeration, and SEH funclets. All 44 taken addresses plus code labels passed as callbacks go into the dispatch table.

### 4.3 x87 floating point (highest fidelity risk)

- **Volume and model.** 4,739 x87 instructions in 113 functions. Generated code must model the 8-register stack, its top-of-stack pointer, and its tag word exactly, including `FXCH` (459 uses) and pop variants.
- **Precision control decides the arithmetic.** At 53-bit precision (the MSVC CRT default [H for this binary]), each `+ − × ÷ √` result is rounded to a double mantissa, so f64 arithmetic matches bit for bit except in extreme exponent ranges. At 24-bit precision, f64 then rounded to f32 also matches for those operations. Only 64-bit extended precision would need soft-float.
- **Direct3D may change the precision [H, must verify].** The game passes no FPU flag to `SetCooperativeLevel` in Gold 1.15's strings (`DDSCL_ALLOWREBOOT|EXCLUSIVE|FULLSCREEN`). Direct3D 6 then switches or restores the FPU around its calls. What precision the game code actually runs at must be measured at runtime by reading the control word from the game's own thread with Frida.
- **Emulation in the VM may differ [H].** Windows on ARM's Prism x86 emulator may not reproduce 80-bit x87 internals by default. Bit-exact float comparisons against the VM need Prism's strict floating-point setting checked first, or the physical x86 machine.
- **Transcendentals.** `FSIN`, `FCOS`, `FPATAN`, and `FPTAN` are not correctly rounded on real x87 hardware, and host `libm` results differ in the last bits. Option 1: accept and document ULP-level differences. Option 2: implement the x87 algorithms, which have known reduction behavior, in the runtime. Recommendation: start with option 1 and measure whether differences ever reach gameplay decisions.
- `__ftol` (`0x42ac24`) sets round-toward-zero through `fldcw` and uses `fistp`. It is modeled exactly as written.

### 4.4 Flags and integer semantics

Lazy flag evaluation: record the last operation's operands and kind, and compute flags only when read. Fuse `CMP`/`TEST` with the following conditional jump in the generated code. `SBB`/`ADC` (619 uses), `SETcc`, `SAHF` (`FNSTSW AX` followed by `SAHF` or `TEST AH`) need exact carry, parity, and zero semantics.

### 4.5 Exception frames and thread data

- There are 98 `fs:` accesses. Point `fs` at a thread information block in guest memory, and the C runtime's SEH chain and thread-local storage work unmodified.
- C++ unwind funclets exist (`Unwind@004347xx`). No exception is expected to be thrown during normal play [H]. Phase 1 treats an actual throw or dispatch as fatal and logs it.

### 4.6 C runtime

Recompile the statically linked MSVC 6 C runtime instead of replacing it. That keeps exact `rand`, `sprintf`, `atof`, and `strtok` behavior, plus heap layout effects, and shrinks the runtime's surface to the Win32 imports.

The CRT needs a subset of KERNEL32 from the runtime. Examples are heap functions, `GetCommandLineA`, environment strings, thread-local storage, standard handles, code pages, and file I/O.

### 4.7 Graphics: where to put the boundary

| Option | Surface | Pros | Cons |
|---|---|---|---|
| **A. Implement DirectDraw4 and Direct3D 6 COM** | About 28 method slots across IDirectDraw/4, IDirectDrawSurface4, IDirectDrawClipper, IDirect3D3, IDirect3DDevice3, IDirect3DViewport3, IDirect3DMaterial3, IDirect3DLight, and textures | Every original instruction runs, including `r3d` | Must reproduce D3D6 semantics: surface locks and pixel formats, texture-format enumeration, lights and materials, render states. Modern features would have to be grafted onto low-level draw calls. |
| **B. Override the `r3d_*` API** | About 40 named entry points (`r3d_Render3DModel`, `r3d_SetMatrix`, `r3d_SetMaterial`, `r3d_SurfaceCreateFromBMP`, `r3d_2DBlit`, …) | High-level and well-documented by the error strings. Ultrawide, high-frame-rate interpolation, and modern filtering fit naturally at the model-plus-matrix level. | It is a platform-facing override of original code. `AGENTS.md` allows that only when documented, and it must be checked to contain no gameplay effects. |

**Recommendation: B for rendering, A only where unavoidable.** Examples of unavoidable cases are the DirectDraw enumeration in WinMain, which feeds the video dialog, and the monitor-frequency query in the frame-timing function. `r3d` is a graphics library, not game logic. It contains the timer (`0x424440`), which will be virtualized anyway for the fixed 60 Hz cadence. The decision gets confirmed after the proof of concept, once `r3d`'s internals have been checked for gameplay side effects.

### 4.8 Timing, message loop, and pacing

The game spins on `QueryPerformanceCounter` and `Sleep(1)` until 4 ticks pass [V, also observed at runtime]. The runtime virtualizes the performance counter to produce the decided 60 Hz cadence of 1/1024-second ticks. `Sleep` and `PeekMessage` pump SDL events and present.

Rendering faster than 60 Hz comes later. The game's simulation is interleaved with its draw calls, so higher refresh rates need draw-list capture and interpolation in the runtime. That fits option B.

### 4.9 Audio

FMOD 3.61 needs 13 functions: init, close, sample load, mode, play, free, master volumes, and song load, play, pause, free. The runtime implements them on SDL3 audio, with libxmp for `.xm` playback.

### 4.10 Build and distribution

Generated code is derived from the user's executable and must not be committed. Two delivery models:

1. **User-side build.** A command takes the user's installer or install folder, verifies the hash, extracts the files, runs the recompiler, and compiles. This needs a C compiler plus the Rust toolchain on the user's machine. It is the cleanest legally.
2. **Prebuilt binaries** containing translated code. This is easier for users, but it distributes code derived from the original. That is a legal decision for the owner.

The proof of concept assumes model 1. The choice can be deferred until the vanilla baseline.

### 4.11 Validation

The Windows 11 VM with Frida is the reference for logic and timing [V: attach works, prologues match, and the frame-delta semantics were confirmed over 480 frames].

The plan is differential testing. Drive the original and the recompiled build with identical scripted inputs, tick sequences, and random seeds, then compare guest memory regions such as object lists, score, and state frame by frame. Fixed addresses make this direct.

## 5. Minimal proof of concept

**Goal:** prove the full pipeline on real game code: decoder, control flow, C emission, flags, x87, string instructions, the guest memory model, thread data through `fs:`, stdcall import thunks, COM dispatch through guest vtables, and host-to-guest calls. Graphics, audio, and the message loop are out of scope.

**Deliverable:** `cargo run -p bugatron-poc -- --exe <path to Bug3D.exe> --data <app dir>` on macOS arm64, Linux x86-64, and Windows. The Windows arm64 build can be tested in the VM.

| Step | Real code exercised | Runtime needed | Pass criterion |
|---|---|---|---|
| **P1. C-runtime startup** | `entry` (0x42ba98) through CRT init: heap, thread data, I/O, environment, command line, floating-point init, up to the call to WinMain (0x4076e0), which the runtime intercepts | About 25 KERNEL32 functions, thread information block, guest stack | Reaches WinMain with correct `hInstance` and command line. No unimplemented import hit. |
| **P2. Level decoding through game code** | `FUN_004076a0` → `0x41ce10`, `0x41e916`, `0x41cfae` (JC decompressor: carry-heavy bit twiddling), plus the CRT's `malloc` and `free` | Runtime reads each `.lev` into guest memory and applies the bitwise NOT (that loop is part of `0x417fe0`, which P2 doesn't run) | Output is byte-identical to `tools/lev_decode.py` for all 30 files |
| **P3. Frame timing (x87, `fldcw`, COM)** | `FUN_0040b720` → `IDirectDraw::GetMonitorFrequency` through a guest vtable, `FUN_00424440` (`fild`/`fmul`/`fidiv`), `__ftol` | Virtual performance counter; a minimal COM object in guest memory | Replaying the counter values that produced run 1's 480 deltas gives identical tick, delta, and float values. The edge cases (fewer than 4 ticks, more than 512) match the static analysis. |
| **P4. Random numbers** | CRT `srand` (0x42ab66) and `_rand` (0x42ab73) through per-thread data | — | The first 10,000 values from a fixed seed match the MSVC formula |

**Also required:** one VM session that reads the x87 control word inside the game thread during gameplay (§4.3). Also check Prism's floating-point emulation setting.

**Exit:** if P1–P4 pass on all three operating systems, scale up to whole-program recompilation (milestone 3 in `AGENTS.md`). If x87 precision, COM dispatch, or SEH turn out harder than expected, the proof of concept will show it before the full build is committed to.

## 6. Decisions

Decided by the owner on 2026-11-15:

1. **Generated code is C.** The recompiler and runtime are Rust. The generated C must spell out x86 semantics and never rely on undefined or platform-dependent C behaviour.
2. **Graphics boundary: replace the game's `r3d` wrapper** with a Rust/SDL3 GPU implementation instead of recreating Direct3D 6. Before committing to it, verify the wrapper's calling conventions, state, globals, callbacks, and any behaviour that gameplay code consumes. That verification is **not done yet**; it is the first task after this gate (§7.6).
3. Distribution model: deferred.

## 7. Proof-of-concept results (feasibility gate)

**Verdict: PASS.** The architecture works. Translated original code runs natively on macOS arm64, Linux arm64, Linux x86-64, and Windows x86-64 (physical PC), and reproduces the original's behaviour bit for bit on every comparison made.

### 7.1 What was built

| Component | Location | Notes |
|---|---|---|
| PE reader | `crates/pe` | Sections, entry point, import table |
| Recompiler | `crates/recomp` | `iced-x86` decoding → recursive-descent discovery → one C function per guest function. Hints from Ghidra (`tools/ghidra/ExportRecompHints.java`) are re-validated. |
| Machine model | `crates/runtime/c/recomp.h` | Registers, lazy flags, x87 stack, guest memory access. Portable C99 with explicit semantics. |
| Runtime | `crates/runtime` | 4 GiB guest reservation, loader, IAT thunks, imports, guest heap, TLS, thread information block, COM objects in guest memory, host↔guest calls |
| Proof-of-concept driver | `crates/poc` | Build script recompiles the user's `Bug3D.exe` (hash-checked) and compiles the C; the binary runs the tests |
| Reference capture | `tools/frida/capture_poc_reference.js` → `tests/fixtures/poc-reference.json` | Outputs of the *original* functions, run inside the original process on the game thread in the VM |

Run it:

```
tools/extract_reference.sh --ghidra      # once: extract files, Ghidra hints
cargo run -p bugatron-poc                # or set BUGATRON_EXE / BUGATRON_HINTS
```

### 7.2 Results

All tests passed on every executed host: macOS 26 arm64 (debug and release), Linux arm64 (Docker, Debian bookworm), and Linux x86-64 (Docker under emulation).

| Test | What ran (real game code) | Compared against | Result |
|---|---|---|---|
| M1 memory model | Image mapped at `0x400000`; `.rdata` strings at `0x4375e0` ("config.xml") and `0x4374e0` ("prefs"); `.data` initial values `0x4370a0` = 6 and `0x4370a4` = 1; host address = base + guest address; read at `0xfffffffc` through the guard | Static analysis of the executable | PASS. macOS base `0x300000000`, Linux arm64 base `0xfffe84e01000`. |
| P1 C-runtime start-up | `entry` `0x42ba98`: heap, TLS, I/O, environment, command line, locale, initializer tables, up to WinMain | Windows behaviour; the VM-measured control word | PASS. Reached WinMain with `hInstance` `0x400000`, `lpCmdLine` "" (a guest pointer), `nCmdShow` 10, x87 control word **0x027F**, identical to the VM measurement in the game thread. 27 distinct imports used. |
| P2 `.lev` decoder | `0x4076a0` → `0x41ce10` / `0x41e916` / `0x41cfae`, plus CRT `malloc`/`free` | The original function run in the VM: length, CRC-32, first 24 bytes per file | PASS, **30/30 files byte-identical** |
| P3 frame timing | `0x40b720` → COM `IDirectDraw4::GetMonitorFrequency` through a guest vtable → `0x424440` (`fild` qword, `fmul`/`fidiv` m32, `__ftol`) → `fstp` m32 | 600 recorded frames: exact performance-counter inputs, plus 13 timer globals before and after each frame | PASS, **600/600 frames bit-exact**, including the float delta at `0x81d67c` |
| P3b timing edge cases | Same function with synthetic counter values | Static analysis: spin on `Sleep(1)` below 4 ticks, reuse the previous delta above 512 | PASS: 4-tick floor (3 Sleep calls), 512 accepted, 1000 → previous delta |
| P4 random numbers | CRT `srand` `0x42ab66` / `_rand` `0x42ab73` through `_getptd` per-thread data | The original functions in the VM (first 16 values, a hash of 10,000 values, final state) and the MSVC formula | PASS, 4 seeds × 10,000 values |
| I1 indirect calls | See below | — | PASS |

Real indirect calls and callbacks that executed:

- **Guest to guest, through function-pointer tables.** The CRT's `_initterm` (`0x42ae4f: call eax`) called 29 initializers. 24 are the game's own C++ static initializers (`0x401000`–`0x422d70`); several were never recognised by Ghidra and were found by the recompiler's data-pointer scan.
- **Guest to guest, C++ `this` callback.** The CRT's array-constructor iterator (`0x42a5d3: call [ebp+0x14]`) called the game constructor `0x4141e0`, with the object pointer in ECX. Its address reaches the program only as an immediate operand.
- **Guest to runtime, COM.** 603 calls through a vtable in guest memory to `IDirectDraw4::GetMonitorFrequency`.
- **Guest to runtime, imports.** 121,136 import calls through the IAT, including `call esi` on register-cached import pointers (`Sleep`).
- **Runtime to guest.** 40,759 calls of translated functions from Rust, each with a stack-balance check, plus the CRT calling the runtime's WinMain replacement.

### 7.3 x87 control word and emulator findings (VM, game thread)

| Measurement | Result |
|---|---|
| Control word in the frame function | 0x027F on 601/601 samples |
| Control word in the timer `0x424440` | 0x027F on 3,001/3,001 samples |
| Precision control | 53-bit, round to nearest, all exceptions masked |
| 2^53 + 1 computed under forced 64-bit precision | 2^53 (real x87 hardware: 2^53 + 1) |
| 1/3 under forced 24-bit precision | `0x3FD5555555555555`, the double result (real hardware: rounded to float) |

Conclusions:

- **The game runs at 53-bit precision.** With that setting, x87 `+ − × ÷ √` round exactly like IEEE double precision, so the machine model's double arithmetic is exact for them. The P3 match confirms this on real code.
- **The model does not assume this.** It tracks the control word and flags any other precision or rounding mode through `rt_x87_note` (`X87_NOTE_PC24`, `PC64`, `RC`). P1–P4 raised no notes.
- **The VM's x86 emulator ignores x87 precision control.** It always computes in double. The VM is therefore a valid float reference only while the game stays at 53-bit precision. Anything involving extended precision or transcendental instructions needs the physical x86 machine as the reference.
- **Reserved control-word bit.** Bit 6 reads back as 1 on real hardware. The first model version stored `0x23F` where the VM showed `0x27F`; `fldcw` now forces the bit.

### 7.4 Portability of generated code

- Generated C is C99 with no extensions. It uses unsigned arithmetic, `memcpy` for memory access and signed views, masked shifts, explicit division-overflow checks, and integer-exact x87 rounding (`frnd_rc`) that doesn't depend on the C floating-point environment.
- It is compiled with `-ffp-contract=off -fno-fast-math`, or `/fp:precise` on MSVC, so the compiler can't fuse multiply-add.
- The generated library is linked whole-archive. GNU `ld` needed that because generated code and the runtime reference each other.
- Release and debug builds produce identical results.
- **The whole program** (728 functions, 65,121 instructions) generates 270,000 lines of C. It compiles at `-O2` in 12 s serially on Apple Silicon with no errors.
- A full proof-of-concept release run takes 0.05 s.

### 7.5 Discrepancies, gaps, and fixes made during the gate

| # | Item | Status |
|---|---|---|
| D1 | The first timing capture logged counter calls from other threads (FMOD/DirectSound), and 439 of 600 frames failed to replay | **Fixed** by a game-thread filter in the capture; 600/600 pass |
| D2 | `fldcw` reserved bit 6 (§7.3) | **Fixed** in the machine model |
| D3 | Ghidra hints: 18 "jump tables" were really `jmp [IAT]` import tail jumps with bogus targets | **Fixed**: the recompiler drops table targets that aren't decodable code |
| D4 | Ghidra missed C++ static initializers, and some function addresses appear only as immediates | **Fixed**: the recompiler roots code pointers found in data sections and immediate operands |
| D5 | Windows x86-64 build | **Resolved.** Kit 1 exited silently: the M1 check read reserved-but-uncommitted guest memory. Guest memory is now commit-explicit on every OS, with fault diagnostics. Kit 2 on a physical AMD Ryzen 7 9800X3D (Windows, x86-64): **all tests PASS**, including 600/600 timing frames, 30/30 `.lev` files, CRT start-up with control word 0x027F, and random numbers. MSVC toolchain still untested (built with MinGW). |
| D6 | The timing spin path (below 4 ticks) never occurred in the VM, which runs too slowly. It is covered only by the synthetic edge test derived from static analysis. | Open: capture on the physical machine |
| D7 | x87 transcendentals versus host `libm` | **Resolved** (§7.10): FSIN/FCOS/FPTAN use x87 argument reduction with the 66-bit π. Confirmed on an AMD Ryzen 7 9800X3D (oracle kit 3): all values **and status words** in the trig regression cases match. FPATAN is modelled the same way and confirmed on the same CPU (oracle kit 5, §7.11). |
| D8 | Exceptions: SEH frames are maintained, but the runtime does not dispatch exceptions | **Explicit**: the 4 C++ EH dispatcher jumps are `rt_trap` sites with reasons (`bugatron_recomp::targets`); `RaiseException`, `RtlUnwind`, and `UnhandledExceptionFilter` stop with a diagnostic |
| D9 | Guest heap addresses differ from the original's Windows heap. Logic that depends on pointer values would diverge. | Open: differential tests must compare logical state, not raw pointers |
| D10 | 8 instruction sites were unimplemented in the emitter: `RCR` ×4, `BTS` ×2, `PUSHAD`, `POPAD` | **Fixed**: `RCL`/`RCR` at all sizes, the `BT`/`BTS`/`BTR`/`BTC` family with bit-string addressing, and `PUSHAD`/`POPAD`. The whole program now has 0 unimplemented sites; regression cases in §7.7. |
| D11 | Only 27 of 120 imports are implemented, and 1 COM method | Expected; the rest are added when called |
| D12 | `r3d` graphics boundary | **Mapped and validated**: `docs/graphics/r3d-interface.md` covers 90 functions and 7 direct COM sites. Stdcall cleanup was confirmed on about 4.5 million runtime calls, and the game calls only statically identified functions. |
| D13 | x87 approximations were silent apart from `Cpu::x87_notes` | **Explicit**: `BUGATRON_X87_STRICT=1` stops at the first approximation; the PoC passes in strict mode |

### 7.7 Instruction-semantics regression suite

- `crates/x86-cases` defines 55 cases as iced-x86 assembly. They cover flags for `ADD`/`ADC`/`SUB`/`SBB`/`INC`/`DEC`/`NEG`/`CMP`, shifts and rotates including `RCL`/`RCR` at all sizes, `SHLD`/`SHRD`, the `BT` family with bit-string addressing, `PUSHAD`/`POPAD`, multiply and divide, string instructions with `REP`/`REPE`/`REPNE` and DF, `SETcc`, and x87 cases (precision control, rounding modes, `FIST` overflow, compares through `FNSTSW`/`SAHF`, 80-bit store/load, transcendentals).
- `cargo test -p bugatron-x86-tests` runs each case through the real recompiler and runtime. It checks hand-derived expectations from the Intel SDM, compared on defined flags only, and every oracle fixture present.
- `crates/x86-oracle` runs the same bytes natively. Fixtures:
  - `tests/fixtures/x86-oracle-qemu-i386.json` comes from QEMU's i386 emulator (**not hardware**). It validates the harness and integer and flag semantics. All strict cases match. The 64-bit-precision case differs as documented, since QEMU implements 80-bit precision.
  - `tests/fixtures/x86-oracle-amd-ryzen7-9800x3d.json` comes from real hardware (AMD Ryzen 7 9800X3D). **All strict cases match**, FPATAN included (kit 5, 2026-11-17). The 64-bit-precision case differs exactly as documented, and the transcendental report-only case matches bit for bit.

### 7.9 x87 transcendentals on real hardware

`x87_transcendental_sweep`, `fstp` to binary64, control word 0x027F, on an AMD Ryzen 7 9800X3D, compared with the model (host `libm` on macOS arm64):

| Input | sin: hardware vs model | cos: hardware vs model |
|---|---|---|
| 0.1, π/4, 2.0, 5.5 | identical | identical |
| π/2 | identical (1.0) | **6.123031769111886e-17** vs 6.123233995736766e-17 |
| π | **1.2246063538223773e-16** vs 1.2246467991473532e-16 | identical (−1.0) |
| 4.0 | **−0.7568024953079282** vs −0.7568024953079283 (1 ULP) | identical |
| 2π | **−2.4492127076447545e-16** vs −2.4492935982947064e-16 | identical (1.0) |

Interpretation:

- **The three large relative differences share one cause.** All are results near a zero of the function, and all differ by about 4.04e-21 absolute. That is the gap between π and the reduced-precision π the x87 uses for argument reduction. It's an architectural, documented x87 behaviour, so emulating it is well defined.
- **The 1-ULP difference at sin(4.0)** comes from the hardware's own polynomial and rounding. Matching that bit for bit in every case would need a reimplementation of the CPU's internal algorithm. That may differ between vendors, and the original game ran on whatever CPU each player had.
- **Consequence:** rotation matrices from `0x421680` (about 127 calls per frame) can differ in the last bits from the original. Whether that can reach gameplay is G7 (`docs/graphics/r3d-interface.md`).
- **Recommendation (adopted, §7.10):** emulate x87 argument reduction in the runtime's `fsin`/`fcos`/`fptan`.

### 7.10 x87-compatible FSIN / FCOS / FPTAN

Implementation: `crates/runtime/src/x87trig.rs`, called from generated code as `rt_x87_fsin/fcos/fptan`.

1. The argument is converted exactly to fixed point: 192 fraction bits, or 1200 for |x| < 2⁻⁸⁰.
2. It is reduced exactly modulo π₆₆/2, where π₆₆ = `0x3243F6A8885A308D3 × 2⁻⁶⁴`, the x87's 66-bit approximation of π.
3. The needed series is evaluated far beyond 64 bits.
4. The result is rounded to the x87's 64-bit significand, then to binary64, which is exactly what `FSTP m64` does on hardware.

Status and stack behaviour:

- C2 = 1 with the operand unchanged for |x| ≥ 2⁶³.
- C1 = 1 when the 64-bit rounding increased the magnitude.
- PE (sticky) is set on every inexact result, which means any nonzero in-range operand.
- ±∞ is an invalid operation (IE) and produces the default QNaN.
- NaN propagates.
- FPTAN pushes 1.0, except for out-of-range operands.
- Stack overflow and underflow raise `X87_NOTE_STACK`.

Prototype check: an independent Python implementation (`_work/x87trig_proto.py`, not committed) reproduced all 19 known hardware values only with π of at least 66 bits. 64- and 65-bit π both miss sin(π), sin(2π), and cos(π/2). Bits 67–68 of π are zero, so 66, 67, and 68 bits are the same constant.

Remaining approximations:

- The machine model keeps ST registers as binary64. When the 64-bit result isn't representable in binary64 (`X87_NOTE_EXT64`), arithmetic that consumes the register *before* it is stored sees the 53-bit value, whereas hardware sees 64 bits. Stored results are exact.
- The CPU's internal polynomial may differ from a correctly rounded 64-bit result by one ULP in rare cases. None has been observed. Per the owner's direction, no further workaround will be added without evidence that it matters.

Real workload (VM capture, `tools/frida/probe_rotation_angles.js`, 5,400 frames, about 690,000 calls to `0x421680`):

- Angles span [−2π, 2π] and are halved before FSIN/FCOS.
- 55% are ±0.
- Exact float32 ±π/2, ±π/4, and π are common.
- There are clusters near 2⁻³⁵ and 2⁻⁴, and a tail of denormal float32 angles.

Regression cases: `x87_trig_workload_angles` (real float32 angles), `x87_trig_negative_and_multiples`, `x87_trig_boundaries` (±0, tiny, largest in-range, ±2⁶³, +∞, NaN, with status words), `x87_fptan_values_and_stack`. The transcendental cases now run strict against the hardware fixture. Emulated (QEMU) oracles are exempt for trig cases, because QEMU doesn't implement x87 reduction.

Cost: about 3.6 µs per call in a release build on Apple Silicon, about 0.9 ms per frame at the measured rate.

**Hardware confirmation (oracle kit 3, AMD Ryzen 7 9800X3D):** all 60 cases match except the documented 64-bit-precision case. Every FSIN/FCOS/FPTAN value matches bit for bit, as does every stored status word (C1, C2, IE, PE, TOP).

The first comparison of kit 3 found two status-flag gaps, both now fixed:

- PE was not set.
- C1 was wrong for sin(1e-20). The `r³/6` correction that decides the rounding direction fell below the 192-bit working precision. The implementation now widens precision losslessly when the reduced argument is small.

The PE flag is modelled for FSIN/FCOS/FPTAN only. Arithmetic instructions don't set PE yet; see open-questions R9.


### 7.11 x87-compatible FPATAN

`FPATAN` (ST(1) = atan2(ST(1), ST(0)), then pop) was the last transcendental still computed by the host `libm`. It now uses the model in `runtime/src/x87trig.rs` (`x87_fpatan`):
- **Computing the angle:** the ratio min(|y|,|x|)/max(|y|,|x|) is formed exactly in wide fixed point (widened for tiny ratios). atan is evaluated by halving the angle (`atan t = 2·atan(t/(1+√(1+t²)))`) until t ≤ 1/8, then by the Taylor series. The quadrant terms π/2 and π are then applied, using the 66-bit π (π₆₆) that FSIN's reduction uses.
- **Rounding:** the result is rounded to the 64-bit significand (C1 = rounded up), then to binary64, flagging `X87_NOTE_EXT64` when that second rounding is inexact. PE is set for inexact results.
- **Special values:** signed zeros and infinities follow the x87 table (the same as C99 `atan2`). NaNs propagate quieted, the larger significand winning when both operands are NaN. FPATAN has no invalid-operand or range cases. C0, C2 and C3 are undefined after FPATAN, so the oracle case compares C1 only.

**Confirmed on hardware (kit 5, 2026-11-17, AMD Ryzen 7 9800X3D):** the quadrant terms use π₆₆, not the true π. The oracle case `x87_fpatan_quadrants_and_table` covers each quadrant, results next to ±π and π/2, and the table entries.
- All eight results matched bit for bit. With the true π, one status word differed: atan2(2, −3) = π − atan(2/3) lies just above a 64-bit rounding midpoint, so it rounds up (C1 = 1), but the hardware rounded it down (C1 = 0).
- π₆₆ is about 4·10⁻²¹ below π, which puts that result just below the midpoint. With π₆₆ the model matches every value and status word.
- The binary64 results are the same either way, and the scripted gameplay frames are unchanged.

Checks so far:
- The hardware case `x87_transcendentals` (atan2(1, 0.5), measured on the Ryzen) matches.
- 3,000 random operands agree with macOS `libm` exactly or within one ULP, where rounding first to 64 bits changes the binary64 result.
- Scripted gameplay frames are unchanged.

### 7.8 Recommended next steps

2. Resolve G1–G7 (`docs/graphics/r3d-interface.md`) for each function before replacing it.
3. Start the graphics replacement, following the leaf-replacement plan.
