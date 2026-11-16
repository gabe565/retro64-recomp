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

## Results

Pending: the VM was offline when the script was written. Native captures: `_work/compare/native/` (not committed).
