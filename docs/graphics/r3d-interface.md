# The original `r3d` graphics wrapper — interface map (Bugatron Gold v1.20a)

Applies to `Bug3D.exe` sha256 `be581a63…758919e` only.

Evidence labels:

- **[S] static:** Ghidra survey `tools/ghidra/R3dSurvey.java` → `_work/ghidra/out/r3d_survey.json`, plus decompiled bodies.
- **[A] assembly:** read directly from disassembly.
- **[R] runtime:** `tools/frida/probe_r3d_boundary.js`, run 2026-11-15 in the VM (§8).

## 1. Summary

- `r3d` is an in-house Direct3D 6 immediate-mode wrapper ("INDOTEK" model format). It spans `0x41e950`–`0x42a3ff`: 90 functions, about 45 KB [S].
- **Calling convention: stdcall.** Every `r3d` function pops its own arguments with `ret N`, and the survey's stack purge matches the `ret` immediates for all 90 [S/A]. **Confirmed at runtime** for the 42 functions exercised, about 4.5 million calls, every one cleaning exactly the static purge [R].
- **50 of the 90 functions are called directly from game code** [S]. The rest are internal. At runtime, 29 of the 50 were called from game code during the menu, four level files, and high scores. None outside the static set was called from game code, and no `r3d` function was entered from CRT code [R].
- **Game code never reads or writes a global variable that `r3d` writes** [S: zero cross-references]. The coupling consists entirely of function calls, the data those calls return, and seven direct COM calls on DirectX objects obtained through `r3d` accessors (§4).
- **Game code consumes data from `r3d`:**
  - model bounds and geometry (collision and placement candidates);
  - screen width and height;
  - the high-resolution timer;
  - rotation matrices built with x87 `fsin`/`fcos`.
  
  These must stay bit-exact, so the functions that produce them stay recompiled.

## 2. Global objects and state [S/A]

| Global | Holds | Created in |
|---|---|---|
| `0xa06b50` | `IDirectDraw` (v1) | `DirectDrawCreate` in `r3d_Initialize` `0x424b90` |
| `0xa06afc` | `IDirectDraw4` | `QueryInterface(IID_IDirectDraw4)` |
| `0xa02adc` | `IDirect3D3` | `QueryInterface(IID_IDirect3D3)` |
| `0xa06b44` | primary (front) surface, `IDirectDrawSurface4` | `_r3d_CreateDirectX` `0x425780` |
| `0xa06b58` | back buffer | `GetAttachedSurface` on the front buffer |
| `0x9fa91c` | `IDirect3DDevice3` | `CreateDevice` on the back buffer |
| `0xa06b20` / `0xa06b5c` | render width / height | mode selection |
| `0xa02a58` (stride 0x428) / `0x9f5780` (count) | model list | `r3d_ListAdd3DModel` / `r3d_Load3DModel` |
| `0x9fa918` (stride 0x1fc) / `0x9d5770` (count) | material list (D3D material, texture surface, mask) | `r3d_ListAdd3DMaterial`, `r3d_ListLoad` |
| `0x9fa900` (stride 0x14) | model component table: vertex count, index count, vertex pointer, index pointer, material | `r3d_Load3DModel` |
| `0x9d5744` / `0x9faa20` | vertex pool (32-byte `D3DVERTEX`) / index pool (u16) | `r3d_Load3DModel` |
| surface table (0x1000 handles) | `r3d_Surface*` handles | `r3d_SurfaceCreate*` |
| `0x9fa928`, `0x9f679c`, `0xa02ad0`, `0xa06b48`, … | timer state (1/1024-second ticks) | `_r3d_InitTimer` `0x424520` |
| `0xa06b38` | "inside RenderBegin/RenderEnd" flag | `0x420f90` / `0x4210d0` |

## 3. Entry points

The tables below group all 90 functions by role. Columns:

- **purge:** bytes popped by `ret N`.
- **game:** number of distinct game-code callers.
- **rec:** recommendation. **K** = keep recompiled, **R** = replace with a Rust/SDL3 implementation, **C** = keep recompiled but provide the COM methods it calls.

### 3.1 Device, window, and enumeration (platform)

