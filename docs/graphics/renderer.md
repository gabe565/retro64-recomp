# Renderer: Direct3D 6 on SDL3 GPU

How the runtime turns the game's Direct3D 6 calls into pixels, and every place where it approximates the original. The interface analysis is in `r3d-interface.md`; implementation status is in `../runtime-status.md`.

## Architecture

```
recompiled game + r3d wrapper
   │  IDirectDraw4 / IDirectDrawSurface4 / IDirect3DDevice3 … (COM, ddraw/)
   ▼
device state model (ddraw/d3d.rs) ── records Clear / Draw commands with full state snapshots
   │
   ├─ render/tnl.rs   Direct3D 6 transform & lighting on the CPU (deterministic, unit-tested)
   └─ render/gpu.rs   SDL3 GPU: rasterise, texture (stage 0), alpha test, blend, depth; present
```

- **Guest capabilities are not host capabilities.** What DirectDraw and Direct3D report to the game comes from `ddraw/caps.rs`: the reference VM's values, captured from the original. The SDL3 GPU backend only has to reproduce the behaviour of the commands the game actually issues. Its own limits never reach the guest.
- **Pixels live in guest memory.** Every surface has a guest-memory buffer in its DirectDraw format and pitch, so `Lock` exposes exactly what the game expects.
  - Render targets and Z-buffers also have GPU textures, keyed by guest storage so that `Flip` carries them along: RGBA8 for colour, `D16_UNORM` for the game's Z16.
- **Coherence.** Each storage records whether the GPU holds newer pixels than guest memory.
  - CPU access (`Lock`, CPU blits) flushes pending work, reads the pixels back, and converts them to the guest format.
  - CPU writes (`Unlock`, which bumps the surface generation) are uploaded before the next GPU use, unless that use begins with a full clear.
  - This keeps the original's semantics for the per-frame `r3d_2DBegin` lock and the screen capture in 0x422e10.
- **Execution.** Commands run at `EndScene`, before CPU access, and before presentation: one copy pass (geometry, changed textures, CPU-written targets), then render passes in command order.
- **Timing is separate from presentation.** The simulation's clock advances only at guest waits (`clock.rs`), and pacing sleeps on host time. Presentation uses a non-blocking present mode (mailbox, else immediate) and never paces the game. A later high-refresh renderer can present more often without changing simulation speed.
- **Presentation.** After each `Flip`, the front buffer is scaled into the window (aspect-preserving, letterboxed, linear filter). This is presentation only and is never visible to the guest.
- **Shaders.** `shaders/fixed.wgsl` is translated at build time by naga into MSL (Metal) and SPIR-V (Vulkan), following SDL3 GPU's binding conventions.

## Performance notes and optimisation targets

- **Per-frame readback.** The game locks the back buffer for its 2D overlay every frame (`r3d_2DBegin`, `DDLOCK_WAIT|DDLOCK_NOSYSLOCK`, not write-only). Each frame therefore reads the render target back to guest memory and uploads it again after `Unlock`. That is 2 × 12 MB per frame at 2048×1536×32, and it still runs at a full 60 Hz on the Apple M-series test machine.
  - This is the main optimisation target for future high-resolution or high-refresh rendering. Options: asynchronous readback, tracking which rows the CPU actually writes, or compositing CPU writes as an overlay when the game provably never reads the pixels.
  - Any such change must keep the original's lock semantics (the screen capture in 0x422e10 does read pixels). Deferred until the vanilla baseline is complete.
- **Resolution range.** Nothing in the renderer assumes 640×480. Surfaces, GPU textures, and presentation follow the display mode the game selects, up to 2048×1536 in the emulated mode list. True widescreen remains a later enhancement, because the original's fullscreen projection is fixed at 4:3.

## Direct3D 6 behaviour reproduced

| Area | Behaviour | Evidence |
|---|---|---|
| Transforms | Row vectors: `[x y z 1]·World·View·Projection`; the `D3DVIEWPORT2` clip volume maps to the viewport | `tnl.rs` tests |
| Rasterisation | Direct3D 6 pixel centres are at integer coordinates; vertices are shifted +½ pixel for the GPU's half-integer centres | `gpu` test `prelit_triangle_covers_pixels_by_direct3d6_centres` |
| Lighting | Directional, point, and spot lights; material emissive, ambient (× `D3DLIGHTSTATE_AMBIENT`), diffuse·N·L, and specular (half vector, `D3DLIGHT_NO_SPECULAR`, `SPECULARENABLE`) | `tnl.rs` tests |
| Colours | Lit and vertex colours are delivered as `D3DCOLOR` bytes; specular is added after texturing | `tnl.rs`, `gpu` tests |
| Texture stage 0 | `COLOROP`/`ALPHAOP` DISABLE, SELECTARG1/2, MODULATE(2X/4X), ADD, ADDSIGNED, BLENDDIFFUSEALPHA, BLENDTEXTUREALPHA; arguments with COMPLEMENT and ALPHAREPLICATE | `fixed.wgsl` |
| Textures | Texels expand to 8 bits per channel by bit replication. Colour-keyed texels (`COLORKEYENABLE` + `DDCKEY_SRCBLT`) are not drawn. Point or linear filtering; wrap, mirror, or clamp | `gpu` test `textured_quad_modulate_and_colour_key` |
| Alpha test, blend, depth | All `D3DCMPFUNC`s; all `D3DBLEND` factors including BOTHSRCALPHA/BOTHINVSRCALPHA; Z enable, write, and function; culling by screen-space winding | `gpu` tests |
| Formats | 32-bit targets are exact: XRGB8888 readback equals the rendered bytes. 16-bit targets truncate to RGB565 like the game's own packing | `gpu` tests |

