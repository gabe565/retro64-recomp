# Function map — Bugatron Gold v1.20a `Bug3D.exe`

Applies only to sha256 `be581a63…758919e`. Ghidra 12.2 auto-analysis found 678 functions, 606 of them unnamed. The confidence column uses these values:

- **V** (verified): behavior read from decompiled code, cross-checked against assembly or data.
- **S** (string-anchored): the role is inferred from the strings or APIs the function references; its body is not yet read.
- **H** (hypothesis).

Rust names are proposed and not yet implemented.

## Application shell

| Address | Role | Conf. | Proposed Rust |
|---|---|---|---|
| 0x42ba98 | CRT entry (`entry`) → WinMain | V | — |
| 0x4076e0 | WinMain: enumerate devices, read and write `prefs`, video dialog, create window, `r3d_Initialize`, message loop | V | `app::run` |
| 0x408280 | Video-mode dialog procedure (dialog 102) | S | (replaced by config/CLI) |
| 0x407d10 | Window procedure: key-state array, pause/resume, window resize, render-quality prompt | S | `platform::events` |
| 0x4087d0 | InitApp: camera, light, `srand(timeGetTime())`, default high scores, load `config.xml` | V | `game::init` |
| 0x408940 | Per-frame entry, calls `0x40ba70` unless `0x8429d4` is set | V | `game::frame` |
| 0x40ba70 | Frame body: lost-surface recovery, timing, game-state switch on `0x4370a0` | V(partial) | `game::update_and_render` |
| 0x40b720 | Frame delta in 1/1024-s ticks (minimum 4, reuse previous if over 512) | V | `timing::frame_delta` |
| 0x41c180 | Vsync-style limiter calibrated from `WaitForVerticalBlank` | V | `timing::vsync_limiter` |
| 0x40b7c0 | vsync on/off toggle and HUD | S | |
| 0x411f70 | Input poll: key bindings at `0x437f9c`, joystick, mouse | V(partial) | `input::poll` |
| 0x41cb50 / 0x41cb70 | key held / key pressed-edge test on `0x841a38[vk]` | V | `input::held` / `input::pressed` |
| 0x412930 | Debug and HUD overlay ("MS LOGIC", "MS RENDER", "POLYGONS"), screenshot | S | |

## Data loading

| Address | Role | Conf. |
|---|---|---|
| 0x417fe0 | Scan `pack*.xml`: decode, read manifest fields, build pack list and menu entries | V |
| 0x417f90 | Copy field until `\r`, return length + 2 | V |
| 0x4076a0 | JC decompress wrapper | V |
| 0x41ce10 / 0x41e916 / 0x41cfae | JC: size, allocation size, decompressor | V |
| 0x419bb0 | Config/level interpreter: WAVE, MUSIC, MODELS, LEVEL, OBJECT, PERSPECTIVE, `NEXT_CONFIG`, `PROCESS_*` | S |
| 0x419890 | `<MODELS>` parser (NAME, FILE_NAME, TIMES) | S |
| 0x418cc0 | Model colour overrides (COLOR1..3, RED/GREEN/BLUE1..3, SCALE) | S |
| 0x408a40 | `<FONTS>` parser | S |
| 0x403730 | `<ALIEN>` parser: STAGE and POINT lists, OFFSET_*, COPY_ALIEN, DORMANT_* | S |
| 0x40ca60 | Alien properties, explosion colours, sounds | S |
| 0x4147c0 / 0x414c90 | `<POWER_UPS>` parser and power-up type table | S |
| 0x40aa80 | `<TEXT>` parser (IN_/OUT_/IDLE_ effects and timing) | S |
| 0x41c800 | Default high-score table (TWISTKILL…JONNYBLAZE) | S |
| 0x4179b0 | Particle and star sprite loading (`Graphics/e?.bmp`) | S |

## Gameplay (string-anchored, to be decompiled in Milestone 3+)

