# Initial investigation report

> **Direction change (2026-11-15):** the project is now a **static recompilation**, not a handwritten Rust port. See `AGENTS.md` and `docs/recompilation-feasibility.md`.
> The findings in §2–§6 still stand. §7 (Rust crate architecture), the handwritten-port parts of §8, and §9 (milestones) are **superseded**.

Date: 2026-11-15. Scope: the investigation phase from `AGENTS.md`. No game code has been written.

Confidence labels used below:

- **[V] Verified**: read from code or data and cross-checked.
- **[S] String or API anchored**: inferred from references.
- **[H] Hypothesis**.

## 1. Summary

- **Primary target: Bugatron Gold v1.20a (`Bug3D.exe`, sha256 `be581a63…`).** It is the newest Gold build, unpacked, and about 210 KB of MSVC 6 code. Every data file it uses has now been decoded.
- **All game data is readable.** The `.lev` "XML" scripts use a bitwise NOT plus JCALG1-style compression, and a verified decoder is in `tools/lev_decode.py`. The models use a simple documented binary format (`tools/r3d_dump.py`). Textures are BMP, sounds are WAV, and music is XM.
- **A Rust/SDL3 reconstruction is technically feasible.** The game is a 3D-rendered 2D shooter. It uses a thin in-house Direct3D 6 wrapper with fixed-function lighting, FMOD 3.61 for audio, and Win32 input. None of these are hard to replace.
- **Main fidelity risks:**
  1. The original uses a **variable timestep** in 1/1024-second ticks, so gameplay depends slightly on frame rate.
  2. x87 floating point.
  3. Reproducing the custom XML-ish parser's quirks.
  4. FMOD 3 XM playback character.
  5. Reproducing D3D6 fixed-function lighting in shaders.

## 2. Tooling status

| Tool | Status |
|---|---|
| REA 6.9.0 (MCP) | **Working.** The Hopper backend was used for imports, resources, strings, xrefs, and pseudocode. Evidence IDs are recorded in this session's REA ledger. |
| REA Ghidra provider | **Not configured.** `GHIDRA_INSTALL_DIR` is unset in the REA MCP registration. Fix: add `GHIDRA_INSTALL_DIR=/opt/homebrew/Cellar/ghidra/12.2/libexec` to the `rea` server env in `~/.claude.json` and restart. |
| REA Hopper backend | It timed out on re-open late in the session, while running at the same time as Ghidra. One retained cleanup is pending (`/private/tmp/rea-1J8P7O`). Restarting the MCP connection will clear it. |
| Ghidra 12.2 (headless) | **Working.** Full auto-analysis plus a decompile export of all 660 non-thunk functions with 0 failures (`tools/ghidra/ExportDecomp.java`). Ghidra rejects project paths that contain a dot-prefixed directory, which is why the workspace is `_work/` rather than `.work/`. |
| innoextract 1.9, upx, objdump | Working. Used for installer extraction, DLL unpacking, and spot assembly checks. |

To reproduce the workspace, run `tools/extract_reference.sh --ghidra`. It writes to `_work/` (gitignored) and never touches `reference/`.

## 3. Version inventory

Each version has its own record under `docs/versions/`.

| Release | Container | Game exe | Linked | Status |
|---|---|---|---|---|
| **Gold 1.20a** | zip → Inno Setup | `Bug3D.exe` + launcher `Bugatron.exe` | 2003-03-19 | **Primary.** Extracted and analysed. |
| Gold 1.15 | zip → Inno Setup | `Bugatron.exe` (single exe, different audio library) | 2001-09-19 | Extracted. Secondary reference. Its verbose `r3d` error strings help name engine internals. |
| Standard 1.51d | wrapper stub + appended Inno Setup 4.2.6 | `Bug3D.exe` + DevIL + ReflexiveArcade DRM | 2003-11-30 | Extracted. A different game: new weapons, Ogg audio, `.wad` archive. Reference only. |
| Standard "v120" | UPX → Ghost Installer | — | — | **Not extracted** (no extractor). Parked per owner direction. |

[V] All three extracted versions share the `.lev` encoding. Gold 1.15→1.20 level diffs are small, mostly added particle models. 1.51d diverges substantially.

## 4. Architecture of the original (Gold 1.20)

[V] unless noted. The full table is in `docs/function-map/gold-1.20.md`.