| Address | Name | purge | game | rec | Notes |
|---|---|---|---|---|---|
| 0x424730 | `r3d_Enumerate` | 12 | 1 | R | Fills driver, device, and mode tables used by WinMain and the video dialog |
| 0x424a10 | mode query | 28 | 1 | R | Selects driver, device, and mode; returns width, height, and depth |
| 0x428a90, 0x4290d0, 0x4291b0, 0x428a10 | enumeration internals | — | 0 | R | |
| 0x428b10, 0x428ee0 | DirectDraw and texture-format enumeration callbacks | 20 / 8 | 0 | R | Host→guest callbacks today; disappear with the replacement |
| 0x424b90 | `r3d_Initialize(hwnd, 184800.0, 75.0, drv, dev, mode)` | 24 | 1 | R | Arguments to be decoded during replacement work |
| 0x425780 | `_r3d_CreateDirectX` | 0 | 0 | R | Front/back/Z buffers, device, viewport |
| 0x426200, 0x426250, 0x4262b0 | window rectangle, move, `r3d_WindowSize` | 32 / 8 / 32 | 1 each | R | |
| 0x4264c0, 0x4265b0 | restore lost surfaces | 0 | 1 / 0 | R | |
| 0x426650 | shutdown | 0 | 2 | R | |
| 0x423b30, 0x423b50 | DirectDraw surface destroy | 0 / 4 | 0 | R | |
| 0x4235d0, 0x4235e0, 0x4235f0, 0x423600 | accessors: DDraw4, front buffer, back buffer, device | 0 | 2/1/1/1 | R | Return runtime COM objects (§4) |
| 0x4245f0 | screen info: width, height, depth, … | 28 | 3 | K | Reads globals only; the runtime keeps them at the guest's virtual 640×480 |

### 3.2 Frame and drawing (platform)

| Address | Name | purge | game | rec | Notes |
|---|---|---|---|---|---|
| 0x420f90 | `r3d_RenderBegin` | 4 | 1 | R | `BeginScene`, frame state reset |
| 0x4210d0 | `r3d_RenderEnd` | 4 | 1 | R | **Flushes the deferred alpha-polygon list.** Sort transcribed exactly; see §7.1 |
| 0x41ff20 | `r3d_Render3DModel(model, matrix)` | 8 | 2 | R | Draws each component with its material; defers alpha polygons |
| 0x420ed0 | `r3d_SetMatrix(matrix)` | 4 | 2 | R | |
| 0x4274b0 | `r3d_SetMaterial(handle)` | 4 | 1 | R | Also called by the game's text renderer (§4) |
| 0x421a30 | `r3d_SetLightLocationColor(x, y, z, r, g, b)` | 24 | 1 | R | InitApp: (1.2, 1.2, −9.0, 1, 1, 1) |
| 0x421c20 | `r3d_LightSetAmbient(r, g, b)` | 12 | 1 | R | |
| 0x423610 | `r3d_EnableSpecular(bool)` | 4 | 1 | R | |
| 0x423650 | `r3d_SetFilteringAndMipMapOptions(mode)` | 4 | 1 | R | Point/linear; no mipmaps |
| 0x425690 | `r3d_ViewportClear` | 8 | 0 | R | |
| 0x426420 | `r3d_PageFlip` | 8 | 2 | R | Flip, or `Blt` in windowed mode |

### 3.3 Resources: materials, textures, surfaces, 2D

| Address | Name | purge | game | rec | Notes |
|---|---|---|---|---|---|
| 0x41ebe0 | `r3d_ListAdd3DMaterial` (16 args) | 64 | 1 | K | Records material parameters in guest memory |
| 0x41ed70 | `r3d_ListLoad` | 8 | 1 | K | Orchestrates loading. Calls the replaced leaves below. |
| 0x4266e0 | `_r3d_MakeMaterial` (`CreateMaterial`) | 68 | 0 | R | |
| 0x427480, 0x427700 | material internals | 4 / 24 | 0 | R | |
| 0x41f090, 0x41f1e0, 0x41f2b0 | unload and release lists | 0 | 1 / 0 / 0 | R | Release device objects; model arrays stay in guest memory |
| 0x4221e0 | `r3d_SurfaceCreate(&h, w, h)` | 12 | 0 | R | |
| 0x4229a0 | `r3d_SurfaceCreateFromBMP` | 8 | 1 | R | Keeps the recompiled BMP decoder (0x4222b0, 0x422430) |
| 0x422750 | `r3d_SurfaceSetAlphaFromBMP` | 8 | 0 | R | |
| 0x422160, 0x422140 | `r3d_SurfaceDestroy`, destroy all | 4 / 0 | 3 / 0 | R | |
| 0x422bb0 | `r3d_SurfaceGet(h)` | 4 | 1 | R | Returns a pointer the game may dereference; see open question G3 |
| 0x422d90, 0x422dd0 | surface width / height | 4 | 1 | R | |
| 0x427bd0, 0x427d20 | `r3d_TextureAnimateSurfaceLock/Unlock` | 20 / 4 | 3 / 3 | R | **Game code writes texels.** The replacement must expose a guest-memory pixel buffer in the original format and upload it on unlock. |
| 0x422be0, 0x422cb0 | `r3d_2DBegin/End` (locks the back buffer) | 12 / 0 | 0 | R | |
| 0x427db0 | `r3d_2DBlit` (13 args) | 52 | 1 | R | |
| 0x422e10 | back buffer → new surface (screen capture) | 4 | 1 | R | **Reads rendered pixels back** into a guest surface (likely the transition effect) |
| 0x4231b0 | `r3d_SurfaceToBMP` (screenshot) | 8 | 1 | R | |
| 0x4222b0, 0x422430 | BMP file decoders (8-bit and 24-bit, uncompressed) | 20 / 16 | 0 | K | File parsing into guest buffers |
| 0x429ef0 | `_r3d_SmoothStretch` | 8 | 0 | K | Pixel math on guest buffers |

