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

**Verdict: PASS.** The architecture works. Translated original code runs natively on macOS arm64, Linux arm64, and Linux x86-64, and reproduces the original's behaviour bit for bit on every comparison made. A Windows x86-64 build compiles and links but has not been run yet.

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
| D5 | Windows x86-64 build cross-compiled with MinGW but **not executed**; MSVC toolchain untested | Open: run the `.exe` on Windows (VM under emulation, or the physical machine) |
| D6 | The timing spin path (below 4 ticks) never occurred in the VM, which runs too slowly. It is covered only by the synthetic edge test derived from static analysis. | Open: capture on the physical machine |
| D7 | x87 transcendentals (`FSIN`, `FCOS`, `FPATAN`, `FPTAN`) and 80-bit loads and stores are emitted but not exercised; they use host `libm`, which differs in the last bits from x87 | Open: measure against the physical x86 machine; decide whether to emulate x87 transcendentals |
| D8 | Exceptions: SEH frames are maintained, but no exception was thrown. The 4 CRT unwind-helper indirect jumps are translated as tail calls, which would be wrong if an exception were dispatched. | Open: implement or explicitly trap exception dispatch |
| D9 | Guest heap addresses differ from the original's Windows heap. Logic that depends on pointer values would diverge. | Open: differential tests must compare logical state, not raw pointers |
| D10 | 8 instruction sites outside the proof-of-concept are unimplemented in the emitter: `RCR` ×4, `BTS` ×2, `PUSHAD`, `POPAD` | Open: trivial to add before the full build |
| D11 | Only 27 of 120 imports are implemented, and 1 COM method | Expected; the rest are added when called |
| D12 | The `r3d` graphics boundary isn't verified yet (§6). Known coupling: the game's frame-timing function calls `r3d`'s timer `0x424440` and reads its globals. | Open: first task after this gate |

### 7.6 Recommended next steps

1. Run the Windows build (D5), and capture the spin path and transcendentals on the physical x86 machine (D6, D7).
2. Verify the `r3d` boundary: calling conventions, global state shared with game code, and callbacks (D12).
3. Begin the full recompiler: whole-program translation, the remaining instructions (D10), exception policy (D8), and a generated callback registry. Then grow the runtime from evidence toward the first rendered frame.
