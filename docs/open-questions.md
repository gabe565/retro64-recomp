# Open questions, uncertainties, and known gaps

This is a living list. Each entry states its scope, what is known, and how to resolve it.

## Gold 1.20 — static analysis

| # | Question | Known | How to resolve |
|---|---|---|---|
| Q1 | Names and semantics of game states in `0x4370a0` (2, 3, 5, 6, 8, …) | State 3 = transition and teardown; initial value 6; **Partly resolved at runtime:** 5 = config running, 3 = transition, 2 = one-frame load (see gold-1.20.md run 2). Still unknown: 6 (initial) and 8. | Decompile all writers of `0x4370a0` |
| Q2 | Which duplicate attribute wins (`FILE_NAME="pinch1.r3d" FILE_NAME="n4b.r3d"`) | Parser is a custom scanner | Port the attribute lookup in `0x419890` |
| Q3 | Number parsing in the scanner (CRT `atof` or custom; locale) | — | Trace numeric conversions in the parsers |
| Q4 | Is per-frame decay and acceleration applied per update or scaled by dt? | dt is available as ticks and as dt/1024 | Read the alien and ship update math |
| Q5 | Meaning of the `r3d_Initialize` args 184800.0 and 75.0 | 75.0 is likely the FOV | Decompile `0x424b90` projection setup |
| Q6 | Meaning of the 11 material floats | Fits diffuse(4), power, specular(3), emissive(3) | Trace `FUN_004266e0` (material create) |
| Q7 | How model colour overrides (RED1..BLUE3, COLOR1..3) apply | — | Decompile `0x418cc0` and the render path |
| Q8 | Is fog actually used? | State strings present | xrefs to the fog render-state calls |
| Q9 | F8+F9 behavior, Insert, Delete, F2, F3 bindings | VK table known | Follow `param_1[...]` consumers of `0x411f70` |
| Q10 | When `.hs` files are written, and tie ordering | Format known from data | Trace `0x4163e0` / `0x418730` |
| Q11 | Screenshot key and file name (README says F12 → `ss.bmp`; binary has `screenshot%d.bmp`) | — | Trace `0x412930` |
| Q12 | How the vsync limiter (`0x41c180`) interacts with `0x40b720`, and its default | Toggled by the "vsync on/off" UI | Trace its caller (one call site) |
| Q13 | Pack manifest line 2 (16 / 8) | Equals `<LEVEL>` count minus 1 in every pack | Trace `0x417fe0` consumers |
| Q14 | Where music and sfx volumes are stored and persisted | `0x4370a8` = 90 and `0x4370ac` = 25 are candidates | Trace the FMOD volume calls |
| Q15 | Role of the 6 MB `.data`/BSS pools (object caps, list limits) | — | Size arrays as structures are recovered |

## Recompilation (see recompilation-feasibility.md)

| # | Question | How to resolve |
|---|---|---|
| R1 | **Resolved:** the control word is 0x027F (53-bit precision) in the frame and timer functions, across 600 frames. Gameplay-heavy functions (`0x40df90`) still need sampling during play. | See recompilation-feasibility.md §7.3 |
| R2 | **Resolved:** no. Prism ignores x87 precision control and computes in double. The VM is a float reference only at 53-bit precision. | §7.3 |
| R3 | Classify 7,410 instruction-like bytes in `.text` outside Ghidra functions | Recompiler recursive descent plus manual review |
| R4 | Semantics of the 4 unresolved indirect jumps in CRT unwind helpers (exception policy) | §7.5 D8 |
| R5 | Does `r3d` contain any gameplay-affecting logic (needed before overriding it)? | Read the 90 `r3d` functions' data flow into game globals |
| R6 | Exact COM methods used, per interface | Type the vtable call sites in `r3d` and WinMain |
| R7 | Windows build executed? | **Resolved**: passes on physical Windows x86-64 (§7.5 D5) |
| R8 | Transcendental x87 results versus real hardware | **Resolved** for FSIN/FCOS/FPTAN (§7.10); FPATAN still libm |
| R9 | The x87 precision flag (PE) and other exception flags are not set by arithmetic instructions | Only matters if game code reads the status-word exception bits; check for FNSTSW consumers that test bits 0–5 |

## Graphics

See `graphics/r3d-interface.md` §7 (G1–G7) and §8: runtime boundary validation is pending.

## Versions

| # | Question | Notes |
|---|---|---|
| V1 | Contents of the standard "v120" Ghost Installer | Parked; extract only in a VM if needed |
| V2 | Is gameplay code identical between Gold 1.15 and 1.20? | Use `match_native_functions` / Ghidra Version Tracking after the 1.15 import |
| V3 | `smash.wad` / `smash.ndx` format (1.51d) | Out of scope for the Gold 1.20 baseline |

## Process

| # | Gap |
|---|---|
| P1 | **Runtime observation works (2026-11-15).** The VM is Windows 11 arm64 under UTM, with dgVoodoo2's x86 `DDraw.dll`/`D3DImm.dll` and frida-server 17.23.2 (windows-x86) on port 27042. REA attaches to the emulated `Bug3D.exe` process; the base is 0x400000 and the prologues match. Because dgVoodoo2 changes rendering, the VM is the reference for logic, timing, and audio only. Still open: dgVoodoo2 version, and the visual reference machine. |
| P2 | **Resolved 2026-11-15.** REA's Ghidra provider works (Ghidra 12.2, 796 procedures listed for Gold 1.20). Ghidra checkpoint mode is off, so names and comments set through REA are discarded on close. Setting `REA_GHIDRA_CHECKPOINT_DIR` would keep them. |
| P3 | No behavioral equivalence has been claimed for anything except the `.lev` decoder and `.r3d` parser. Both are verified by exact reconstruction of every shipped file. |

## Observed issues (unconfirmed)

| # | Report | Notes |
|---|---|---|
| O1 | 2026-11-16, macOS fullscreen: P (pause) → three-finger swipe to another Space → swipe back → P did not unpause. Not reproduced on retry. | Possibly a missed focus-gained event across a Spaces switch, which would leave the game inactive: a frozen screen looks paused. Or the build predated 22ebd92 (restore on refocus). If it recurs, try `BUGATRON_TRACE=messages,sdl` and check for `WM_SETFOCUS`/`WM_SIZE` on the way back. Possible fix: reconcile with the window's actual focus on each `PeekMessage` (not done; event-driven focus is the norm) |
| O2 | 2026-11-16: atoms "very fast, nearly vertical". | Explained by the original code (function map: 0x415bb0, step index × 1.5° per frame, about 4 orbits/s at any frame rate). Not yet compared against the original at runtime |

## Known deviations

None yet, because no game code exists. Record deviations here as they are introduced.
