# The original `r3d` graphics wrapper — interface map (Bugatron Gold v1.20a)

Applies to `Bug3D.exe` sha256 `be581a63…758919e` only.

Evidence labels:

- **[S] static:** Ghidra survey `tools/ghidra/R3dSurvey.java` → `_work/ghidra/out/r3d_survey.json`, plus decompiled bodies.
- **[A] assembly:** read directly from disassembly.
- **[R] runtime:** `tools/frida/probe_r3d_boundary.js`. **Not yet run.** The VM was unavailable; see §8.

## 1. Summary

- `r3d` is an in-house Direct3D 6 immediate-mode wrapper ("INDOTEK" model format). It spans `0x41e950`–`0x42a3ff`: 90 functions, about 45 KB [S].
- **Calling convention: stdcall.** Every `r3d` function pops its own arguments with `ret N`, and the survey's stack purge matches the `ret` immediates for all 90 [S/A]. The runtime probe re-checks this on every call (§8).
- **50 of the 90 functions are called directly from game code** [S]. The rest are internal.
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
| 0x4210d0 | `r3d_RenderEnd` | 4 | 1 | R | **Flushes the deferred alpha-polygon list.** Its sort order must be ported exactly (`_R3D_MAXALPHAPOLYS`) |
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
| 0x4288b0 | sets flag `0x9f6798` | 4 | 1 | K | |

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

## 5. Recommendation

1. **Leaf replacement.** Replace the 54 device-, window-, and surface-facing functions (§3, R). Keep the 36 pure functions recompiled (K): model loading and queries, math, BMP decoding, timer, errors, file I/O.
   - Orchestration functions such as `r3d_ListLoad` stay recompiled. Their calls into replaced leaves then go through the runtime.
   - Model geometry and material records therefore stay byte-identical in guest memory, which is what gameplay reads.
2. **Replace through the existing override mechanism.** Each R address becomes an `rt_override` handled in Rust with the original stdcall signature: read arguments from the guest stack, `esp += 4 + purge`.
   - Overrides must preserve return values and out-parameters exactly as the original, including failure codes the game checks.
   - The probe in §8 records real argument tuples to test against.
3. **Port behaviour-bearing logic faithfully, in Rust.** Three pieces of logic inside the replaced functions affect what appears on screen rather than just how it is drawn:
   - alpha-polygon deferral and its sort in `r3d_Render3DModel` / `r3d_RenderEnd`;
   - material selection and colour modulation;
   - viewport clearing.
   
   These are ported from the disassembly with tests, not reinvented.
4. **COM objects for the six game-side DirectX methods** (§4).

## 6. Proposed graphics interface (for review; not implemented)

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

## 7. Open questions (graphics)

| # | Question | How to resolve |
|---|---|---|
| G1 | Pixel format of surfaces and textures (16-bit 565/555, or 32-bit) chosen by `r3d_Initialize` | Decode 0x424b90 / 0x428ee0; read surface descriptors at runtime |
| G2 | Light type and attenuation in 0x421a30 | Decode the `D3DLIGHT` structure built there |
| G3 | Does game code dereference the pointer returned by `r3d_SurfaceGet` or `TextureAnimateSurfaceLock`, and with what pitch? | Trace the 3+3 game callers; runtime probe argument samples |
| G4 | Exact alpha-polygon sort key and capacity | Port 0x41ff20 / 0x4210d0 with unit tests |
| G5 | Meaning of `r3d_Initialize` arguments 184800.0 and 75.0 | Decode 0x424b90 / 0x421fd0 |
| G6 | Is fog used? | xrefs to the fog render states |
| G7 | Do the matrices from 0x421680 feed any gameplay decision, or rendering only? | Data-flow trace from its 10 game callers |

## 8. Runtime validation (pending)

`tools/frida/probe_r3d_boundary.js` hooks all 90 entry points and the seven COM sites, and records:

- calls per function and per running config (menu, levels, high scores), split by caller region;
- a stack-cleanup check on every return (ESP must equal entry ESP + 4 + purge);
- the first argument tuple seen from game code.

Pass criteria:

1. Zero purge mismatches.
2. Only the functions marked "game > 0" are called from game code.
3. No `r3d` function is reached from CRT code except through the documented paths.
4. The COM sites fire as described.

**Status: not yet run.** The VM was offline after the proof-of-concept session. Run it with REA's `start_frida_session` against the running game and record the results here. Graphics replacement work starts only after it passes.