### 3.4 Model data, math, and queries (pure; consumed by gameplay)

| Address | Name | purge | game | rec | Notes |
|---|---|---|---|---|---|
| 0x41e950 | `r3d_ListAdd3DModel(file, scale)` | 8 | 1 | K | |
| 0x41f4c0 | `r3d_Load3DModel` | 8 | 0 | K | `.r3d` parser (docs/formats/r3d.md) |
| 0x421430 | model bounds query (8 args) | 32 | 1 | K | Bounding box and radius; **gameplay-relevant** |
| 0x421600 | model component count | 8 | 1 | K | |
| 0x421cc0 | model vertex pass (`__ftol`, writes vertex pool) | 4 | 1 | K | Likely applies per-model colours; data stays in guest memory |
| 0x4279c0, 0x427a10, 0x427ab0 | `r3d_Get3DModelMaterialCount/Handle/Geometry` | 8 / 12 / 28 | 3 / 2 / 3 | K | **Game reads vertices and indices** |
| 0x421640 | 3×4 identity matrix | 4 | 10 | K | |
| 0x421680 | axis-angle rotation matrix (x87 `fsin`/`fcos`) | 20 | 10 | K | **Transcendental fidelity matters here** (D7) |
| 0x421f60, 0x421fa0, 0x421fd0 | 4×4 identity, view, projection (uses render width/height) | 4 / 4 / 28 | 0 | K | Projection is the documented hook point for ultrawide (§6) |

### 3.5 Timer, errors, strings, file I/O (pure or import-backed)

| Address | Name | purge | game | rec | Notes |
|---|---|---|---|---|---|
| 0x424440 | timer read (1/1024 s ticks) | 0 | 4 | K | QPC is virtualised by the runtime; verified 600/600 frames |
| 0x424520, 0x424500 | `_r3d_InitTimer`, timer reset | 0 / 4 | 0 / 1 | K | |
| 0x424400, 0x424430, 0x4220e0, 0x423bd0 | error reporting and DirectX error strings (`MessageBoxA`) | — | 0 / 9 / 9 / 0 | K | `MessageBoxA` becomes a runtime diagnostic |
| 0x424670, 0x4246b0, 0x4246f0, 0x425660 | string and record helpers | — | 0 | K | |
| 0x4288c0–0x4289d0 | `CreateFileA` / `ReadFile` / `WriteFile` / `SetFilePointer` / `CloseHandle` wrappers | — | 0 | K | Runtime implements the imports, with case-insensitive lookup |
| 0x4288b0 | `r3d_` alpha-deferral disable: sets `0x9f6798` | 4 | 1 | K | Called once by game code, with 1 (§7.1) |

Totals: **R** 54 functions, **K** 36 (54 + 36 = 90). R functions still return values the game uses: handles, status codes, surface dimensions, enumeration tables (read by WinMain and the video dialog), and pixel pointers from lock and get calls. The replacement must reproduce each of these exactly. They are listed per function above and in G1/G3. Computed gameplay data (geometry, bounds, matrices, timer) comes only from K functions.

## 4. Game code that calls DirectX directly [S/A]

