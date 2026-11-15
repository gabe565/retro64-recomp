# Bugatron Gold v1.20a — analysis record (PRIMARY TARGET)

All addresses in this file refer to **this executable only**. Do not reuse them for other versions.

## Identity

| Item | Value |
|---|---|
| Distribution | `reference/Bugatron_Win_EN_Gold_120.zip` (sha256 `f3ee37e5…94f9`) → `buggold_120.exe` (sha256 `4519e880…669b`), Inno Setup, setup data 2.0.18, title "Bugatron Gold 1.20" |
| Game executable | `app/Bug3D.exe`, 253,952 bytes, sha256 `be581a63bccfc7dcc78a5c55cacc02e9f7aab9e5398540d2cc86a7aba758919e` |
| Window class / title | `Bugatron Gold v1.20a` (string at `0x43743c`) |
| Format | PE32 i386 GUI, MSVC 6 linker (6.0), Rich header present, image base `0x400000`, entry `0x42ba98` |
| Link timestamp | 2003-03-19 22:02:09 UTC |
| Packing / obfuscation | None on `Bug3D.exe`. `fmod.dll` is UPX-packed; `upx -d` unpacks it cleanly. |
| Launcher | `app/Bugatron.exe`, 24,576 bytes, sha256 `cf4ced81…82e2cd`, linked 2001-07-07. It checks the DirectX version (DirectDraw, DirectInput, and DirectMusic probes) and then runs `Bug3D.exe` with `WinExec`. It contains no game logic. |

## Sections (verified)

| Name | VA | VirtualSize | Notes |
|---|---|---|---|
| .text | 0x401000 | 0x33965 | code |
| .rdata | 0x435000 | 0x1a44 | IAT, float constants, D3D GUIDs |
| .data | 0x437000 | 0x5d12f4 | about 6 MB, mostly BSS: static object pools and level tables |
| .rsrc | 0x609000 | 0xe70 | 2 icons and the video-mode dialog 102 ("Play / Exit / Video Mode / Display Device / 3D Device") |

## Imports (verified, `inspect_pe_imports`, imphash `f84fdf12…0da102`)

- **DDRAW.dll**: `DirectDrawCreate`, `DirectDrawEnumerateA`. Direct3D is reached via `QueryInterface`. Only `IID_IDirect3D3` is present as a static GUID, so this is the DirectX 6 interface generation (IDirectDraw4 / IDirect3D3 / IDirect3DDevice3).
- **fmod.dll** (FMOD **3.61**, read from `FSOUND_GetVersion`): `FSOUND_Init`, `FSOUND_SetMaxHardwareChannels`, `FSOUND_Sample_Load/SetMode/Free`, `FSOUND_PlaySound`, `FSOUND_SetSFXMasterVolume`, `FMUSIC_LoadSong/PlaySong/SetPaused/SetMasterVolume/FreeSong`, `FSOUND_Close`.
- **WINMM.dll**: `timeGetTime`, `joyGetPos`.
- **USER32.dll**: window, message, and dialog APIs, plus `GetAsyncKeyState`, `GetCursorPos`, and `SetCursorPos`.
- **KERNEL32.dll**: `QueryPerformanceCounter/Frequency`, `Sleep`, `SetPriorityClass`, `SetThreadPriority`, `FindFirstFileA/FindNextFileA`, and file I/O.
- **SHELL32.dll**: `ShellExecuteA`, used for website links.
- **GDI32.dll**: `GetStockObject`.
- There is no DirectInput, DirectSound, or network import in `Bug3D.exe`.

## Shipped content

832 files were extracted across the three extracted versions. Gold 1.20 `app/` contains:

- `Bug3D.exe`, `Bugatron.exe`, `fmod.dll`, `README.TXT`, three `.url` files.
- 30 `*.lev` files (encoded config, level, pack, and high-score definitions). See `docs/formats/lev.md`.
- 9 `pack*.hs` binary high-score tables. See `docs/formats/misc.md`.
- `m/`: 136 `.r3d` models (see `docs/formats/r3d.md`) and their `.bmp` textures (24-bit, plus 8-bit paletted in places).
- `Fonts/`: per-glyph 16×16 24-bit BMPs for the WH, NJ, SCORE, POPUP, and LIVES faces, plus loading screens.
- `Graphics/`: 32×32 8-bit particle and star sprites (`eb`, `ebl`, `ec`, `ed`, `eg`, `em`, `ep`, `er`, `ey`).
- `Audio/`: 25 `.wav` files (8-bit mono 22,050 Hz PCM, as sampled) and 11 `.xm` FastTracker 2 modules.

