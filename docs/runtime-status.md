# Runtime status (full game, `crates/game`)

How far the recompiled game runs on the portable runtime, which imports and COM methods are implemented, and every known deviation from the original. Update this with each milestone.

## Current progress

`cargo run --release -p bugatron` translates 772 functions. It runs the CRT, WinMain, `r3d_Enumerate`, the video dialog, and window creation, then enters `r3d_Initialize`. Inside it, the cooperative level, display mode, flip chain, and Z-buffer are set up. It stops at `IDirect3D3::CreateDevice`.

Requires SDL3 (found through pkg-config; on macOS `brew install sdl3`). For a headless run, set `SDL_VIDEODRIVER=dummy`.

| Stage | Status |
|---|---|
| CRT start-up | runs (PoC P1) |
| Prefs file read and write | runs. `save/prefs` = driver 0, device 1 (Direct3D HAL), mode 1 (640×480×32) |
| DirectDraw/Direct3D enumeration (0x424730 → 0x428a90) | runs through the original code with emulated COM |
| Video dialog (0x66, proc 0x408280) | runs headless (see deviations) |
| Window creation (0x407b4e) | runs. The original window procedure receives the creation and activation messages |
| `r3d_Initialize`: display mode, flip chain, Z-buffer | runs |
| Direct3D device, rendering | in progress |

## Implemented imports

- **KERNEL32:** process and module queries, environment, code pages, heap, `VirtualAlloc`/`VirtualFree`, TLS, critical sections, interlocked, standard handles, QPC/`Sleep` (replay clock), and exceptions (diagnostic stop). Files: `CreateFileA`, `ReadFile`, `WriteFile`, `SetFilePointer`, `GetFileSize`, `CloseHandle`, `GetFileType`.
- **USER32 dialogs:** `MessageBoxA` (stderr, returns IDOK), `DialogBoxParamA`, `EndDialog`, `SendDlgItemMessageA` (list-box messages 0x180/0x182/0x188/0x189/0x18c), `SetWindowTextA` (dialog only).
- **USER32 windows (`user.rs`):** `RegisterClassA`, `CreateWindowExA`, `ShowWindow`, `UpdateWindow`, `SetFocus`, `DestroyWindow`, `DefWindowProcA`, `PeekMessageA`, `TranslateMessage`, `DispatchMessageA`, `SendMessageA`, `PostMessageA`, `PostQuitMessage`, `GetClientRect`, `ClientToScreen`, `GetSystemMetrics(0/1)`, `LoadIconA`, `LoadCursorA`, `SetCursor`, `GetAsyncKeyState`, `GetCursorPos`, `SetCursorPos`. Also **GDI32** `GetStockObject` and **KERNEL32** `OutputDebugStringA`.
  - `CreateWindowExA` sends `WM_GETMINMAXINFO`, `WM_NCCREATE`, `WM_NCCALCSIZE`, and `WM_CREATE`.
  - The first `ShowWindow` sends `WM_SHOWWINDOW`, `WM_ACTIVATEAPP`, `WM_ACTIVATE`, `WM_SETFOCUS`, `WM_SIZE`, and `WM_MOVE`.
  - SDL events become `WM_KEYDOWN`/`UP` (or `WM_SYSKEYDOWN`/`UP`) with Win32 virtual keys and scan codes, `WM_MOUSEMOVE`, and `WM_[LR]BUTTONDOWN`/`UP`. Window close becomes `WM_SYSCOMMAND(SC_CLOSE)`, then `WM_CLOSE`, then `WM_DESTROY`.
- **WINMM:** `timeGetTime`.
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

### Surfaces

- Every surface's pixels live in guest memory in its DirectDraw format. The pitch is the row size rounded up to a DWORD. `Lock` returns a pointer into that memory.
- The primary surface takes the current display mode's format: XRGB8888 for 32-bit, RGB565 for 16-bit.
- The flip chain is circular, as in DirectDraw. `Flip` exchanges the pixel storage of the front and back buffers.
- A per-surface generation counter records CPU changes for the GPU renderer.
- Render targets drawn by the GPU are read back into guest memory before CPU access (`render::sync_for_cpu`). This is a no-op until the renderer lands.
- Blits between different pixel formats are not emulated; they stop with a diagnostic, since DirectDraw blits do not convert formats.

### Simulation clock (`clock.rs`)

Guest time = `Flip` count × 1/60 s plus the total of the game's `Sleep` calls. It is exact (rational) arithmetic, with no drift.
- QPC, `timeGetTime`, and message times all derive from it, so time is constant within a frame.
- Each `Flip` and `Sleep` then waits for host time to catch up (simulation pacing). Presentation is independent of the cadence.
- If the host falls more than 250 ms behind, the backlog is forgiven: the simulation slows rather than racing.
- If the game busy-waits on the clock (1,000,000 reads without a `Flip` or `Sleep`), guest time follows host time until the next frame, with a one-time diagnostic.

The capability records (`DDCAPS`, both `D3DDEVICEDESC`s) are the values the original saw in the reference VM. They are in `crates/runtime/src/ddraw_caps.rs`, generated from `tests/fixtures/ddraw-enumeration-vm-dgvoodoo.json`, which was captured with `tools/frida/dump_ddraw_enumeration.js`.

## Files

Guest paths resolve case-insensitively against the game directory (`--data`, read-only) and a save directory (`--save`, default `./save`). Writes are copy-on-write into the save directory, so original game files are never modified.

## Tests

`cargo test -p bugatron-runtime-tests` drives imports and COM methods through the same dispatch path as recompiled code. Small recompiled guest helpers act as window procedures and callbacks. Coverage: files, enumeration and capabilities, pixel formats, surfaces (creation, descriptors, locks, flips, blits), window messages and input, list boxes, and the simulation clock.

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