| Site | Game function | Object (accessor) | Method (vtable slot) | Purpose |
|---|---|---|---|---|
| 0x409e75 | 0x409b20 text renderer | device (0x423600) | `IDirect3DDevice3::DrawPrimitive` (28): `D3DPT_TRIANGLEFAN`, `D3DFVF_VERTEX` (0x112), 4 vertices | Draws each text glyph quad after `r3d_SetMaterial` |
| 0x40b739 | 0x40b720 frame timing | DDraw4 (0x4235d0) | `GetMonitorFrequency` (15) | Result unused; already implemented |
| 0x41c1e7 | 0x41c180 vsync limiter | DDraw4 (0x4235d0) | `WaitForVerticalBlank` (22) | Calibrates the refresh interval |
| 0x40baa1 | 0x40ba70 frame body | front buffer (0x4235e0) | `IsLost` (24) | Triggers surface restore |
| 0x40b893, 0x40b8cd, 0x40ba1a | 0x40b7c0 vsync toggle and HUD | back buffer (0x4235f0) | `Blt` (5) with `DDBLT_COLORFILL\|DDBLT_WAIT` (0x1000400) | Clears regions for the "vsync on/off" message |

**Recommendation: keep these game functions recompiled.** The runtime provides four COM objects through the replaced accessors. They implement exactly these six methods; any other slot stops with a diagnostic. This keeps the text renderer's vertex math exact, which matters because glyph positions come from game state.

## 5. Decision: COM-level emulation (owner, 2026-11-15)

**The graphics boundary is the DirectDraw 6 / Direct3D 6 COM surface, not the r3d wrapper.** This supersedes the earlier leaf-replacement recommendation; the R/K/C column in §3 is kept as a record of that analysis.

- **All 90 r3d functions stay recompiled**, like the rest of the game. That includes enumeration, the video dialog, format selection, BMP-to-texel conversion, material setup, alpha deferral and its sort, and error paths. None of it is hand-ported.
- **The runtime implements the COM methods r3d and game code call:** `IDirectDraw`/`IDirectDraw4`, `IDirectDrawSurface4`, `IDirectDrawClipper`, `IDirect3D3`, `IDirect3DDevice3`, `IDirect3DViewport3`, `IDirect3DLight`, `IDirect3DMaterial3`, and `IDirect3DTexture2`. The imports `DirectDrawCreate` and `DirectDrawEnumerateA` are also implemented. The census in §5.1 is about 60 methods.
  - As with Win32 imports, methods are implemented on evidence: an unimplemented slot stops the run with its interface and method name.
- **Fidelity target:** the original 32-bit path (§9) first, with the 16-bit path retained. Blending, colour quantisation, and texture formats follow D3D6 fixed-function semantics for the format codes the original code selects.
- **Overrides of individual r3d functions** are allowed only if the COM path proves impractical, each narrowly scoped and documented here. There are no parallel graphics implementations.
- **Modern features** (ultrawide, interpolation) are deferred. They remain possible at this boundary through `SetTransform(PROJECTION)` and the per-frame draw stream.

### 5.1 COM census (static, from vtable offsets in `decomp.c`)

| Object (global) | Interface | Slots called (vtable index → method) |
|---|---|---|
| `0xa06b50` | `IDirectDraw` | 0 QueryInterface (→ `IDirectDraw4`) |
| `0xa06afc` | `IDirectDraw4` | 0 QI, 2 Release, 4 CreateClipper, 6 CreateSurface, 10 FlipToGDISurface, 20 SetCooperativeLevel, 21 SetDisplayMode, 25 RestoreAllSurfaces; also 8 EnumDisplayModes and 15 GetMonitorFrequency through locals |
| `0xa06b98` | `IDirectDraw` (enumeration temporary) | 0 QI, 2 Release, 11 GetCaps |
| `0xa02adc` | `IDirect3D3` | 2 Release, 4 CreateLight, 5 CreateMaterial, 6 CreateViewport, 8 CreateDevice, 10 EnumZBufferFormats, 11 EvictManagedTextures; 3 EnumDevices through a local |
| `0x9fa91c` | `IDirect3DDevice3` | 2 Release, 5 AddViewport, 8 EnumTextureFormats, 9 BeginScene, 10 EndScene, 12 SetCurrentViewport, 16 Begin, 18 Vertex, 20 End, 22 SetRenderState, 24 SetLightState, 25 SetTransform, 29 DrawIndexedPrimitive, 38 SetTexture, 40 SetTextureStageState; 28 DrawPrimitive from game code (§4) |
| `0xa06b44` | front `IDirectDrawSurface4` | 2 Release, 5 Blt, 11 Flip, 12 GetAttachedSurface, 24 IsLost, 27 Restore, 28 SetClipper |
| `0xa06b58` | back-buffer surface | 2, 3 AddAttachedSurface, 24, 25 Lock, 27, 32 Unlock |
| `0xa02a64` | Z-buffer surface | 2, 24 IsLost, 27 Restore |
| `0xa06b18` | `IDirect3DViewport3` | 2, 13 AddLight, 17 SetViewport2 |
| `0xa06b34` | `IDirectDrawClipper` | 2, 8 SetHWnd |
| `0xa06b2c` | `IDirect3DLight` | 2, 4 SetLight |
| locals | texture surfaces, `IDirect3DTexture2`, `IDirect3DMaterial3` | surface 1/2/3/5/15/22/24/25/28/32; texture 3 GetHandle, 5 Load; material 3 SetMaterial, 5 GetHandle (typing per call site to be confirmed at runtime) |

