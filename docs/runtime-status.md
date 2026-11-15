# Runtime status (full game, `crates/game`)

How far the recompiled game runs on the portable runtime, which imports and COM methods are implemented, and every known deviation from the original. Update this with each milestone.

## Current progress

`cargo run --release -p bugatron` translates 772 functions. It runs the CRT, WinMain, `r3d_Enumerate`, the video dialog, and window creation, then enters `r3d_Initialize`. It stops at `IDirectDraw4::SetCooperativeLevel`, the first method of the surface and device milestone.

Requires SDL3 (found through pkg-config; on macOS `brew install sdl3`). For a headless run, set `SDL_VIDEODRIVER=dummy`.

| Stage | Status |
|---|---|
| CRT start-up | runs (PoC P1) |
| Prefs file read and write | runs. `save/prefs` = driver 0, device 1 (Direct3D HAL), mode 1 (640×480×32) |
| DirectDraw/Direct3D enumeration (0x424730 → 0x428a90) | runs through the original code with emulated COM |
| Video dialog (0x66, proc 0x408280) | runs headless (see deviations) |
| Window creation (0x407b4e) | runs. The original window procedure receives the creation and activation messages |
| `r3d_Initialize`: surfaces, device, rendering | next milestone |

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
| `IDirect3D3` | EnumDevices |

The capability records (`DDCAPS`, both `D3DDEVICEDESC`s) are the values the original saw in the reference VM. They are in `crates/runtime/src/ddraw_caps.rs`, generated from `tests/fixtures/ddraw-enumeration-vm-dgvoodoo.json`, which was captured with `tools/frida/dump_ddraw_enumeration.js`.

## Files

Guest paths resolve case-insensitively against the game directory (`--data`, read-only) and a save directory (`--save`, default `./save`). Writes are copy-on-write into the save directory, so original game files are never modified.

## Known deviations

| # | Deviation | Effect | Plan |
|---|---|---|---|
| RD1 | The video dialog is not shown. The original dialog proc runs against emulated list boxes and receives IDOK. | The driver, device, and mode come from `prefs` or the original defaults; the user cannot change them yet | Visible chooser driving the same dialog state |
| RD2 | Driver enumeration uses the `DirectDrawEnumerateA` fallback: one driver, "Primary Display Driver". The reference VM used `DirectDrawEnumerateExA` and listed two drivers, with dgVoodoo's names. | Driver list and names in the dialog | None needed; selection logic is unchanged |
| RD3 | The display-mode list is a fixed set (640×480 to 1280×1024, at 32 then 16 bpp), not the host's list. | Mode indices in a `prefs` file copied from Windows may refer to a different mode | By design (r3d-interface.md §9.3) |
| RD4 | The desktop mode is reported as 1024×768×32. | Only decides whether "Window" is offered | — |
| RD5 | The full game's QPC, `timeGetTime`, and `Sleep` follow the host clock. | Timing is not yet the decided fixed 60 Hz cadence | Advance virtual time by 1/60 s per presented frame once `Flip` is emulated |
| RD6 | The exact message sequence of `CreateWindowExA`/`ShowWindow` is modelled on Windows documentation, not traced from the VM. Non-client and paint messages are omitted. | The window procedure handles none of the omitted messages | Confirm against a VM message trace |
| RD7 | `TranslateMessage` generates no `WM_CHAR`, and `SetCursorPos` does not warp the host pointer. | The window procedure does not use `WM_CHAR` | — |
