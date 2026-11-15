# Bugatron — Static Recompilation

You are building a **static recompiler** and a **portable runtime** for **Bugatron**, the classic arcade game developed by Retro64, so it runs as a native, cross-platform application.

The original game's 32-bit x86 machine code is translated ahead of time into native code. Windows-specific APIs are replaced by a portable runtime written primarily in Rust. The original game logic runs as recompiled machine code; it is **not** rewritten by hand.

This is **not**:

- a handwritten reimplementation or decompilation-based rewrite of the gameplay logic,
- an emulator or interpreter that runs the original executable at runtime, or
- a byte-for-byte rebuild of the original executable.

"Non-matching" means the final application may add modern enhancements (higher rendering frame rates, ultrawide resolutions, modern controllers, additional settings). It does **not** mean the original gameplay algorithms may be reimplemented by hand.

## Project Goals

1. Build a **static recompiler** that translates the primary target's x86 code into native code at build time.
2. Build a **runtime**, primarily in **Rust**, using **SDL3** for platform integration, that replaces every Windows, DirectX, and FMOD dependency the game uses.
3. Support Windows, macOS (including Apple Silicon), and Linux natively.
4. Preserve the original game logic as faithfully as possible by executing its recompiled code: algorithms, constants, timing, physics, collision, AI, scoring, randomness, and progression.
5. Run without the original executable, Windows, Wine, or any emulator at runtime. The user supplies original game files separately.
6. Reach a **fully playable vanilla baseline** before adding modern features.

**Behavioral fidelity takes precedence over code elegance or optimization.**

The resulting application should play and feel indistinguishable from the original wherever reasonably possible.

## Primary Target

**Bugatron Gold v1.20a**, `Bug3D.exe`, SHA-256 `be581a63bccfc7dcc78a5c55cacc02e9f7aab9e5398540d2cc86a7aba758919e`.

Other releases (Gold 1.15, standard 1.51d, standard "v120") are reference material only. Do not support them unless the owner asks.

## Reverse-Engineering Tools

Use **REA (Reverse Engineer Anything)** and **Ghidra** to investigate the supplied original executables.

- Inspect actual disassembly rather than guessing how code works. Pseudocode is a reading aid; the recompiler works from machine code.
- Identify functions, jump tables, callbacks, global data, calling conventions, and every import and COM interface the game uses.
- Maintain a mapping between original addresses and runtime implementations or overrides.
- Distinguish confirmed findings from hypotheses.
- Do not assume Ghidra's analysis is complete or correct. Consult the assembly and runtime observations when behavior is ambiguous.
- If tooling is unavailable or fails, report the issue rather than fabricating reverse-engineering findings.

### Multiple Game Versions

Do not mix function addresses or version-specific assumptions between executables. Maintain a separate analysis record for each version.

## Recompilation Principles

### The recompiled code is the game

- All original game code, including its statically linked C runtime and its in-house Direct3D wrapper, is recompiled by default.
- **Do not reimplement original gameplay logic by hand.** Do not "fix", simplify, or reinterpret game code paths in generated output.
- Preserve x86 semantics: integer widths, signedness, wrapping, flags, the x87 floating-point stack, precision control, rounding, and `__ftol` truncation.
- The recompiler must not depend on Ghidra pseudocode. Function boundaries, jump tables, and callback lists may come from analysis exports, but translation works from decoded instructions.
- Keep the recompiler deterministic. The same input executable produces the same generated code.

### The runtime replaces the platform

- The runtime implements the **import surface** the game calls: KERNEL32, USER32, GDI32, SHELL32, WINMM, DDRAW and the COM interfaces it returns, and FMOD 3.61.
- Higher-level replacement (overriding a specific original function with a runtime implementation) is allowed **only for platform-facing code**, such as the game's Direct3D wrapper or its timer. Every override must be documented with its original address, the reason, and the behavior it preserves. Never override gameplay functions.
- Guest code sees a 32-bit address space that matches the original layout. The runtime translates guest structures and pointers at the API boundary.
- Use SDL3 for windowing, graphics (the SDL3 GPU API), input, audio, and timing. XM music uses libxmp through FFI.
- Gameplay timing follows the decided model: the original variable-step update driven by a fixed 60 Hz reference cadence of 1/1024-second ticks, independent of the display rate.

### Languages

- The recompiler and runtime are written in **Rust**.
- Generated code may be C or Rust, whichever compiles and performs best. That choice is recorded in the feasibility document.
- C/C++ may be used for isolated interoperability components such as libxmp.

### Original files and distribution

- The user supplies their own copy of the game. The tooling verifies it by hash.
- Never commit original executables, game data, or code generated from them. Keep them in gitignored directories.
- Do not modify original game files.
- Load original asset formats directly rather than converting them.

## Verification and Testing

Validation against the original executable is essential.

- Use the Windows 11 arm64 VM (UTM, dgVoodoo2, frida-server) for runtime traces of the original game. Treat the VM as the reference for logic, timing, and audio, not for visuals.
- Build **differential tests**: drive the original (through Frida hooks) and the recompiled build with the same scripted inputs, timer ticks, and random seed, then compare game state frame by frame.
- Test the recompiler itself with focused instruction-semantics tests, especially flags, x87, and string instructions.
- Do not claim behavioral equivalence without supporting evidence.
- Maintain a list of known discrepancies, uncertainties, and unimplemented runtime functions.

## Modern Features — Deferred

The long-term goal includes optional modern improvements such as:

- Configurable resolution and fullscreen modes.
- Widescreen and ultrawide support.
- High-DPI and high-refresh-rate rendering.
- Controller support and remappable inputs.
- Independent audio volume controls.
- A modern settings menu.
- Additional accessibility or quality-of-life features.

**Do not implement these enhancements until the vanilla baseline is playable and validated.**

Design the runtime so they can be added later without changing recompiled game logic. Enhancements belong in the runtime layer, not in generated code.

Avoid adding new gameplay features, modifying balance, or changing original mechanics.

## Development Workflow

Work incrementally, using small, verifiable milestones.

1. **Done:** inventory executables, dependencies, and assets; identify architecture and subsystems.
2. Assess static-recompilation feasibility and build a minimal proof of concept that runs real game code.
3. Build the full recompiler: all functions, jump tables, callbacks, x87, and exception-handling frames.
4. Build the runtime: memory model, kernel and user APIs, file system, timing, input.
5. Graphics: DirectDraw/Direct3D 6 or the game's Direct3D wrapper, on the SDL3 GPU API.
6. Audio: FMOD 3.61 API on SDL3 audio and libxmp.
7. Reach a fully playable vanilla baseline on all three platforms.
8. Validate against the original with differential traces. Tag a known-good vanilla baseline.
9. Only then add modern features.

Adjust the ordering when findings justify it. Keep the project compilable and testable throughout.

## Documentation

Maintain documentation covering:

- Original executable versions and their differences.
- Function mappings, callbacks, jump tables, and runtime overrides.
- The guest memory model and API boundary conventions.
- Every runtime-implemented import and COM method, with its status.
- Asset and data formats.
- Unresolved questions and assumptions.
- Behavioral verification results.
- Known deviations from the original.

Record relevant executable addresses, version identifiers, and supporting evidence so findings can be revisited.

The overriding objective is to preserve Bugatron as faithfully as possible by running its original logic natively on modern operating systems.