Typing of the local-pointer calls is inferred from slot ranges; the runtime's named stubs confirm each one as it is reached.

## 6. Graphics interface (superseded by §5; the implemented design is in renderer.md)

```
recompiled game ──(stdcall overrides)──► r3d facade (Rust)
                                            │  records per frame:
                                            ▼
                                 FrameCommands { clear, set_matrix, set_material,
                                                 draw_model(handle, matrix, material state),
                                                 draw_prim(fan, vertices), blit_2d, alpha list,
                                                 texture uploads, readbacks }
                                            │
                                            ▼
                              renderer (SDL3 GPU): D3D6 fixed-function emulation
```

- **Fidelity first.** The default renderer draws every recorded frame once, at the guest's virtual 640×480 with the original projection, then scales to the window with letterboxing. Lighting follows the D3D6 fixed-function equations:
  - one light;
  - ambient, diffuse, specular, and emissive terms;
  - per-vertex (Gouraud) evaluation.
  
  Textures, alpha masks, filtering, and the alpha-polygon order come from the recorded state.
- **Gameplay isolation.** Game code only ever sees guest-visible values: the virtual 640×480 screen size, original handles, and original return codes. Output resolution and refresh rate never reach gameplay.
- **Ultrawide (later):** a renderer option that widens the horizontal field of view by rebuilding the projection from the recorded parameters. Pillarboxing is the vanilla default. HUD and 2D blits stay in the 640×480 space.
- **High frame rate (later):**
  - The 60 Hz simulation stays unchanged.
  - The renderer interpolates between the last two recorded `FrameCommands` by matching draws on (model handle, call order) and interpolating their matrices.
  - Frames whose command structure differs fall back to the most recent frame.
  - This needs no change to recompiled code.
- **CPU pixel paths:** locked textures, `2DBegin` back-buffer access, `SurfaceGet` pointers, and the capture in `0x422e10` use guest-memory buffers in the original pixel format.
  - Uploads happen on unlock.
  - Readback captures the rendered virtual frame, so the screen-capture effect still works.

## 7. Graphics questions

| # | Question | Status and finding |
|---|---|---|
| G1 | Pixel formats | **Mostly resolved.** `r3d_Initialize` (0x424b90) picks format codes from the display depth and the driver's enumerated texture formats: opaque textures → `0xa06b08`, alpha-masked textures → `0x9fa910`, back buffer → `0xa02a50`. Codes: 0 = RGB565, 1 = ARGB1555, 2 = ARGB4444, 3 = ARGB8888 (4 and 5 are back-buffer variants). **Observed** in the VM (dgVoodoo2, 32-bit display, 640×480): opaque 0 (565), alpha 3 (8888), back buffer 4. **Open:** which display depth the vanilla baseline emulates. A 16-bit display, typical in 2001 and mentioned in the README ("800x600x16"), takes different branches. This is an owner decision; see §9. |
| G2 | Light | **Resolved.** One `D3DLIGHT_DIRECTIONAL` (type 3) with direction −normalize(x, y, z) of the `r3d_SetLightLocationColor` arguments, InitApp's (1.2, 1.2, −9.0), colour (r, g, b), attenuation0 = 1.0. The normalized direction and colour are also kept CPU-side (`0x9faa14..`, `0x9fa934..`). |
| G3 | CPU pixel access | **Resolved.** `r3d_TextureAnimateSurfaceLock(material, &w, &format, &pitch, &bits)` returns the material's format code and a locked pointer and pitch. Game code (0x41ae80, 0x41b6a0, 0x417850) reads and writes texels with format-specific packing for codes 0–3. The replacement must return the original format code and a guest buffer in exactly that layout, converting on unlock. `r3d_SurfaceGet` pointers are dereferenced by game code in 0x417850. |
| G4 | Alpha-polygon deferral and sort | **Resolved (static, from assembly).** See §7.1. |
| G5 | `r3d_Initialize(hwnd, 184800.0, 75.0, …)` | **Resolved.** 184800.0 = far clip distance and fog end (fog start = 92400). 75.0 = vertical FOV in degrees. Projection = 0x421fd0(near 1.0, far 184800, fov 75°·π/180, aspect from the render size). World and view are identity via `SetTransform`. |
| G6 | Fog | **Resolved: never enabled.** The only writer of the fog-enable flag `0x9b573c` sets it to 0 at init; `FOGENABLE` is set FALSE, fog colour 0. |
| G7 | Do `0x421680` matrices feed gameplay? | **Partly resolved.** Callers build the camera tilt (0x412930 / 0x415380, π/4 about X for the "perspective" view), star and object render matrices stored in object structs (+0x44, +0xb0), and menu rotations. The collision-relevant path (0x40f2a0) derives object extents from model bounds and geometry, not from these matrices. Since FSIN/FCOS now match hardware (feasibility §7.10), the remaining risk is limited to `X87_NOTE_EXT64` cases. |