## Verified behavior (static analysis)

The function map is in `docs/function-map/gold-1.20.md`.

- **WinMain-equivalent** `FUN_004076e0`:
  1. Enumerates DirectDraw drivers, devices, and modes.
  2. Reads `prefs`, which holds 3 × i32 indices: driver, device, mode.
  3. Shows dialog 102 and writes `prefs` back.
  4. Creates a 640×500 window, or a fullscreen popup window when a mode is chosen.
  5. Calls `r3d_Initialize` (`FUN_00424b90`).
  6. Runs a `PeekMessage` loop.
  - On the first idle iteration it calls `InitApp` (`FUN_004087d0`). After that, each iteration calls the per-frame function `FUN_00408940`, which calls `FUN_0040ba70`.
- **InitApp** `FUN_004087d0`:
  - Sets up the camera and light (`r3d_SetLightLocationColor(1.2, 1.2, -9.0, 1, 1, 1)`), with ambient 0 and specular on.
  - Sets filtering mode 3.
  - Seeds the C runtime random number generator with `timeGetTime()`.
  - Loads the defaults for `pack1.hs`, sets the current config to `config.xml`, and loads it.
- **Random numbers**: the generator is the MSVC CRT `rand()` (`_rand` at `0x42ab73`): `state = state*0x343FD + 0x269EC3; return (state>>16)&0x7FFF`. `srand` is at `0x42ab66`. Because the seed comes from wall-clock time, runs are not reproducible in the original.
- **Frame timing** `FUN_0040b720`, called every frame:
  - The time base is `FUN_00424440`, which converts QPC to **1/1024-second ticks**: `(qpc_low − base) * 1024.0 / freq_low`, truncated toward zero by `__ftol`.
  - It busy-waits with `Sleep(1)` until at least **4 ticks** have elapsed since the previous frame.
  - If more than **512 ticks** elapsed, it reuses the previous frame's delta.
  - It stores the delta as an integer tick count (`0x8429e0`, `0x8429e8`) and as a float `delta/1024` (`0x81d67c`).
  - **Gameplay uses a variable timestep.** There is no fixed-rate simulation tick.
- **Optional vsync-style limiter** `FUN_0041c180`:
  - It measures the median of 60 `WaitForVerticalBlank` intervals with QPC.
  - It then spins on QPC with `Sleep(0)` until one refresh interval has passed.
  - It also raises the process priority (`SetPriorityClass(0x20)`).
  - It is controlled by the "vsync on/off" toggle strings.
- **Game state machine**: the global at `0x4370a0` holds the mode (initial value 6). `FUN_0040ba70` dispatches on values 2, 3, 5, and 8. State 3 is a fade or transition that tears down all object lists and loads the next config. The state names are not yet assigned.
- **Input**:
  - The window procedure (`FUN_00407d10`) fills a per-virtual-key state array at `0x841a38`. A value of 1 means newly pressed and 2 means held, as used by `FUN_0041cb70`.
  - `FUN_00411f70` polls the bindings table at `0x437f9c`: ←, →, ↑, ↓, Ctrl, Space, Insert, F3 (edge-triggered), Delete, F1, F2, Esc, PgUp, PgDn, Home, End.
  - It also polls `joyGetPos(0)` and the mouse (cursor recentered to the screen centre, with left and right buttons via `GetAsyncKeyState(1/2)`).
  - Holding **F8 and F9 together** clears the object lists. This is likely a debug feature, not yet understood.
- **Level and data loading**:
  - Every `*.xml` name is opened as `<name>.lev`, NOT-ed, and JC-decompressed. See `docs/formats/lev.md`.
  - The parser is a hand-written tag and attribute scanner, not a real XML parser. It tolerates unclosed tags, duplicate attributes, and `<!...>` comment-outs. The config interpreter is `FUN_00419bb0`. The alien parser is `FUN_00403730`, the largest function at about 59 KB of pseudocode.
