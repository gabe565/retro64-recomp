# `.r3d` model format ("INDOTEK 2")

Status: **layout verified** against Gold 1.20.

- All 136 `.r3d` files parse to exactly their file size.
- All 25,324 vertex normals are unit length.
- All indices are below their component's vertex count.
- Parser: `tools/r3d_dump.py`.
- Source functions: `r3d_ListAdd3DModel` (`FUN_0041e950`, a pre-pass that sums vertex and index counts) and `r3d_Load3DModel` (`FUN_0041f4c0`).

All values are little-endian.

| Offset | Type | Field |
|---|---|---|
| 0 | char[10] | `"INDOTEK 2\0"`. The loader checks `INDOTEK` and reads the version digit. |
| 10 | f32 × 3 | bounds min (x, y, z) — *field meaning inferred from values* |
| 22 | f32 × 3 | bounds max (x, y, z) — *inferred* |
| 34 | f32 | bounding radius — *inferred* |
| 38 | i32 | `n` = component count |
| 42 | u32 × n | vertex count per component |
| … | u32 × n | index count per component (always a multiple of 3, so triangle lists) |
| … | vertex × Σvcount | 32 bytes each: `x y z nx ny nz tu tv` (f32). This is identical to `D3DVERTEX`, so the engine lights vertices itself through D3D. |
| … | u16 × Σicount | triangle indices, **local to each component** |
| … | material × n | per component, see below |

Material record, 302 bytes per component:

| Size | Field |
|---|---|
| f32 × 11 | Hypothesis, based on value patterns and D3D usage: diffuse RGBA (4), specular power (1), specular RGB (3), emissive RGB (3). |
| char[129] | texture BMP file name, which may be empty |
| char[129] | second texture name. The loader guards this read with `local_818 == 2`. That variable's source is not yet traced, but it is likely the version digit. Every shipped file contains the field. In practice it is `eb.bmp` for the star sprite. Mask textures such as `retro64m.bmp` appear as separate components instead. |

Level `<MODEL>` tags can override per-model colours (`RED1..BLUE3`, `COLOR1..3`) and `SCALE`. The meaning of the colours, either replacing material diffuse values for components 1 to 3 or modulating them, is **unresolved**. See `FUN_00418cc0`.

Textures are uncompressed BMPs, 24-bit or 8-bit paletted (`FUN_00422430` rejects other formats). An alpha mask can be applied from a same-size BMP with `r3d_SurfaceSetAlphaFromBMP`.