### 7.1 Alpha polygons (G4)

Evidence: `r3d_Render3DModel` 0x41ff20, `r3d_RenderBegin` 0x420f90, `r3d_RenderEnd` 0x4210d0 (sort loop 0x421140–0x4212a0, read as assembly), `_r3d_MakeMaterial` 0x4266e0, 0x4288b0, and its game caller 0x412930.

**Camera-relative world space.** `RenderBegin(camera)` stores the camera translation (`camera[9..11]`) in `0x9fa938..0x9fa940` and sets the VIEW transform to the camera rotation with zero translation. `Render3DModel` sets WORLD = model rotation with translation `t − camera`. Everything is drawn relative to the camera. `RenderBegin` also resets the alpha count `0x9b2848` and the triangle counter `0xa02a6c`.

**Which polygons are deferred.** For each component of a model (record stride 0x428 at `0xa02a58`), the polygon is deferred if material `+0x164` (an int "translucent" flag, compared as float `== 1.0`) is set **and** `0x9f6798 == 0`. Otherwise the component is drawn immediately with `DrawIndexedPrimitive(TRIANGLELIST, 0x112, …, flags 8)`. `_r3d_MakeMaterial` sets `+0x164 = 1` when the material's alpha is < 1.0, or when its 13th and 15th arguments are both 1 (meaning not yet identified).

**Deferral is off during gameplay.** `0x9f6798` is written only by `r3d_Initialize` (0) and 0x4288b0. The only game call is `0x4288b0(1)` in the gameplay render 0x412930, right after `RenderBegin`, and nothing resets it. So deferral and the sort apply only to frames drawn before the first gameplay frame: title and menus after start-up. From then on, translucent components draw in model order through the immediate path. [S; worth one runtime confirmation of the flag value at the menu after a game ends.]

**Deferred record.** Each entry is 3 vertices × 32 bytes at `0x98de58 + 0x60·i`. Positions are transformed on the CPU: `R·v + (t − camera)` in float, with the evaluation order of 0x41ff20. Normals are rotated (type 0); colour, specular, and UV are copied. A parallel pair at `0x9b2850 + 8·i` holds (vertex type = model `+0x24`, material index = (material ptr − `0x9fa918`) / 0x1fc).

**Capacity.** `_R3D_MAXALPHAPOLYS` = 1500 (`0x5db < count` checks).
- In `Render3DModel`, the check runs before each push and again after each increment. Reaching 1500 logs "_r3d_AlphaPolyIndex>_R3D_MAXALPHAPOLYS" through 0x424400 and returns 0, partway through the model; the game then shows its "Error" box.
- `RenderEnd` with count ≥ 1500 returns 0 **without drawing or calling `EndScene`**.

**Sort.** It runs whenever `0xa06af4 == 1`. That flag is set to 1 by `r3d_Initialize` and after every successful `EndScene`, and is never cleared, so the sort always runs. Raw-byte hits at 0x421d0c / 0x422013 are misaligned coincidences.

```
idx[k] = k                                  // 0x9b10d8, int[1500]
for p in 0 .. n-2:                          // exchange sort, not stable
  for q in p+1 .. n-1:
    A = idx[p]; B = idx[q]
    c(P, axis) = ((v0 + v1) + v2) * 0.33333334f   // binary64 at PC53; constant 0x3eaaaaab @ 0x4353e0
    DA = (ax·f32(ax) + ay·f32(ay)) + f32(az)·f32(az)
    DB = (bx·f32(bx) + by·f32(by)) + bz·bz
    if DA < DB (ordered; fcompp + test ah,0x41): swap idx[p], idx[q]
```

