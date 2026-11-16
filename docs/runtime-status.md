# Runtime status (full game, `crates/game`)

How far the recompiled game runs on the portable runtime, which imports and COM methods are implemented, and every known deviation from the original. Update this with each milestone.

## Current progress

`cargo run --release -p bugatron` translates 772 functions. It runs the original start-up, `r3d_Initialize` (display mode, flip chain, Z-buffer, Direct3D device, viewport, light, states), timer calibration, asset loading (288 textures), and the title-screen frame loop at a paced 60 Hz. Draw calls are recorded with full state; nothing is displayed yet, because the GPU renderer is the next step.

- Requires SDL3 (found through pkg-config; on macOS `brew install sdl3`).
- `SDL_VIDEODRIVER=dummy` runs headless.
- `--frames N` stops after N vertical blanks and prints import/COM call counts.
- `BUGATRON_TRACE=messages,sdl,draws` turns on diagnostic traces.

| Stage | Status |
|---|---|
| CRT start-up | runs (PoC P1) |
| Prefs, video dialog (headless) | runs. `save/prefs` = driver 0, device 1 (Direct3D HAL), mode 1 (640×480×32) |
| Enumeration, window creation | runs through original code |
| `r3d_Initialize`: display, surfaces, device | runs |
| Timer calibration (0x41c180) | runs. It measures 400000 QPC ticks (exactly 1/60 s at 24 MHz) |
| Asset loading, title frame loop | runs. Per frame: one Clear2, about 88 lit, textured draws, one back-buffer lock for 2D, one Flip |
| Rendering and presentation | next step (SDL3 GPU) |

## Implemented imports

- **KERNEL32:** process and module queries, environment, code pages, heap, `VirtualAlloc`/`VirtualFree`, TLS, critical sections, interlocked, standard handles, QPC/`Sleep` (replay clock), and exceptions (diagnostic stop). Files: `CreateFileA`, `ReadFile`, `WriteFile`, `SetFilePointer`, `GetFileSize`, `CloseHandle`, `GetFileType`.
- **USER32 dialogs:** `MessageBoxA` (stderr, returns IDOK), `DialogBoxParamA`, `EndDialog`, `SendDlgItemMessageA` (list-box messages 0x180/0x182/0x188/0x189/0x18c), `SetWindowTextA` (dialog only).
- **USER32 windows (`user.rs`):** `RegisterClassA`, `CreateWindowExA`, `ShowWindow`, `UpdateWindow`, `SetFocus`, `DestroyWindow`, `DefWindowProcA`, `PeekMessageA`, `TranslateMessage`, `DispatchMessageA`, `SendMessageA`, `PostMessageA`, `PostQuitMessage`, `GetClientRect`, `ClientToScreen`, `GetSystemMetrics(0/1)`, `LoadIconA`, `LoadCursorA`, `SetCursor`, `GetAsyncKeyState`, `GetCursorPos`, `SetCursorPos`. Also **GDI32** `GetStockObject` and **KERNEL32** `OutputDebugStringA`.
  - `CreateWindowExA` sends `WM_GETMINMAXINFO`, `WM_NCCREATE`, `WM_NCCALCSIZE`, and `WM_CREATE`.
  - The first `ShowWindow` sends `WM_SHOWWINDOW`, `WM_ACTIVATEAPP`, `WM_ACTIVATE`, `WM_SETFOCUS`, `WM_SIZE`, and `WM_MOVE`.
  - SDL events become `WM_KEYDOWN`/`UP` (or `WM_SYSKEYDOWN`/`UP`) with Win32 virtual keys and scan codes, `WM_MOUSEMOVE`, and `WM_[LR]BUTTONDOWN`/`UP`. Window close becomes `WM_SYSCOMMAND(SC_CLOSE)`, then `WM_CLOSE`, then `WM_DESTROY`.