## Approximations

| # | Approximation | Why / effect | How to check |
|---|---|---|---|
| A1 | Lighting follows the documented Direct3D fixed-function equations (camera space, local viewer, normals not renormalised). Direct3D 6's software T&L (d3dim) has not been compared bit for bit. | Possible small colour differences on lit models | Capture lit frames in the VM and compare |
| A2 | Lit colours are rounded to nearest when quantised to `D3DCOLOR` bytes. | ±1/255 per channel at worst | Same |
| A3 | A stage-0 operation that references a missing texture passes the diffuse colour and alpha through. | Matches the common hardware behaviour; Direct3D 6 left it driver-defined | — |
| A4 | Colours and texture coordinates are interpolated perspective-correctly. Some 2001 hardware interpolated Gouraud colour linearly in screen space. | Subtle shading differences on large, steep triangles | VM comparison |
| A5 | Bilinear filtering uses modern GPU precision. No mipmaps are used (the game disables them). | Sub-texel filtering differences | VM comparison |
| A6 | 16-bit display mode renders at 8 bits per channel and truncates to RGB565 at readback and present. There is no dithering (`DITHERENABLE` is ignored), and blending happens before quantisation. | 16-bit output can be smoother than period hardware | 16-bit captures (pending) |
| A7 | GPU rasterisation (top-left fill rule, sub-pixel precision) matches Direct3D's rules, but not any one 2001 card's precision. | Edge pixels can differ | VM comparison |
| A8 | Partial-rectangle `Clear2` stops with a diagnostic. | The game only clears the full target | — |
| A9 | Destination alpha is always 1, because the guest targets have no alpha channel. | Matches XRGB/RGB565 semantics | — |
| A10 | The SPIR-V (Vulkan) shader path shares one binding between the texture and its sampler (SDL's combined image sampler), and leaves out naga's Y flip because SDL's Vulkan backend already presents a Y-up clip space. Verified on Linux with Mesa lavapipe: the `gpu` tests pass and game frames match Metal to within 4/255 per channel (`testing/linux-validation.md`). Not yet run on a hardware Vulkan driver or on Windows | Vendor drivers could still differ | Run the `gpu` tests on hardware Vulkan and on Windows |
| A11 | Presentation scales with linear filtering and letterboxing. | Presentation only; guest pixels are unaffected | — |
| A12 | Lines and points are not emulated (they stop with a diagnostic). | Not used by the title or menus | — |
| A13 | Fog is not emulated; a warning is printed if the game enables it. | r3d sets `FOGENABLE` to 0 at init and never changes it (r3d-interface.md G6) | — |

## Tests

- `cargo test -p bugatron-runtime-tests --test tnl` covers transform and lighting.
- `cargo test -p bugatron-runtime-tests --test gpu` renders offscreen through the real backend and reads pixels back with `Lock`. It runs on the main thread (SDL's video subsystem requires that on macOS) and skips itself without a GPU.
- Diagnostics:
  - `bugatron --dump-frames A,B --dump-dir DIR` saves front buffers as PNG, through the same readback path.
  - `BUGATRON_TRACE=draws` summarises each frame's commands.

## Visual validation

The title screen renders through the original code. Frames 139 (logo zoom), 400, and 700 (settled title) were inspected. A side-by-side comparison with captures from the original in the VM is still to do (`../testing/visual-comparison.md`); it is the acceptance check for A1–A7.

**Oval boing ball (investigated).** The title ball's shape follows from the original projection, not from a rendering error:
- The ball is `m/chex.r3d`: one 523-vertex component, and every vertex lies exactly at radius 1.237. It is a true sphere.
- Its world matrix in frame 450 is a pure rotation (orthonormal rows, no scale), translated to (4.50, −1.30, 10.09).
- The projection is exactly the original's at 640×480: x = cot 37.5° = 1.3032, y = x / 0.75 = 1.7376, with standard viewport and clip volume.
- Under the 75° vertical FOV, a sphere that far off-axis (about 25°) projects to an ellipse stretched away from the screen centre. Its predicted bounding box is 113×104 px, centred at (509, 295).
- The ball's unlit lower side is near black, which makes it look more oval than the silhouette is.
- Same appearance at every resolution. The VM capture will confirm it directly.

`BUGATRON_TRACE=draws,worlds` prints the world matrix and vertex count of each draw.