The resulting order is far to near, by squared distance of the triangle centroid from the camera position (not view depth). The comparator is **asymmetric**. Because of the `fst`/`fstp` spills to float slots, A's x and y terms multiply a register value by its float-rounded copy, A's z term squares the float-rounded value, and B's z term squares the unrounded value. Ties and NaN never swap. The port must transcribe this sequence exactly (binary64 arithmetic, f32 rounding at exactly these points). Because the comparator is asymmetric, this is not a valid ordering for library sorts.

**Flush.** For each `idx` entry in order:
1. `0x4274b0(material)`.
2. `Begin(TRIANGLELIST, type 0 ? 0x112 (D3DVERTEX) : 0x1e2 (D3DLVERTEX), type 0 ? 8 : 0x18)`, then three `Vertex` calls and `End(0)`.

This happens after WORLD is reset by 0x421640 with the camera translation added back, then set by 0x420ed0, which undoes the camera-relative transform. Flags: 8 = DONOTUPDATEEXTENTS, 0x10 = DONOTLIGHT. Then `EndScene`, and `*out = triangle count`.

**Verification plan for the port.** Run a differential test against the recompiled original 0x4210d0. Fill the guest buffers with synthetic and recorded alpha lists, including ties, a near-equal centroid, NaN, and n = 1, 2, and 1499. Stub the device COM methods to log the draw order, and compare the order with the Rust port's.

## 8. Runtime validation [R]

`tools/frida/probe_r3d_boundary.js` was attached to the original game in the Windows 11 arm64 VM (x86 emulation, dgVoodoo2). It hooked all 90 `r3d` entry points.

**Run 1:** 15,000 frames. The owner played through the menu (`config.xml`), pack select (`config2.xml`), level files `bug1.xml` and `bug5.xml`–`bug7.xml`, and the high-score screen.

**Run 2:** 3,900 frames, used to calibrate the stack-cleanup check. Frida reports ESP in `onLeave` after its trampoline has consumed the return address, so the expected delta equals the purge. This was established from 9 purge-0 functions and holds uniformly.

| Criterion | Result |
|---|---|
| Stack cleanup equals static purge | **Pass**: 42 functions, about 4.5 million calls, every delta equal to the static purge. No function shows more than one delta value. |
| Game code calls only the statically identified functions | **Pass**: 29 called from game code at runtime, all within the static set of 50 |
| No `r3d` entry from CRT code | **Pass**: zero |
| Game-side COM sites | The 3-byte `call [reg+disp8]` sites can't be hooked by Frida. The accessors that hand out the objects were measured instead (below). |

Hot paths, per rendered frame on average in run 1:

