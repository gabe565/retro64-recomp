# Bugatron Gold 1.15 — analysis record (secondary reference)

Addresses here refer to this executable only.

## Identity

| Item | Value |
|---|---|
| Distribution | `reference/Bugatron_Win_EN_Gold.zip` (sha256 `f0247bb5…887a1e`) → `Setup.exe` (sha256 `02c47ff7…089b`, dated 2001-09-20), Inno Setup, setup data 2.0.11, title "Bugatron Gold 1.15" |
| Game executable | `app/Bugatron.exe`, 487,424 bytes, sha256 `31a2bbc48826dc756c9d05cbcda95d10489895cbdaf1557fc416b9c0595a4b40` |
| Format | PE32 i386 GUI, MSVC 6 linker, linked 2001-09-19 23:39:06 UTC, entry `0x456e2a` |
| Packing | None observed |

## Differences from Gold 1.20 (verified unless marked)

- **One executable.** There is no launcher or `Bug3D.exe` split. The DirectX 7 version check is inside the game ("Bugatron requires at least DirectX 7.0…").
- **Different audio stack.** There is no `fmod.dll`. The executable imports DSOUND (by ordinal), MSACM32, and waveOut, and contains `gaudio::GAudio` C++ symbols. Hypothesis: it statically links a third-party audio library (Galaxy Audio, or similar) with its own XM player. Music is still `.xm`, per the README entry "1.15: …now all music files are '.xm' format".
- **Imports `DirectDrawCreateEx`**, unlike 1.20.
- **The `r3d` engine has verbose internal error strings.** Examples are `_r3d_DirectDraw4->SetCooperativeLevel(...)` and `_r3d_Device->SetLightState(D3DLIGHTSTATE_AMBIENT, …)`. These confirm that the engine uses IDirectDraw4, IDirect3D3, viewports, and lights, which matches the 1.20 engine. **This executable can serve as a "symbol oracle" for naming 1.20's `r3d_*` internals.**
- **Assets** are a strict subset of 1.20's. 1.20 adds `Graphics/*.bmp` particle sprites, `m/eb|ed|em.*`, `m/pack1-6.*` pack-selection icons, and `m/pointer.r3d`.
- **Level data** decodes with the same NOT plus JC scheme. All 30 `.lev` files decode. The 1.15→1.20 diffs are small, mostly added particle and star `<MODEL>` entries, which matches the 3/20/2003 changelog entry, "Improved all particle effects… Added icons for level pack selection". That entry appears only in the 1.51d README. Gold 1.20's README stops at 1.18, but its `Bug3D.exe` was linked on 2003-03-19.

## Use in this project

- Cross-check gameplay constants and AI code paths. The gameplay code should be largely shared, but this is unverified.
- Recover names for `r3d` engine internals from the verbose error strings.
- Do not use it as the primary target: it predates 1.17/1.18 gameplay fixes and the 2003 effects update.
