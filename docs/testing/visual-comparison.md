# Side-by-side visual comparison with the original

The goal is to compare the native build and the original game at the same resolution, display depth, and **game state**, so that any remaining difference comes from rendering.

## Making the game state identical

The original's simulation depends on its time sources, its input, and its random seed (`srand(timeGetTime())` at start-up). `tools/frida/capture_deterministic_frames.js` spawns the original under Frida in the VM and gives it the native runtime's inputs. Only calls from `Bug3D.exe` are affected; FMOD and the DirectDraw driver keep real time.

| Input | Native runtime | Original under the script |
|---|---|---|
| QPC | 24 MHz; each read returns now, then one tick passes | same |
| `timeGetTime` | virtual ms (starts at 0) | same |
| `Sleep(n)` | +n ms; `Sleep(0)` takes no time | same (and really sleeps n ms) |
| Vblank waits, `Flip(WAIT)` | next 1/60 s boundary | same, after the real wait |
| `GetMonitorFrequency` | 60 | 60 |
| Keyboard / mouse / joystick | `--input` script; cursor otherwise only moved by `SetCursorPos`; joystick unplugged | `INPUT_SPEC`, same syntax and the same messages and lParams |
| Video dialog | headless, OK | `DialogBoxParamA` returns IDOK (selection from `prefs`) |

Frame N is then the same scene in both. Native determinism is verified: two runs give byte-identical captures.

## Procedure

1. In the VM, check that `prefs` selects 640×480×32 on device 1 (the reference: driver 0, device 1, mode 38 with dgVoodoo2). Then start `frida-server`.
2. Set `CAPTURE_FLIPS`, `INPUT_SPEC` and `TRACE_FLIPS` in a copy of the script. Spawn the original under it (REA `start_frida_session` with `mode: spawn` and the `Bug3D.exe` path, load the script, resume), and let it run past the last capture. The script writes `C:\Users\Public\bugatron-frames\flip-N.raw` and sends a `{type: 'frame', flip, w, h, pitch, bpp, file}` message for each.
3. Copy the raw files out of the VM and convert them: `tools/compare_frames.py --from-raw flip-450.raw 640 480 2560 32 original-450.png`.
4. Capture the native side: `bugatron --video-mode 640x480x32 --frames 800 --dump-frames 139,450,700 --dump-dir _work/compare/native`.
5. Compare: `tools/compare_frames.py _work/compare/native/frame-00450.png original-450.png --diff diff-450.png`.

Repeat at other resolutions (`--video-mode`, and the dialog choice in the VM). A 16-bit comparison (approximation A6) is optional, since 32 bpp is the default.

## Results (2026-11-15, 640×480×32, VM with dgVoodoo2)

**The harness works.** Spawned with the script, the original:
- calibrated its timer to 399999 ticks, the same as native;
- reached flips 139, 450, and 700 at the same virtual times as the native build (3300 ms, 8483 ms, …);
- had the same game tick (28621) at flip 1620.

| Flip | Pixels differing | Mean channel difference |
|---|---|---|
| 139 | 0.67% | 0.49 |
| 450 | 12.85% | 2.87 |
| 700 | (similar to 450) | |

**Same scene.** The settled title screen matches in layout and content. The boing ball has **the same oval shape in the original**, confirming the analysis in `../graphics/renderer.md` (perspective of an off-axis sphere).

**Rendering on identical geometry.** 281 of 368 world matrices in a frame are bit-identical between the two. Where geometry matches:
- 2D text: 97–99.8% of pixels identical; the rest differ by exactly 1 level.
- 3D power-up icons: 92–98% identical, about 3.5% within ±10 levels (lighting and filtering rounding, A1/A2/A5), and 1–2% large differences, mostly on polygon edges (rasterisation, A7).

**Small divergence in animation.** The 1/1024 s game timer (0x424440: `ftol((QPC − base) × 1024 / freq)`) lands exactly on a tick boundary every 15 frames at 24 MHz. In one such frame the original computed a delta of 17 where native computed 18 (ticks 28672 vs 28673 after flip 1623). Animated objects (ball rotation, about 0.25° by flip 1623; marquee scroll) then drift slightly apart, while static ones stay bit-identical.
- The suspected cause, different QPC read counts, turned out not to be it. See the 2026-11-16 results below.

The VM runs the original under Windows-on-Arm x86 emulation (Prism). Its x87 arithmetic has not been validated against hardware; the native model matches an AMD Ryzen bit for bit. Any remaining numeric divergence should be checked against that before it is attributed to the runtime.

## Results (2026-11-16): timer phase aligned

**The cause** was the harness, not the runtime.
- The script hooked `Flip` only once `r3d_Initialize` returned, so the original's virtual clock missed the **two flips** the game makes during initialisation. Their refreshes are 1/30 s, or 34.13 ticks at 1/1024 s.
- Counts still lined up: QPC reads, deltas and `tick_now` matched ours exactly, but two flips later.
- The 0.13-tick phase shift changed which frames landed on a tick boundary: hence the 17-versus-18 delta.

**The fix:** the game has a single `Flip` call site, 0x426493 in its page-flip wrapper 0x426420. That instruction is 3 bytes, too short for Frida to hook, so the script now:
- captures frames at the wrapper's entry;
- counts the flip and advances the clock at 0x426496, the instruction after `Flip` returns.

Both work from process start. Flip numbers now mean the same frame on both sides; before, the original's "139" was our 141.

The script also reports `{type:'tick', flip, qpc_reads, delta, tick_now}` at chosen flips, matching `BUGATRON_TRACE=delta` natively. It writes frames to raw files in the VM instead of base64-encoding them in-process, which stalled the game. `compare_frames.py --from-raw` converts them.

**Result,** title sequence, no input:
- Flip number, QPC reads, delta and `tick_now` agree exactly at every reported flip from 1 to 200.
- Frames:

  | Flip | Pixels that differ | Max channel diff |
  |---|---|---|
  | 139 | 206 (0.07 %) | 1 |
  | 450 | 6,886 (2.24 %) | 2 |
  | 700 | 8,861 (2.88 %) | 2 |

  Animated frames no longer drift. What remains is rasterisation and rounding between dgVoodoo's Direct3D and our renderer (renderer.md A1–A7).

## Results (2026-11-16): scripted gameplay

**Input:** a click at flip 600 starts the game from the title screen. A second click at 800 picks the first level. Ctrl (fire) is held from 1000. Left and right then alternate, each held 45 flips, every 100 flips. This is `--input "600:click@70x120,800:click@70x120,1000:CTRL+,1000:LEFT+,1045:LEFT-,1100:RIGHT+,1145:RIGHT-,…"` natively and `INPUT_SPEC` in the script. The script must have `CAPTURE_FLIPS` 2000, 4000, 6000.

**Timer:** QPC reads, delta and `tick_now` agree exactly at every reported flip, from 1 to 6000. At 6000 they are 501,860,307 QPC reads and tick 103,373 on both sides. The frame limiter's polling, and so the read count, depends on the game state, so this also shows the two runs stayed in the same state.

**Frames** (native `--video-mode 640x480x32 --frames 6100 --dump-frames 2000,4000,6000`):

| Flip | Pixels that differ | Max channel diff |
|---|---|---|
| 2000 | 1,197 (0.39 %) | 1 |
| 4000 | 1,104 (0.36 %) | 1 |
| 6000 | 1,159 (0.38 %) | 1 |

The ship, enemies, shots and score are in the same places in both, after 100 seconds of play with the same steering and firing. Every remaining difference is a single level, from rounding.

**F1 does not start the original under the script.** See R10 in `../open-questions.md`; the click is used instead.