| Function | Calls per frame |
|---|---|
| `0x421680` rotation matrix (x87 `fsin`/`fcos`) | about 127 |
| `r3d_Render3DModel` | about 87 |
| `r3d_SetMaterial` from game code | about 86 |
| Device accessor `0x423600` (the text renderer's direct `DrawPrimitive`) | about 86 |
| `r3d_SetMaterial` internally (via 0x427480) | about 100 |
| `r3d_SetMatrix` | about 53 |
| identity matrix | about 63 |
| timer `0x424440` | about 17 |
| `RenderBegin`, `RenderEnd`, `PageFlip`, `ViewportClear` | once |

Observed behaviour relevant to the replacement:

- **Texture locking:** `r3d_TextureAnimateSurfaceLock/Unlock` were called 2,909 / 2,897 times from game code. Game code really does write texels into locked textures (G3).
- **2D path:** `r3d_2DBlit`, surface width/height, and `0x421cc0` were each called 800 times from game code. `r3d_SurfaceCreateFromBMP` was called 45 times from game code at runtime, beyond level loading.
- **Lost surfaces:** pausing and resuming the VM made the game lose its DirectX surfaces. The restore path ran for real: `0x4264c0` → `0x4265b0` → `0x425780` (`_r3d_CreateDirectX`), followed by material and texture rebuilding.
- **Not exercised:** the back-buffer accessor `0x4235f0` (the vsync HUD's `Blt` path), screen capture `0x422e10`, `r3d_SurfaceToBMP`, and the window-move functions. These are covered by static analysis only.

Raw data stays local in `_work/runtime/r3d-boundary*.json`, since it contains run-specific pointers.

**Conclusion:** the documented boundary holds at runtime. Graphics replacement may proceed under the leaf-replacement plan (§5), once G1–G7 are resolved for each function being replaced.

## 9. Decision: display depth (G1)

**Decided (owner, 2026-11-15):**
- The default is **32-bit display depth**, matching the original 32-bit path observed in the VM.
- 16-bit (the README's 800x600x16) remains an available original mode.
- Both modes run the original format selection, pixel conversion, blending, and rounding, with no artificial limits.
- The 32-bit path is implemented first. Captures of both modes will be compared with the original once both exist.

### 9.1 Evidence that 32-bit is a native mode of the original [S]

Everything below is game code in `Bug3D.exe`. dgVoodoo2 only supplies what the driver reports: the mode list and the device's render bit depths.

1. **Mode enumeration accepts 32-bit.** 0x428ee0 is the `EnumDisplayModes` callback (it formats `"%ld x %ld x %ld"` @ 0x43b878 as width × height × bpp). It filters each mode as follows:
   - **Device support:** keeps 32 / 24 / 16 bpp only if the Direct3D device's `dwDeviceRenderBitDepth` (device desc `+0xd8`) has `DDBD_32` (0x100) / `DDBD_24` (0x200) / `DDBD_16` (0x400).
   - **8-bit:** always rejected.
   - **Channel layout:** keeps 16-bit only as 565 or 555, and 24/32-bit only as RGB (masks ff0000/ff00/ff) or BGR (masks ff/ff00/ff0000).

   Accepted modes are appended to the device's mode list. The callback marks 640×480×16 as the list's preferred entry, but `0x424a10` / `r3d_Initialize` reset the cursor to the head before indexing, so that mark is unused.
2. **The user chooses the mode in the game's own dialog.** WinMain (around 0x4047xx) builds the driver, device, and mode name lists (0x424730), then shows dialog resource 0x66 (`DialogBoxParamA`, dialog proc 0x408280). It saves the choice as three ints in the `prefs` file. The README's 1.01 note says the game "remembers the last video selection". Without `prefs`, the default is the first enumerated mode that is 640 wide or 480 high, so it depends on the driver's enumeration order.
3. **The renderer handles 32-bit explicitly.** `r3d_Initialize` (0x424b90) reads the bpp and masks of the selected mode, or of the desktop for a windowed device, and maps them to the back-buffer format code `0xa02a50`:
   - 16 bpp: 565 → 0, 555 → 1.
   - 24 bpp: RGB → 2, BGR → 3.
   - 32 bpp: RGB → 4, BGR → 5.

   The VM's selected mode (driver 0, device 1, mode 38) is 32 bpp RGB, giving code 4.

The README line "Fixed a crash when the user's video card didn't support 800x600x16" (1.03) describes a crash fix in a pre-1.20 default, not a required mode.

### 9.2 What the display depth actually changes [S]

Texture formats do **not** depend on the display depth. `r3d_Initialize` chooses them from the formats `EnumTextureFormats` reports (callback 0x425400 fills six slots at `0x9fa800 + 0x24·k`):
- **Alpha textures** (`0x9fa910`) prefer 3 (8888), then 2 (4444), 1 (1555), 0 (565), 4, 5.
- **Opaque textures** (`0xa06b08`) prefer, on a hardware device (`+0x138 == 1`): 0 (565), 4, 5, 1, 3, then 2. On software emulation, 3 comes first.

The display depth changes only the **back-buffer format** (`0xa02a50`). That affects:
- frame-buffer quantisation of the rendered and blended output;
- the CPU back-buffer paths (`r3d_2DBegin` access, the screen capture in 0x422e10, `SurfaceGet`), which the game reads and writes in that format.

The texture formats offered are a property of the emulated driver. The runtime offers the VM's set by default (it contains 565 and 8888, since those were chosen). Exactly which formats dgVoodoo2 enumerated is still to be logged, so a "period driver" set can be added later for the 16-bit mode.

### 9.3 Runtime design consequence

- The runtime emulates one hardware Direct3D device ("Direct3D HAL") whose mode list includes 640×480 at 32 bpp RGB, plus the 16-bit 565 modes (800×600×16 among them).
- The original enumeration, dialog, and selection code runs unchanged. The chosen mode reaches the original format logic, and the renderer stores the back buffer in the resulting format code.
- **32-bit default:** with no `prefs` file, the default is whichever mode the original logic picks from the emulated enumeration order. To make 32-bit the default without changing game code, the emulated mode list puts 640×480×32 first, which is the original logic's own "first 640-wide mode" rule. The order is recorded as a runtime configuration, not a game-code change.
