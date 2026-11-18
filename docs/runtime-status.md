# Runtime status (full game, `crates/game`)

How far the recompiled game runs on the portable runtime, which imports and COM methods are implemented, and every known deviation from the original. Update this with each milestone.

## Current progress

`cargo run --release -p bugatron` translates 772 functions. It runs the original start-up, `r3d_Initialize` (display mode, flip chain, Z-buffer, Direct3D device, viewport, light, states), timer calibration, asset loading (288 textures), and the title-screen frame loop at a paced 60 Hz. **The original title screen renders** through the recompiled code on SDL3 GPU (Metal on macOS). See `graphics/renderer.md`.

- Requires SDL3 (found through pkg-config; on macOS `brew install sdl3`).
- `SDL_VIDEODRIVER=dummy` runs headless.
- `--frames N` stops after N vertical blanks and prints import/COM call counts.
- `BUGATRON_TRACE=messages,sdl,draws` turns on diagnostic traces.
- `--dump-frames A,B --dump-dir DIR` saves front buffers as PNG.
- `--video-mode 1024x768x32` (or "1024 x 768 x 32") selects a display mode in the game's video dialog. The game saves it to `prefs`, as when a user picks it.
- `--input "600:F1,800:click@70x120,1000:CTRL+"` injects input at flip numbers: key taps, holds (`+`) and releases (`-`), and mouse `move@`/`click@`. It enters at frame boundaries through the same path as SDL events. Because the simulation is deterministic, a script replays identically.

**Interactive status (scripted).**
- F1 on the title opens "Select Level Pack" (all six packs render).
- Clicking "Free Levels" starts level 1.
- In a 100-second session with fire held and left/right sweeps: aliens dive and are shot, the score increases, and lives are lost. No unimplemented function was reached.

**Resolutions.** By default the game renders at the largest 4:3 mode that fits the display: 2616×1962 on a 3024×1964-pixel MacBook Pro display, at a full 60 Hz (900 vblanks in 15.9 s, 5.3 s CPU, 220 MB resident). The title screen also renders correctly and at a full 60 Hz at 1024×768, 1600×1200, and 2048×1536 (32-bit). 520 vblanks take about 9 s of wall time (8.7 s ideal) and about 2.8 s of CPU, including the per-frame back-buffer readback and upload.

| Stage | Status |
|---|---|
| CRT start-up | runs (PoC P1) |
| Prefs, video dialog (headless) | runs. `save/prefs` = driver 0, device 1 (Direct3D HAL), mode 1 (640×480×32) |
| Enumeration, window creation | runs through original code |
| `r3d_Initialize`: display, surfaces, device | runs |
| Timer calibration (0x41c180) | runs. It measures 400000 QPC ticks (exactly 1/60 s at 24 MHz) |
| Asset loading, title frame loop | runs. Per frame: one Clear2, about 88 lit, textured draws, one back-buffer lock for 2D, one Flip |
| Rendering and presentation | title screen renders (SDL3 GPU; renderer.md) |

## Implemented imports