```
entry (CRT) ─► WinMain 0x4076e0
                 ├─ DirectDraw enumerate ─► prefs ─► dialog 102 ─► CreateWindow (640×500 or fullscreen)
                 ├─ r3d_Initialize 0x424b90 (DDraw4/D3D3, fov 75°[H])
                 └─ PeekMessage loop
                      ├─ once: InitApp 0x4087d0 (light, srand(timeGetTime), load config.xml)
                      └─ each idle: frame 0x40ba70
                           ├─ input poll 0x411f70 (VK table, joystick, mouse)
                           ├─ surface-lost recovery
                           ├─ timing 0x40b720 (dt in 1/1024 s, ≥4, >512 ⇒ reuse last)
                           └─ switch(game_state @0x4370a0)  {2,3,5,8,…}
                                 ├─ config/level interpreter 0x419bb0 + parsers
                                 ├─ aliens 0x40df90 (stage state machine), ship, bullets,
                                 │  power-ups, boing ball, atoms, stars, text effects
                                 └─ r3d_RenderBegin … r3d_Render3DModel … r3d_RenderEnd, PageFlip
```

- **Platform surface to replace:**
  - DirectDraw4 and Direct3D3 for rendering.
  - Win32 window, message, and dialog APIs.
  - `GetAsyncKeyState`, the window-procedure key array, `joyGetPos`, and the cursor.
  - FMOD 3.61 for XM music and WAV effects.
  - QPC, `timeGetTime`, and `Sleep`.
  - `ShellExecute` for URLs.
  - There is no networking. The only file writes are `prefs`, `*.hs`, and screenshots. The registry is not used.
- **Rendering requirements:**
  - Perspective 3D with a depth buffer.
  - Per-vertex lighting with 1 positioned light at (1.2, 1.2, −9.0), white, plus ambient and specular.
  - D3D materials (diffuse, power, specular, emissive).
  - BMP textures with optional alpha masks and point or linear filtering.
  - Alpha-blended polygons drawn after opaque geometry.
  - Fog render states exist (use not yet traced) [S].
  - Particles, stars, and glyphs as textured quads [H].
  - A "PERSPECTIVE" config option [S].
  - **SDL3's 2D renderer is not sufficient.** Use the SDL3 GPU API (or OpenGL through SDL3) with shaders that emulate D3D6 fixed-function Gouraud lighting.
- **Randomness:** MSVC `rand()` linear congruential generator, seeded from `timeGetTime()` [V]. The port must use the identical generator. A seed override can make tests deterministic, and the default should stay time-seeded to match the original.

## 5. Asset accessibility

| Asset | Format | Status |
|---|---|---|
| `*.xml.lev` (30) | NOT + JC LZ, then XML-like text | [V] decoded (`docs/formats/lev.md`) |
| `m/*.r3d` (136) | "INDOTEK 2" binary meshes | [V] layout (`docs/formats/r3d.md`) |
| `m/*.bmp`, `Fonts/*.bmp`, `Graphics/*.bmp` | Uncompressed BMP, 24-bit or 8-bit | [V] standard |
| `Audio/*.wav` (25) | RIFF PCM | [V] standard |
| `Audio/*.xm` (11) | FastTracker 2 XM | [V] standard. Needs an XM replayer. |
| `pack*.hs` (9) | 10 × {char[20], u32 score, u32 level} | [V] from data; writer not traced |
| `prefs` | 3 × i32 | [V] |

All assets can be loaded in their original formats at runtime. No conversion step is needed. The port should take the path to an existing Gold 1.20 install, so assets are never bundled.

## 6. Feasibility and principal risks

1. **Timing model (high impact).** [V] Simulation advances by a measured frame delta, in integer 1/1024-second ticks with a minimum of 4. `AGENTS.md` requires gameplay timing that is independent of the display rate. The original was never fully independent: physics such as `DECAY=0.995` per update applied each frame would be frame-rate dependent [H, check in code]. A decision is needed (see §8).
2. **x87 floating point (medium).** The code stores f32 but computes on the x87 stack with extended precision and `__ftol` truncation. Rust f32 on SSE will differ in the last bits. This matters only where results feed discrete decisions such as collision thresholds, integer truncation, or random-number draws. Mitigation: mirror the original evaluation order, use f64 intermediates where the original keeps values on the x87 stack, and add tests at decision points.
3. **Custom parser semantics (medium).** Duplicate attributes, unclosed tags, `<!` comments, `\r`-terminated field copies, and number parsing (`atof` versus a custom routine, still unverified). Getting these wrong silently changes level content. Mitigation: port the scanner function by function and add golden tests against all 21 level files.
4. **Alien AI and level-script volume (medium, effort).** The alien parser plus stage machine is about 95 KB of pseudocode across `0x403730`, `0x40ca60`, and `0x40df90`, with roughly 40 stage types. The work is large but mechanical, and the data is fully available to test against.
5. **Audio fidelity (medium).** FMOD 3.61 XM replay, 22 kHz samples, and FMOD's mixer. Any Rust or C replayer will differ subtly. libxmp or libopenmpt through FFI is the most accurate option available, though neither matches FMOD bit for bit.
6. **Rendering fidelity (low to medium).** D3D6 fixed-function lighting is well documented and reproducible in a shader. Driver-specific behavior from 2003, such as 16-bit dithering, has no specification and is out of scope.
7. **Runtime validation (process).** There is no Windows host yet for observing the original. It is needed for timing, feel, and audio comparisons.