| Address | Role | Conf. |
|---|---|---|
| 0x40df90 | Alien stage state machine (ALIEN_ATTACKING, DIVING, SETTLING, EXPLODING, …) | S |
| 0x409fb0 | Rotation types (ROTATE_X/Y/Z) | S |
| 0x40a0b0 | Movement easing (SQUARED, SQUARE_ROOT) | S |
| 0x40a240 / 0x40a840 / 0x40ff90 | Text and HUD layout effects (SCRAMBLE, STAGGERED, corner anchors) | S |
| 0x408e70 | Colour schemes (RAINBOW, EARTH_TONES, RANDOM_GRADIENT) | S |
| 0x406b40 | Star field (`star_model`) | S |
| 0x406c90 | Bullets | S |
| 0x402a30 / 0x415380 / 0x407240 / 0x4138a0 | Boing-ball power-up and its objects | S |
| 0x4156e0 | Power-up handler. Case 0xF (0x41599c) creates the three atoms (atomr/g/b) in the ship's list at +0x530. Each gets: +0x64 = (rand()%100 − 50)·0.02; +0x68 = 1 − √(+0x64²); +0x6c = (rand()%100 − 50)·0.02; +0x70 = index | V |
| 0x415bb0 | Atom orbit, run on the last simulation step of a frame. Angle +0x4c += step·1.5° (wrapped once at 360°). Position = ship + (sin + cos table at 0x82e988 / 0x82e38c, indexed by `_ftol`(angle)) × the per-atom factors. It uses the step *index* (frame ticks − 1), so atoms move about 24° per frame, about 1,440°/s at 60 fps and about the same at any rate. This looks like the original's own quirk: the owner noticed very fast atoms (2026-11-16) | V |
| 0x41295a–0x4129b1 | Simulation loop: one step per 1/1024 s tick of the frame delta (0x8429e0); the step index is at 0x832030 | V |
| 0x4103a0 | Menu: difficulty (EASY/NORMAL/HARD), link handling, website | S |
| 0x411020 | High-score screen | S |
| 0x418730 | High-score name entry | S |
| 0x4163e0 | High-score bookkeeping | S |
| 0x4188e0 | Loading screen | S |

## r3d engine (Direct3D 6 wrapper)

| Address | Name (from error strings) | Conf. |
|---|---|---|
| 0x424b90 | `r3d_Initialize(hwnd, 184800.0, 75.0, drv, dev, mode)` | S |
| 0x424a10 | Device and mode query (`r3d_Enumerate` family) | S |
| 0x424440 | Timer: ms-like ticks (×1024 / QPC frequency) | V |
| 0x424520 | `_r3d_InitTimer` | V |
| 0x41e950 | `r3d_ListAdd3DModel` | V |
| 0x41f4c0 | `r3d_Load3DModel` | V |
| 0x41ebe0 / 0x41ed70 / 0x41f2b0 | material list, `r3d_ListLoad`, unload | S |
| 0x41ff20 | `r3d_Render3DModel` | S |
| 0x420ed0 | `r3d_SetMatrix` | S |
| 0x420f90 / 0x4210d0 | `r3d_RenderBegin` / `r3d_RenderEnd` (alpha polygon flush) | S |
| 0x421a30 / 0x421c20 | `r3d_SetLightLocationColor` / `r3d_LightSetAmbient` | S |
| 0x4221e0 / 0x4229a0 / 0x422750 | surface create / from BMP / alpha from BMP | S |
| 0x422430 | BMP loader (8-bit and 24-bit, uncompressed only) | S |
| 0x422be0 / 0x422cb0 | `r3d_2DBegin` / `r3d_2DEnd` | S |
| 0x4231b0 | `r3d_SurfaceToBMP` | S |
| 0x423610 | `r3d_EnableSpecular` | S |
| 0x423650 | `r3d_SetFilteringAndMipMapOptions` | S |
| 0x423bd0 | DirectDraw/D3D error-code → string table | S |
| 0x424400 / 0x424430 / 0x4220e0 | error reporting helpers | S |

## CRT (Ghidra library matches)

`_rand` 0x42ab73, `srand` 0x42ab66 (V), `_malloc`, `__ftol` 0x42ac24 (V; x87 `fistp` with round-toward-zero control word), `_strlen`, `_strcmp`, `_strncpy`, `_memset`, `__allmul`, `__aulldiv`. File I/O wrappers `0x42b164` (fopen), `0x42b01c` (fread), `0x42ae66` (fwrite), `0x42af9f` (fclose), `0x42b79d` (fseek), and `0x42b61a` (ftell) are identified from call patterns (H).

## Globals (selection)

| Address | Meaning | Conf. |
|---|---|---|
| 0x4370a0 | Game state: 2 = load next config, 3 = transition, 5 = config running; 6 (initial) and 8 unnamed | V + runtime |
| 0x81d548 | Next config file name (set from `NEXT_CONFIG`, the pack manifest, or game over) | V + runtime |
| 0x4370a4 | ".lev files are encoded" flag (initial 1) | V |
| 0x4370a8 / 0x4370ac | Probably music and sfx volume (90 / 25) | H |
| 0x8429dc / 0x8429e4 | Current and previous timer tick | V |
| 0x8429e0 | Frame delta in ticks | V |
| 0x81d67c | Frame delta in seconds-ish (delta / 1024, f32) | V |
| 0x841a38 | Virtual-key state array | V |
| 0x437f9c..0x437fd8 | Key binding table (16 VK codes) | V |
| 0x98d4a8 / ac / b0 | Selected driver / device / mode indices (`prefs`) | V |