- **KERNEL32:** process and module queries, environment, code pages, heap, `VirtualAlloc`/`VirtualFree`, TLS, critical sections, interlocked, standard handles, QPC/`Sleep` (replay clock), and exceptions (diagnostic stop). Files: `CreateFileA`, `ReadFile`, `WriteFile`, `SetFilePointer`, `GetFileSize`, `CloseHandle`, `GetFileType`.
- **USER32 dialogs:** `MessageBoxA` (stderr, returns IDOK), `DialogBoxParamA`, `EndDialog`, `SendDlgItemMessageA` (list-box messages 0x180/0x182/0x188/0x189/0x18c), `SetWindowTextA` (dialog only).
- **USER32 windows (`user.rs`):** `RegisterClassA`, `CreateWindowExA`, `ShowWindow`, `UpdateWindow`, `SetFocus`, `DestroyWindow`, `DefWindowProcA`, `PeekMessageA`, `TranslateMessage`, `DispatchMessageA`, `SendMessageA`, `PostMessageA`, `PostQuitMessage`, `GetClientRect`, `ClientToScreen`, `GetSystemMetrics(0/1)`, `LoadIconA`, `LoadCursorA`, `SetCursor`, `GetAsyncKeyState`, `GetCursorPos`, `SetCursorPos`. Also **GDI32** `GetStockObject` and **KERNEL32** `OutputDebugStringA`.
  - `CreateWindowExA` sends `WM_GETMINMAXINFO` (not for a popup without a sizing border, such as the fullscreen window), `WM_NCCREATE`, `WM_NCCALCSIZE`, `WM_CREATE`, `WM_SIZE` and `WM_MOVE`.
  - The first `ShowWindow` sends `WM_SHOWWINDOW`, `WM_ACTIVATEAPP`, `WM_ACTIVATE` and `WM_SETFOCUS`.
  - `DestroyWindow` sends `WM_DESTROY` only (no `WM_KILLFOCUS`).
  - Host focus changes (Cmd-Tab/Alt-Tab, clicking elsewhere) are delivered by the next `PeekMessageA`, as Windows sends them. Losing focus sends `WM_ACTIVATE(WA_INACTIVE)`, `WM_ACTIVATEAPP(FALSE)` and `WM_KILLFOCUS`. Regaining it sends `WM_ACTIVATEAPP(TRUE)`, `WM_ACTIVATE(WA_ACTIVE)` and `WM_SETFOCUS`.
    - A window in exclusive fullscreen (`DDSCL_FULLSCREEN | DDSCL_EXCLUSIVE`) is also minimised and restored, as Windows does for exclusive DirectDraw apps. Losing focus ends with `WM_SIZE(SIZE_MINIMIZED)`; regaining it ends with `WM_SIZE(SIZE_RESTORED, client size)`.
    - The game's window procedure (0x407d10) handles them itself. On `WM_KILLFOCUS` it pauses the music (`FMUSIC_SetPaused(1)`), clears its "active" flag 0x8429a8 and its key array. Its main loop (0x407c51) then stops updating and just polls messages.
    - `WM_SETFOCUS` resumes the music. It sets the active flag again only in windowed mode (0x8429a4 = 1, the "Window" choice). In fullscreen the flag follows `WM_SIZE` (active unless minimised), so the restore is what resumes the game.
    - Surfaces are never lost, which differs from real exclusive mode, where an Alt-Tab loses them and the game restores them.
    - The audio output is muted while the window is unfocused (`audio.md`).
    - While unfocused, `PeekMessageA` waits up to 10 ms for host events instead of spinning. No guest time passes either way.
    - Scripted runs (`--input`) ignore host focus.
    - `--run-in-background` (off by default) also ignores host focus, so the game keeps running and playing sound while another application is active. The original paused, so this is an option and not the default. It is implemented in the runtime only.
  - `--input` also takes `QUIT`, the host's close request (`WM_SYSCOMMAND(SC_CLOSE)`).
  - **Presentation:** when the game takes exclusive fullscreen (`SetCooperativeLevel(DDSCL_FULLSCREEN | DDSCL_EXCLUSIVE)`), the host window goes fullscreen on its current display: borderless, letterboxed, without changing the host's display mode. This is how the original presented itself.
    - `--windowed` starts in a window, and scripted runs stay windowed.
    - Alt+Enter, or Ctrl+Cmd+F on macOS, toggles fullscreen. These shortcuts are handled by the runtime and never reach the game.
  - SDL events become `WM_KEYDOWN`/`UP` (or `WM_SYSKEYDOWN`/`UP`) with Win32 virtual keys and scan codes, `WM_MOUSEMOVE`, and `WM_[LR]BUTTONDOWN`/`UP`. Window close becomes `WM_SYSCOMMAND(SC_CLOSE)`, then `WM_CLOSE`, then `WM_DESTROY`.
