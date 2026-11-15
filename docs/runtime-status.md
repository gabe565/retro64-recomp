# Runtime status (full game, `crates/game`)

How far the recompiled game runs on the portable runtime, which imports and COM methods are implemented, and every known deviation from the original. Update this with each milestone.

## Current progress

`cargo run --release -p bugatron` translates 772 functions. It runs the CRT, WinMain, `r3d_Enumerate`, and the video dialog. It then stops at the first window-creation import (`LoadIconA`, from 0x407b4e).

| Stage | Status |
|---|---|
| CRT start-up | runs (PoC P1) |
| Prefs file read and write | runs. `save/prefs` = driver 0, device 1 (Direct3D HAL), mode 1 (640×480×32) |
| DirectDraw/Direct3D enumeration (0x424730 → 0x428a90) | runs through the original code with emulated COM |
| Video dialog (0x66, proc 0x408280) | runs headless (see deviations) |
| Window creation, `r3d_Initialize`, rendering | not yet |

## Implemented imports

- **KERNEL32:** process and module queries, environment, code pages, heap, `VirtualAlloc`/`VirtualFree`, TLS, critical sections, interlocked, standard handles, QPC/`Sleep` (replay clock), and exceptions (diagnostic stop). Files: `CreateFileA`, `ReadFile`, `WriteFile`, `SetFilePointer`, `GetFileSize`, `CloseHandle`, `GetFileType`.
- **USER32:** `MessageBoxA` (stderr, returns IDOK), `DialogBoxParamA`, `EndDialog`, `SendDlgItemMessageA` (list-box messages 0x180/0x182/0x188/0x189/0x18c), `SetWindowTextA` (dialog only), `ShowWindow` (dialog only).
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