- **WINMM:** `timeGetTime`, `joyGetPos` (returns `JOYERR_UNPLUGGED`: no joystick).
- **KERNEL32 files and scheduling:** `SetCurrentDirectoryA`/`GetCurrentDirectoryA` (the guest current directory, confined to the game directory), `FindFirstFileA`/`FindNextFileA`/`FindClose` (wildcards; game and save directories merged, in NTFS case-insensitive order), and `SetPriorityClass`/`SetThreadPriority` (no-ops).
- **FMOD 3.61** (`fmod.rs`): all 13 imports with their return contracts. Loads read the real files and return NULL when one is missing. No audio is produced yet (RD8).
- **DDRAW:** `DirectDrawEnumerateA`, `DirectDrawCreate` (primary driver only).

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
- Every frame it busy-waits (QPC + `Sleep(0)`) for one period since its previous exit, then calls `Flip(DDFLIP_WAIT)`.

Guest time is one exact counter that advances only when the guest waits:
- `Flip(WAIT)` and `WaitForVerticalBlank` advance it to the next 1/60 s refresh boundary.
- `Sleep(n)` adds n ms; `Sleep(0)` (a yield) adds 1 µs.
- QPC, `timeGetTime`, and message times all read it.

Consequences:
- Calibration measures exactly 1/60 s, and every frame starts on a refresh boundary.
- The limiter's 1 µs overshoot drops one frame every 16,667 frames, deterministically. Accumulated overshoot does the same on real hardware.
- After each wait, the host sleeps until real time catches up. Presentation never drives the cadence. If the host falls more than 250 ms behind, the backlog is forgiven.
- A busy-wait without `Sleep` (1,000,000 clock reads) follows host time until the next wait, with a one-time diagnostic.

## Tests

`cargo test -p bugatron-runtime-tests` drives imports and COM methods through the same dispatch path as recompiled code. Small recompiled guest helpers act as window procedures and callbacks. Coverage:
- files (including search and the current directory);
- FMOD contracts;
- enumeration and capabilities;
- pixel formats;
- surfaces (creation, descriptors, locks, flips, blits);
- the Direct3D device (defaults, legacy state mirroring, scenes, draw recording and snapshots, textures, clears);
- window messages and input;
- list boxes;
- the simulation clock, including the original's calibration and limiter loops reproduced against the runtime.

## Known deviations

| # | Deviation | Effect | Plan |
|---|---|---|---|
| RD1 | The video dialog is not shown. The original dialog proc runs against emulated list boxes and receives IDOK. | The driver, device, and mode come from `prefs` or the original defaults; the user cannot change them yet | Visible chooser driving the same dialog state |
| RD2 | Driver enumeration uses the `DirectDrawEnumerateA` fallback: one driver, "Primary Display Driver". The reference VM used `DirectDrawEnumerateExA` and listed two drivers, with dgVoodoo's names. | Driver list and names in the dialog | None needed; selection logic is unchanged |
| RD3 | The display-mode list is a fixed set (640×480 to 1280×1024, at 32 then 16 bpp), not the host's list. | Mode indices in a `prefs` file copied from Windows may refer to a different mode | By design (r3d-interface.md §9.3) |
| RD4 | The desktop mode is reported as 1024×768×32. | Only decides whether "Window" is offered | — |
| RD5 | The simulation runs at exactly one 60 Hz refresh per `Flip`. On a slower host it slows down instead of taking larger steps, as the original would. | Only when the host cannot sustain 60 simulated frames per second | By design (fixed cadence) |
| RD6 | The exact message sequence of `CreateWindowExA`/`ShowWindow` is modelled on Windows documentation, not traced from the VM. Non-client and paint messages are omitted. | The window procedure handles none of the omitted messages | Confirm against a VM message trace |
| RD7 | `TranslateMessage` generates no `WM_CHAR`, and `SetCursorPos` does not warp the host pointer. | The window procedure does not use `WM_CHAR` | — |
| RD8 | FMOD is silent: samples and songs are loaded but not played. | No audio | Audio milestone (SDL3 audio + libxmp) |
| RD9 | `Sleep(0)` takes exactly 1 µs of guest time. | Sets how often the limiter's overshoot drops a frame (1 in 16,667) | By design (deterministic) |