- **Rendering**: an in-house Direct3D 6 immediate-mode wrapper, `r3d_*`.
  - The model file header is "INDOTEK", which suggests a reusable engine library.
  - It uses fixed-function per-vertex lighting with 1 light, a global ambient level, specular highlights, and per-component D3D materials.
  - It loads BMP textures, with an optional second BMP used as an alpha mask (`r3d_SurfaceSetAlphaFromBMP`).
  - It supports point or linear filtering, no mipmaps, and fog states.
  - Alpha polygons are collected, with a cap at `_R3D_MAXALPHAPOLYS`, and drawn after opaque geometry.
  - There is a 2D blit path (`r3d_2DBegin/Blit/End`).
  - `r3d_Initialize` receives 75.0 (likely the vertical field of view in degrees) and 184800.0 (meaning unknown).
- **Screenshots**: the string `screenshot%d.bmp` is referenced in `FUN_00412930`, and `r3d_SurfaceToBMP` exists. The README says F12 writes `ss.bmp`. The key binding and the actual file name are not yet traced.

## Runtime observations

Run 1 (2026-11-15): Windows 11 arm64 VM with x86 emulation and dgVoodoo2, attached with `tools/frida/probe_frame_timing.js`, 480 frames sampled.

- **Confirmed:** after every call to `FUN_0040b720`, the delta at `0x8429e0` equals the change in the tick at `0x8429dc`, and the float at `0x81d67c` equals the delta divided by 1024. Zero exceptions.
- Deltas cluster at 16–17 ticks and 31–33 ticks, which is 60 Hz presentation with frequent dropped frames under emulation, about 43 fps on average. The range was 8 to 57 ticks. The 4-tick floor and the over-512 rule were not exercised.
- The game state at `0x4370a0` was 5 for the whole run, on the **main menu** (owner-confirmed).
- A one-shot read on the **pack-select screen** also gave state 5. The config-name string at `0x81d548` read `BUG1.xml`. InitApp sets it to `config.xml`, and run 2 showed it holds the *next* config. State 5 therefore appears to mean "a config is running" rather than one screen. Selecting a pack seems to change the current config name before gameplay begins, but that hasn't been traced in code.

Run 2 (2026-11-15): `tools/frida/probe_state_changes.js` during a session where the owner started pack 1, played a couple of levels, died, saw the high scores, and returned to the main menu.

| Frame | Tick | State | `0x81d548` | Event |
|---|---|---|---|---|
| 0 | 232996 | 5 | `BUG1.xml` | pack select (config2.xml) running |
| 961 | 258588 | 5 | `bug1.xml` | pack chosen: name taken from the pack manifest |
| 962 | 258592 | 3 | | transition begins |
| 1043 | 264202 | 2 | `bug2.xml` | load bug1.xml; its `NEXT_CONFIG` overwrites the name |
| 1044 | 264226 | 5 | | gameplay; several in-file levels pass with no state change |
| 5099 | 372936 | 5 | `highscores.xml` | game over sets the next config |
| 5453 | 382581 | 3 | | transition |
| 5531 | 386988 | 2 | `config.xml` | load highscores.xml |
| 5532 | 387051 | 5 | | high-score screen |
| 5814 | 395036 | 3 | | transition |
| 5885 | 398311 | 2 | `config2.xml` | load config.xml (main menu) |
| 5886 | 398317 | 5 | | main menu |

Conclusions (observed at runtime, consistent with the decoded `NEXT_CONFIG` chain config.xml → config2.xml → BUG1.xml and highscores.xml → config.xml):

- **`0x81d548` is the *next* config name**, not the current one. Loading a file sets it from that file's `NEXT_CONFIG`. Choosing a pack sets it from the manifest, and game over sets it to `highscores.xml`.
- **State 5** = a config is running (menus, gameplay, high scores). **State 3** = transition or fade-out, lasting 3,275–5,610 ticks (about 3 to 5.5 s) in these samples. **State 2** = load the next config, which takes exactly one frame and then enters 5. State 8 and the initial value 6 have not been seen yet.
- Level changes inside one file (bug1.xml holds 4 `<LEVEL>` blocks) happen within state 5.
- `config.xml` is the main menu, and `config2.xml` is pack select (`PROCESS_PACKS`).

## Not yet established for this version

See `docs/open-questions.md`.
