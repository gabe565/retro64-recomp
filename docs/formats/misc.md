# Other data files (Gold 1.20)

## `packN.hs`: high-score table, binary

- **Observed from data** in all 9 shipped files: 280 bytes = 10 records × 28 bytes.
- Record layout: `char name[20]` (NUL-padded, uppercase), `u32 score`, `u32 level`.
- Default contents (TWISTKILL 8000/15 … JONNYBLAZE 3500/4) match the name strings referenced by `FUN_0041c800`, which is likely the default-table initializer.
- The writer function is not yet traced. Unverified details: whether the game writes the file back on exit, and the sort order on ties.
- Shipped `pack7.hs`–`pack9.hs` exist without matching `pack7-9.xml` files.

## `prefs`: video selection, binary

**Verified** in `FUN_004076e0`. The file holds 3 × i32: DirectDraw driver index, D3D device index, and display mode index. It is read before the setup dialog and written after it. If all three are zero, the game auto-selects the first 640×480 mode. It is created in the working directory.

## Audio

- `.wav`: plain RIFF PCM, for example 8-bit mono 22,050 Hz. Loaded with `FSOUND_Sample_Load`.
- `.xm`: FastTracker 2 Extended Modules, played by FMOD 3.61 `FMUSIC`. A port needs an XM replayer. **Fidelity risk**: FMOD 3's mixer, interpolation, volume ramping, and effect quirks differ from other replayers.
- Volumes: PgUp/PgDn change music volume and Home/End change effects volume. The defaults are probably the globals at `0x4370a8` (90) and `0x4370ac` (25). These two values are a hypothesis.

## Fonts and sprites

- `Fonts/<face><char>.bmp`: one 16×16 24-bit BMP per glyph. Faces are `wh`, `nj`, `score`, `popup`, and `lives`. Punctuation is encoded in the file name, as in `nj!.bmp` and `wh@.BMP`.
- `Graphics/e?.bmp`: 32×32 8-bit sprites used for particles and stars, loaded by `FUN_004179b0` (`eb%d`, `ed2%d`, and so on).
