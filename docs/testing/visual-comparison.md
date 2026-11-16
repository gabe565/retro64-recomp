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
| Keyboard / mouse / joystick | none; cursor only moved by `SetCursorPos`; joystick unplugged | same |
| Video dialog | headless, OK | `DialogBoxParamA` returns IDOK (selection from `prefs`) |

Frame N is then the same scene in both. Native determinism is verified: two runs give byte-identical captures.

## Procedure

1. In the VM, check that `prefs` selects 640×480×32 on device 1 (the reference: driver 0, device 1, mode 38 with dgVoodoo2). Then start `frida-server`.
2. Spawn the original under the script (REA `start_frida_session` with `mode: spawn` and the `Bug3D.exe` path, load the script, resume). Let it run past flip 700. The script sends `{type: 'frame', flip, …}` messages with raw back-buffer pixels for flips 139, 450, and 700.
3. Save the messages as JSON and convert them: `tools/compare_frames.py --from-frida messages.json _work/compare/original`.
4. Capture the native side: `bugatron --video-mode "640 x 480 x 32" --frames 800 --dump-frames 139,450,700 --dump-dir _work/compare/native`.
5. Compare: `tools/compare_frames.py _work/compare/native/frame-00450.png _work/compare/original/frame-00450.png --diff diff-450.png`.

Repeat at other resolutions (`--video-mode`, and the dialog choice in the VM) and at 16-bit depth for approximation A6.

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
- Most likely cause: the two runs perform a different number of QPC reads between the timer's base sample and the frame starts. The script skips the video dialog (`DialogBoxParamA` returns at once), but the native build runs the dialog proc.
- This is a harness alignment issue, not a runtime deviation. Next step: count QPC calls on both sides and align them, so that animated frames match too and the remaining differences can be attributed exactly.

The VM runs the original under Windows-on-Arm x86 emulation (Prism). Its x87 arithmetic has not been validated against hardware; the native model matches an AMD Ryzen bit for bit. Any remaining numeric divergence should be checked against that before it is attributed to the runtime.