- **WINMM:** `timeGetTime`, `joyGetPos`: joystick 0 is the host's controllers through SDL's gamepad API (`gamepad.rs`), always connected and centred while none is attached, since the game stops polling after its first failure (0x98d500). Left stick or D-pad for position, South/East/West/North for buttons 1–4, and the right trigger is button 1 too. Scripted and headless runs report `JOYERR_UNPLUGGED`, as in the captures of the original.
- **KERNEL32 files and scheduling:** `SetCurrentDirectoryA`/`GetCurrentDirectoryA` (the guest current directory, confined to the game directory), `FindFirstFileA`/`FindNextFileA`/`FindClose` (wildcards; game and save directories merged, in NTFS case-insensitive order), and `SetPriorityClass`/`SetThreadPriority` (no-ops).
- **FMOD 3.61** (`fmod.rs`): all 13 imports, with sound effects and XM music on the software mixer, libxmp, and SDL3 audio. See `audio.md`.
- **DDRAW:** `DirectDrawEnumerateA`, `DirectDrawCreate` (primary driver only).
- **Rare paths (2026-11-16 audit):** all 120 imports of `Bug3D.exe` are now implemented. The last ten are reached only on rare paths:
  - **C runtime errors and teardown:**
    - `TerminateProcess` exits like `ExitProcess`.
    - `LoadLibraryA` fails, so the "Runtime Error!" box is skipped, as with `GetProcAddress`.
    - `IsBadReadPtr`, `IsBadWritePtr` and `IsBadCodePtr` check committed guest pages and translated functions.
    - `FlushFileBuffers` and `SetEndOfFile` work on files.
    - `SetStdHandle` and `SetEnvironmentVariableA` (`_chdir`'s `=C:`) are accepted.
  - **`ShellExecuteA`**, used by the menu-link action `LINK_BUY` (0x4103a0): it reads `website3.url` and passes its second token, `URL=http://www.bugatron.com/purchase.html`, which is neither a file nor a URL. So it fails with `SE_ERR_FNF` and nothing opens, as on Windows. Gold 1.20's shipped scripts define no `LINK=` items at all; only standard 1.51d's Reflexive scripts do. So this is unreachable with the game's own data and is implemented for completeness. Only a genuine `http(s)` URL would be opened with the host's browser.
  - Every COM method in the static census (r3d-interface.md §5.1) is implemented.

`GetModuleHandleA("DDRAW.DLL")` returns a module handle. `GetProcAddress` returns NULL for everything, so the game uses its `DirectDrawEnumerateA` fallback rather than `DirectDrawEnumerateExA`.

## Implemented COM methods (`crates/runtime/src/ddraw.rs`)

| Interface | Methods |
|---|---|
| all | QueryInterface (`IDirectDraw4` and `IDirect3D3` from DirectDraw; others return `E_NOINTERFACE` with a log line), AddRef, Release |
| `IDirectDraw` / `IDirectDraw4` | GetCaps, GetDisplayMode, EnumDisplayModes (DD4), GetMonitorFrequency (DD4) |
| `IDirectDraw4` | SetCooperativeLevel, SetDisplayMode (one of the enumerated modes), CreateSurface, CreateClipper, RestoreDisplayMode, FlipToGDISurface, RestoreAllSurfaces |
| `IDirect3D3` | EnumDevices, EnumZBufferFormats (16, 24, then 32-bit Z; the game stops at 16) |
| `IDirectDrawSurface4` | QueryInterface (itself), AddAttachedSurface, Blt (colour fill, depth fill, copy, nearest-neighbour stretch, `KEYSRC`/`KEYSRCOVERRIDE`), BltFast (`SRCCOLORKEY`), DeleteAttachedSurface, Flip, GetAttachedSurface, GetCaps, GetColorKey, GetPixelFormat, GetSurfaceDesc, IsLost, Lock, Restore, SetClipper, SetColorKey, Unlock |
| `IDirectDrawClipper` | SetHWnd, GetHWnd |
| `IDirectDraw4` (timing) | WaitForVerticalBlank (`DDWAITVB_BLOCKBEGIN`) |
| `IDirect3D3` | CreateDevice (HAL/RGB), CreateViewport, CreateLight, CreateMaterial, EvictManagedTextures |
| `IDirect3DDevice3` | AddViewport, EnumTextureFormats (the five reference formats), BeginScene/EndScene, SetCurrentViewport, Begin/Vertex/End, Get/SetRenderState (legacy texture states mirror onto stage 0), Get/SetLightState, SetTransform, DrawPrimitive, DrawIndexedPrimitive, SetTexture, Get/SetTextureStageState |
| `IDirect3DViewport3` | SetViewport2, GetViewport2, AddLight, DeleteLight, Clear2 |
| `IDirect3DLight` | SetLight |
| `IDirect3DMaterial3` | SetMaterial, GetHandle |
| `IDirect3DTexture2` | GetHandle, Load; created by `QueryInterface` on a surface |

The device starts with the documented Direct3D 6 defaults. Z is enabled when the render target has a Z-buffer attached. Each draw is recorded with a snapshot of everything it depends on: vertices and indices, render and texture-stage states, light states, transforms, the viewport, active lights, the selected material, and each texture's surface and generation.

### Surfaces

- Every surface's pixels live in guest memory in its DirectDraw format. The pitch is the row size rounded up to a DWORD. `Lock` returns a pointer into that memory.
- The primary surface takes the current display mode's format: XRGB8888 for 32-bit, RGB565 for 16-bit.
- The flip chain is circular, as in DirectDraw. `Flip` exchanges the pixel storage of the front and back buffers.
- A per-surface generation counter records CPU changes for the GPU renderer.
- Render targets drawn by the GPU are read back into guest memory before CPU access (`render::sync_for_cpu`). This is a no-op until the renderer lands.
- Blits between different pixel formats are not emulated; they stop with a diagnostic, since DirectDraw blits do not convert formats.

### Simulation clock (`clock.rs`)

The original paces itself against vertical blanks:
- At start-up it calibrates a refresh period from 60 `WaitForVerticalBlank` intervals timed with QPC (median; 0x41c180).
- Every frame it busy-waits (QPC + `Sleep(0)`) until one calibrated period has passed since its previous exit, then calls `Flip(DDFLIP_WAIT)`.

Guest time is one exact counter (no drift) that advances only when the guest waits or reads the clock:
- `Flip(WAIT)` and `WaitForVerticalBlank` advance it to the next 1/60 s refresh boundary.
- `Sleep(n)` adds n ms. `Sleep(0)` is a scheduling yield and adds nothing.
- Each QPC read takes one QPC tick (1/24,000,000 s), so busy-waits always terminate deterministically.
- QPC, `timeGetTime`, and message times all derive from this counter.

**Frame drops: an investigation.** An earlier version charged 1 µs per `Sleep(0)` and dropped one frame every 16,667. The cause was the virtual clock, not the original:
- The limiter's exit phase drifts by (overshoot − calibration shortfall) per frame. The overshoot is how far past the target the poll loop exits. The shortfall is how much the calibration under-measures the refresh, because time passes between the QPC after one wait and the QPC before the next.
- With an ideal calibration (no shortfall) and a 1 µs overshoot, the phase crept forward until a frame was missed.
- On real hardware both quantities are microsecond-scale overheads, and whether a machine ever drops frames depends on which is larger. The VM recording (600 frames, deltas of 7–75 ticks under x86 emulation) is too noisy to say what reference hardware did.
- Now each QPC read costs one tick. The calibration measures one refresh minus one tick, as real hardware measures slightly under. The limiter's exit settles at a fixed offset after each vblank.

Measured in the game over 1,236 frames, its frame delta (global 0x8429e0, in 1/1024 s) is 17 or 18 ticks: exactly the 1024/60 pattern, with no doubled frames. The regression test `limiter_and_flip_keep_one_frame_per_refresh` runs the original's calibration and limiter loops for 20,000 frames.

**Pacing.** After each wait, the host sleeps until real time catches up. Presentation never drives the cadence. If the host falls more than 250 ms behind, the backlog is forgiven.

**Limiter cost.** During gameplay the original's limiter polls about 90,000 times per frame. The cause:
- One early frame makes many clock reads before the limiter, which leaves its exit phase late in the refresh.
- The original's strict `>` test exactly cancels the calibration's one-tick shortfall, so that phase persists, and every later frame busy-waits the same amount. Real hardware behaves the same, but there the spin is free.

The runtime keeps this behaviour. QPC and `Sleep` take a fast import path, and calls are counted by index rather than by formatted name. That costs about 0.08 µs per poll (0.34 µs before), and gameplay runs in real time at about 34% CPU with identical poll counts.

**Busy-wait fallback.** A busy-wait on `timeGetTime` (1,000,000 reads without advancing time) follows host time until the next wait, with a one-time diagnostic.

## Tests

`cargo test -p bugatron-runtime-tests` drives imports and COM methods through the same dispatch path as recompiled code. Small recompiled guest helpers act as window procedures and callbacks. Coverage:
- files (including search and the current directory);
- FMOD contracts;
- enumeration and capabilities;
- pixel formats;
- surfaces (creation, descriptors, locks, flips, blits);
- the Direct3D device (defaults, legacy state mirroring, scenes, draw recording and snapshots, textures, clears);
- transform and lighting;
- the GPU backend, run offscreen on the main thread (`--test gpu`; skipped without a GPU): clears, pixel centres, depth, blending, texturing, colour key, CPU/GPU coherence, and RGB565.
- window messages and input;
- list boxes;
- the simulation clock, including the original's calibration and limiter loops reproduced against the runtime;
- audio: WAV decoding, channel allocation and stealing, resampled length, looping and `SetMode`, levels and pan, interpolation, clipping, music volume, pause and restart, cleanup, the FMOD import contracts (with a generated XM), and rendering the game's own XM (skipped without game files).

## Known deviations

| # | Deviation | Effect | Plan |
|---|---|---|---|
| RD1 | By design (owner's decision, 2026-11-16): the video dialog is never shown. The original dialog procedure still runs, against emulated list boxes, and receives IDOK, so its own logic picks the mode and writes `prefs`. The mode is chosen automatically (RD3) or with `--video-mode`. | The player cannot pick a mode in-game | — |
| RD2 | Driver enumeration uses the `DirectDrawEnumerateA` fallback: one driver, "Primary Display Driver". The reference VM used `DirectDrawEnumerateExA` and listed two drivers, with dgVoodoo's names. | Driver list and names in the dialog | None needed; selection logic is unchanged |
| RD3 | The display-mode list is a period-typical set (4:3 from 512×384 to 2048×1536 plus 5:4 1280×1024, at 32 then 16 bpp), followed by extra modes.<br><br>**Default:** the largest 4:3 mode that fits the host display in physical pixels (points × density), at 32 bpp, with height capped at 2160. It is offered as one more driver mode and selected in the game's own dialog (`display.rs`).<br><br>**Override:** `--video-mode WxHxBPP` can offer any size. The original's fullscreen projection assumes 4:3 (0x421fd0 fixes the aspect at 0.75), so non-4:3 sizes stretch, as they did on real hardware.<br><br>Presentation letterboxes or pillarboxes, never stretches. | Mode indices in a `prefs` file copied from Windows may refer to a different mode | By design |
| RD4 | The desktop mode is reported as 1024×768×32. | Only decides whether "Window" is offered | — |
| RD5 | The simulation runs at exactly one 60 Hz refresh per `Flip`. On a slower host it slows down instead of taking larger steps, as the original would. | Only when the host cannot sustain 60 simulated frames per second | By design (fixed cadence) |
| RD6 | **Traced 2026-11-17** (fullscreen, VM): creation, showing and closing now send what the original receives, in the same order, minus the messages the window procedure ignores: non-client, paint, `WM_WINDOWPOS*`, IME, `WM_GETICON`, and the registered and `WM_SETCURSOR`/`WM_NCHITTEST` traffic.<br><br>The window procedure (0x407d10) handles only `WM_CREATE`, `WM_DESTROY`, `WM_MOVE`, `WM_SIZE`, `WM_SETFOCUS`, `WM_KILLFOCUS`, `WM_CLOSE`, `WM_ACTIVATEAPP` (clears the key array), `WM_SETCURSOR`, `WM_GETMINMAXINFO` (minimum 320×260), key, mouse and `WM_SYSCOMMAND` messages.<br><br>**Not traced:** the focus-change sequence (still modelled on Windows documentation), because forcing the foreground away from the original in the VM ended its process. Windowed mode: `WM_SIZE` reports the outer size, since non-client borders are not modelled. | Only messages the window procedure ignores are omitted | Trace focus changes with a real Alt-Tab in the VM if focus behaviour is ever in doubt |
| RD7 | `TranslateMessage` generates no `WM_CHAR`. | The window procedure does not use `WM_CHAR` | — |
| RD11 | The mouse is captured (SDL relative mode) and drives a guest cursor by relative motion, 1 host point per guest pixel, like a mickey at Windows' default pointer speed. `SetCursorPos` moves that guest cursor.<br><br>This matters because the original reads the mouse relatively in gameplay (0x411f70: speed = \|x − centre\| × 0.0625, capped at 2, then `SetCursorPos(centre)`) and absolutely in menus (0x4103a0, which draws its own pointer).<br><br>Pointer acceleration is the host's (macOS), not Windows'. | Gameplay sensitivity is the same at every resolution, window size, and HiDPI scale. The menu pointer covers the screen more slowly at higher resolutions, as on the original | — |
| RD8 | Audio approximations AU1–AU5 (XM via libxmp; FMOD pan law and stealing per its documentation). A capture of the original confirms XM tempo and pitch; its music is 2.1 dB louder at volume 56/256 | See `audio.md` | Find the source of the level difference (FMOD music mixer or a second capture); effects capture |
| RD9 | Intentional: guest time is idealised. Every frame spans exactly one 1/60 s refresh, `Sleep(0)` takes no time, and a QPC read takes one tick. Real machines add jitter and, depending on their overheads, an occasional dropped frame. | Stable, drift-free 60 Hz; identical simulation on every host | By design (owner decision, 2026-11-15) |
| RD10 | Rendering approximations A1–A13 | See `graphics/renderer.md` | VM capture comparison |

## Roadmap to the vanilla baseline

| Item | Status |
|---|---|
| Side-by-side captures against the original (deterministic Frida harness; `testing/visual-comparison.md`) | First comparison done: same scene and oval ball; static content near-identical. Next: align QPC read counts for exact animated frames, then other resolutions and 16-bit |
| Interactive play: menus, input, starting a game, waves | Scripted play covers the menus, level 1, waves, mouse steering, lives, and the GAME OVER screen. Level clearing, high-score entry, and the return to the menu need a human player or a smarter bot |
| Windows (Vulkan or D3D12) and Linux (Vulkan) graphics validation | **Linux:** Debian 13 arm64 and x86_64 in OrbStack, with Vulkan on Mesa lavapipe. Tests pass; game frames match Metal within 4/255; a Vulkan Y-flip bug was found and fixed (`testing/linux-validation.md`). Not yet: hardware Vulkan drivers, an interactive session, Ubuntu 24.04.<br><br>**Windows:** DXIL shaders built from the same WGSL (renderer.md). The cross-built x86-64 kit, run in the Windows 11 VM on D3D12/WARP: all 94 tests pass, including `gpu` with `BUGATRON_REQUIRE_GPU=1`. Frames match Metal within 1–2/255 at 32 bpp, including gameplay 6000 frames in; title audio is bit-identical to macOS (`testing/windows-validation.md`). Not yet: a hardware D3D12 GPU, Vulkan on Windows, and interactive keyboard/mouse on Windows (the VM is too slow to play) |
| 16-bit display mode comparison | Pending (A6) |
| Audio: FMOD 3.61 on SDL3 audio + libxmp | Done (`audio.md`). Compared with a capture of the original's FMOD mix: tempo and pitch match; the original's music is 2.1 dB louder at volume 56/256 (low priority, open) |
| Performance: per-frame back-buffer readback at high resolution | Documented optimisation target; deferred (renderer.md) |
| Modern feature (deferred): adjustable mouse sensitivity | In the input layer only: scale host motion before it moves the guest cursor (`user::mouse_motion_rt`). The original game code is not modified. The default stays 1 point = 1 guest pixel (RD11). The owner found the default slightly low on macOS (2026-11-15) |
| Modern feature: keyboard alternatives and remapping | Done (`bindings.rs`): host keys and controller buttons are bound to the game's keys in front of it, so the game still receives its original key messages. Defaults keep every original key and add `=`/`-` and `]`/`[` for the volume keys, controller Start for pause and Back for exit. `config.toml` in the save folder overrides them. A settings overlay to edit them is next |
| Modern feature: settings overlay | `overlay.rs`, opened by the `settings` binding (F10 or controller Back by default; its last binding cannot be removed). The game is frozen while it is open: the guest thread waits inside the `Flip` that presented the last frame, the audio mixer is paused, and the pacing lag is forgiven on close, so no virtual time passes. Drawn on the CPU with the game's title font (`Fonts/nj*.bmp`) at the size of the game's text, and composited over the frame by the fixed-function pipeline (`render::gpu::present_overlay`). The difficulty (0x4370e4) is set directly on any screen, with the title's label written as the F3 handler does; the high scores are reached by an F2 tap after it closes, on the title screen (flag 0x8429d0), since changing screens happens only inside the game's input handling. The music and sound volumes (0x4370a8, 0x4370ac) are set directly and applied by the game's own 0x417210, called from the next `PeekMessageA` as the volume keys call it. Pages: main (resume, difficulty, music volume, sound volume, high scores, controls, quit) and controls (add, remove, reset bindings) |
| Packaging: finding the game | `--exe`/`BUGATRON_EXE` accepts `Bug3D.exe`, the original Inno Setup 2.0.18 installer (`buggold_120.exe`), or the download (`Bugatron_Win_EN_Gold_120.zip`). It tells them apart by content.<br><br>An installer or zip is unpacked in memory at startup (about 0.1 s, `inno` and `zip` crates), and its `{app}` files are served read-only as the game folder; nothing is written but saves. The unpacked files match innoextract's byte for byte (408 files, tested).<br><br>Without `--exe`, each search folder is checked for `Bug3D.exe`, then `buggold_120.exe`, then the zip, ignoring case. The folders are the current directory, the program's own folder, `%ProgramFiles(x86)%\Bugatron Gold` (Windows), and the folder the program was built from. The game folder defaults to the folder of the `Bug3D.exe` found. The exe's SHA-256 is checked at startup.<br><br>**Single-file test builds:** `cargo build --release -p bugatron --features embed-game` builds the whole game folder (about 11 MB, 408 files) into the binary. It then runs from any folder, with prefs and high scores in the per-user data folder. `--exe`/`--data` still override. For Windows: `tools/windows/build-kit.sh --features embed-game`. The binary contains the original game files, so it is for the owner's own testing only |
| Packaging: save location | Done. Prefs and high scores (`.hs`) go to the per-user data folder from `SDL_GetPrefPath("", "bugatron")` (`~/.local/share/bugatron` on Linux), printed at startup, or to `--save`. They are copied there from the game folder on first write (copy-on-write) |
| Windows arm64 build | Not started. Would avoid x64 emulation on Windows on ARM, including the VM |