No packing, anti-debugging, or obfuscation exists in the primary executable. The DRM exists only in 1.51d.

## 7. Proposed project architecture (Rust)

```
Cargo workspace
├─ crates/formats     (no I/O policy) lev decode, XML-ish scanner (faithful port), r3d, bmp, hs, prefs
├─ crates/game        reconstructed logic, platform-free: state machine, config interpreter, aliens,
│                     ship, power-ups, text effects, scoring, high scores, MSVC rand, tick timing.
│                     Consumes an Input snapshot, emits a render list and audio events.
├─ crates/r3d         r3d-API-shaped scene layer: models, materials, light, matrices, alpha-poly list
├─ crates/platform    SDL3: window, GPU renderer (fixed-function emulation), input mapping,
│                     audio mixer + XM replayer, timing source, file-system (case-insensitive lookup)
└─ crates/bugatron    binary: CLI/config (asset dir, window size), main loop wiring
```

Rules:

- `game` never calls SDL.
- Every reconstructed function carries a doc comment with the original address, for example `/// Original: Bug3D.exe Gold 1.20a FUN_0040b720`.
- Enhancements go in `platform` and `bugatron` behind config, and are deferred.
- Tests use the user's local assets through an environment variable and skip when it is absent. No game data is committed.

## 8. Decisions

**Decided 2026-11-15 by the owner:** timing option (b), a fixed 60 Hz reference cadence; the SDL3 GPU API; libxmp through FFI. A Windows VM is available for runtime observation, and a physical Windows machine is the fallback.

The original options are kept below for context.


1. **Timing strategy.** Recommendation: **(b)**.
   - (a) Pure original: feed the measured frame delta in ticks, keeping the minimum of 4 and the over-512 rule. This is most authentic, but gameplay varies with refresh rate, which conflicts with `AGENTS.md`.
   - **(b) Fixed reference cadence:** run the original variable-step update with synthetic deltas generated at a fixed reference rate. The default would be 60 Hz, giving 17/17/18-tick deltas, the cadence of a vsynced 2003 machine. The display rate would be fully decoupled from it.
   - (c) As (b), but with an option to switch to (a).
2. **GPU API.** Recommendation: the **SDL3 GPU API**. It needs a shader toolchain such as SDL_shadercross or prebuilt SPIR-V/MSL/DXIL. The alternative, OpenGL 3.3 through SDL3, is simpler to set up but deprecated on macOS.
3. **XM replay.** Recommendation: **libxmp through FFI**. It is isolated C, which `AGENTS.md` allows, and it is FT2-accurate. The alternative is a pure-Rust replayer such as `xmrs`, which is less proven.
4. **Runtime reference.** Is a Windows machine or VM available for observing the original game? That would cover frame-delta logging, side-by-side play, and Frida traces through REA. Wine on macOS is an alternative, but its D3D6 path is less trustworthy for visuals.

## 9. Milestone plan

| # | Milestone | Exit criteria |
|---|---|---|
| 0 | **Done:** investigation, decoders, docs | This report |
| 1 | Workspace skeleton plus `formats` crate: lev decode, r3d, bmp, hs, prefs in Rust | Golden tests: all 30 `.lev` and 136 `.r3d` parse, matching the Python tools |
| 2 | SDL3 window, GPU pipeline, model viewer with D3D6 lighting emulation | Renders `n3d2.r3d` (ship) and `retro64.r3d` with textures and masks |
| 3 | Port the XML-ish scanner and config interpreter (`0x419bb0` family); load `config.xml` and render the attract screen | Text effects and stars visible. Parser golden tests. |
| 4 | Main loop, timing (`0x40b720`), input (`0x411f70`), game-state machine (`0x40ba70`) | Menu navigation and pack selection |
| 5 | Ship, bullets, collisions, scoring, lives | Level 1 playable without aliens moving |
| 6 | Alien parser and stage machine (`0x403730`, `0x40ca60`, `0x40df90`) | Pack 1 playable end to end |
| 7 | Power-ups, boing ball, atoms, capture, perspective mode | All of pack 1 features |
| 8 | Audio: WAV mixer, XM music, volume keys | Parity checklist |
| 9 | High scores (`.hs`), remaining packs, screens | All 6 packs completable |
| 10 | Validation against the original, discrepancy log, tag `vanilla-baseline` | Signed-off checklist |

## 10. Unresolved questions

The tracked list is in `docs/open-questions.md`.
