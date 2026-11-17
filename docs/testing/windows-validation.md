# Windows validation

## Kit 5 (oracle only): x87 FPATAN confirmation

Only the 32-bit instruction oracle is needed. It now includes `x87_fpatan_quadrants_and_table` (docs/recompilation-feasibility.md §7.11).

| File | SHA-256 |
|---|---|
| `bugatron-x86-oracle.exe` (32-bit x86) | `d6969590d1f0214f876e2789f5b9798f835929f3886e598c896ffeea6d3787c8` |

Run it on the physical x86-64 PC (not in the arm64 VM: Windows on Arm's x86 emulation does not reproduce x87 precision):

```powershell
Get-FileHash .\bugatron-x86-oracle.exe -Algorithm SHA256
.\bugatron-x86-oracle.exe .\x86-oracle-windows.json > $null
```

Send back `x86-oracle-windows.json`. It replaces `tests/fixtures/x86-oracle-amd-ryzen7-9800x3d.json`. Every strict case should match the model, the new FPATAN case included, apart from the documented 64-bit-precision case. A mismatch next to ±π would mean the hardware's quadrant terms use its 66-bit π.

**Result (2026-11-17):** all 61 cases ran. Every FPATAN value matched, but one status word did not: atan2(2, −3) had C1 = 0 where the model gave 1. That is the predicted sign of the 66-bit π in the quadrant terms. The model now uses it (§7.11), and every strict case matches. The output replaced the fixture.

Built with: `tools/windows/build-kit.sh`'s toolchain plus `gcc-mingw-w64-i686` and `rustup target add i686-pc-windows-gnu`, then `cargo build --release -p bugatron-x86-oracle --target i686-pc-windows-gnu`.

---

## Building natively on Windows on Arm (arm64)

A native arm64 build avoids x64 emulation on Windows on Arm, the reference VM included. In the VM (or on any arm64 Windows machine):

1. **Install the tools:**
   - Visual Studio 2022 Build Tools, with the *C++ ARM64 build tools* and a *Windows 11 SDK* (which provides `dxc.exe`);
   - CMake;
   - Git;
   - Rust from <https://rustup.rs> (it selects `aarch64-pc-windows-msvc` on arm64).
2. **Get the source and the game:** clone the repository and put `Bugatron_Win_EN_Gold_120.zip` in `reference\`.
3. **Put DXC on `PATH`** in a *Developer PowerShell for VS* (ARM64):

   ```powershell
   $env:PATH = (Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\arm64\dxc.exe" | Sort-Object FullName -Descending | Select-Object -First 1).DirectoryName + ";" + $env:PATH
   ```

4. **Build and run**, with SDL3 built from source as CI does on Windows:

   ```powershell
   cargo build --release -p bugatron --features bugatron-runtime/sdl3-from-source
   .\target\release\bugatron.exe
   ```

The game finds the zip through the build path, or pass `--exe`. This path is untested so far: CI builds Windows x86-64 only.

---

## Kit 4: the game on Direct3D 12 (Windows x86-64)

Kit 4 is the full game (`bugatron.exe` with SDL3.dll) plus every test binary. `tools/windows/build-kit.sh` cross-builds it with MinGW on x86-64 Linux (the OrbStack machine `bugatron-linux-x64`), with DXC compiling the Direct3D 12 shaders.

The output is `_work/bugatron-windows-kit.zip`. It contains code translated from your `Bug3D.exe`, so keep it for your own use.

### Running it (VM or physical PC)

1. Unzip the kit anywhere, for example `C:\Users\Public\bugatron-kit`.
2. In PowerShell, run:

   ```powershell
   powershell -NoProfile -ExecutionPolicy Bypass -File .\winkit\run-validation.ps1 -Game "C:\Program Files (x86)\Bugatron Gold"
   ```

   Run it as its own `powershell -File`: a nested `& .\run-validation.ps1` is blocked by the default execution policy.

The script:
- runs every test binary, with the `gpu` tests under `BUGATRON_REQUIRE_GPU=1`;
- runs three scripted game runs: the title at 32 bpp with 12 s of recorded audio, 6000+ frames of gameplay from `score-input.txt`, and the title at 16 bpp;
- writes `results\` and `results.zip`.

Scripted runs ignore the host keyboard and mouse. Windows may open and close game windows during the run; leave them alone. `SDL_LOGGING=gpu=info` makes SDL print the driver and adapter, for example `D3D12 Adapter: …`. To force a backend, set `SDL_GPU_DRIVER=direct3d12` or `vulkan`.

**On a physical PC**, also play a few minutes by hand: `bugatron.exe --exe "<game>\Bug3D.exe" --data "<game>"`. Check the menus with the mouse, keyboard steering, sound, and Alt-Tab. Then send back `results.zip`, the CPU and GPU names, and anything that looked or sounded wrong.

### Results: Windows 11 VM (2026-11-16)

The VM is UTM on Apple Silicon: Windows 11 Home 23H2 arm64. Its display adapter is VirtIO GPU DOD, a display-only driver with no 3D. The x86-64 kit runs under Windows' x64 emulation.

- **Backend:** `SDL_GPU Driver: D3D12`, `D3D12 Adapter: Microsoft Basic Render Driver` (WARP, Microsoft's software D3D12 renderer), driver 10.0.22621.4541. SDL warns that `WinPixEventRuntime.dll` is missing. That only affects GPU debug labels, which we do not use.
- **Tests:** all 94 pass. That includes all 7 `gpu` tests with `BUGATRON_REQUIRE_GPU=1` on D3D12/WARP, and the x86 semantics suite with the hardware-oracle fixtures.
- **Game:** startup, timer calibration (399999 ticks, as on macOS), the title, the menus, and scripted gameplay through flip 6000 all run without errors.
- **Frames vs Metal:**

  | Run | Frame | Pixels that differ | Max channel diff |
  |---|---|---|---|
  | Title, 32 bpp | 139 | 94 (0.03 %) | 1 |
  | Title, 32 bpp | 450 | 3 413 (1.11 %) | 2 |
  | Gameplay | 2000 / 4000 / 6000 | 769 / 804 / 812 (0.26 %) | 1 |
  | Title, 16 bpp | 139 / 450 | 28 / 713 (≤ 0.23 %) | 9 (one RGB565 step) |

  These are rounding differences between rasterisers. Because the gameplay frames still agree after 6000 frames of scripted play, the game state is identical between Windows x86-64 (generated C compiled by MinGW GCC) and macOS arm64 (clang). Against the Vulkan/lavapipe frames the differences are at most 4/255.
- **Audio:** the 12 s title recording is bit-identical to the macOS recording (1 058 400 samples, max difference 0). The owner heard the music playing in the VM. UTM's emulated sound card ignores the guest's volume and mute for every app, Windows' own sounds included, so that is not a bug in this build.
- **Speed:** very slow, about 15 flips/s for gameplay (8000 vertical blanks took 544 s). WARP, x64 emulation and the per-frame back-buffer readback (renderer.md) add up. Simulation timing is virtual, so results do not depend on speed.
- **Not covered by the VM:**
  - a hardware D3D12 GPU;
  - Vulkan on Windows;
  - interactive keyboard and mouse: the input path is the same as for scripted input, but real Windows events, relative mouse mode and Alt-Tab are unchecked. The VM is too slow to play.

  Use the physical PC steps above for these.

### Windows build fixes found on the way

- **MinGW's linker keeps unused references.** The runtime declares `recomp_lookup`, which the generated code defines, and ELF and Mach-O linkers drop the unused reference in test binaries but MinGW's does not.
  - The runtime's unit tests now define stubs.
  - The `pixel`, `tnl` and `listbox` tests now link the test-support crate, which carries generated guest code.
- **The semantics test takes `BUGATRON_ROOT`,** so the hardware-oracle fixtures can travel with the kit.

---

# Windows validation on a physical x86-64 PC (kits 1–3)

## Kit 3 (oracle only): x87 FSIN/FCOS/FPTAN confirmation

Only the oracle is needed this time.

| File | SHA-256 |
|---|---|
| `bugatron-x86-oracle.exe` (32-bit x86) | `5a7ac08afce5342e8ece278135bcf9e997dd7aa324b2f7b10a72bf7ed7c1f18b` |

```powershell
Get-FileHash .\bugatron-x86-oracle.exe -Algorithm SHA256
.\bugatron-x86-oracle.exe .\x86-oracle-windows.json > $null
```

Result (2026-11-15): all 60 cases match the model except the documented 64-bit-precision case. That came after one fix, PE and C1 for tiny arguments. Expected: `wrote .\x86-oracle-windows.json`, with 60 cases. Send back `x86-oracle-windows.json`. The model is expected to match it exactly on every strict case, including the four new trig cases (`x87_trig_workload_angles`, `x87_trig_negative_and_multiples`, `x87_trig_boundaries`, `x87_fptan_values_and_stack`). The only expected difference is the documented 64-bit-precision case.

---


Purpose:

1. Run the proof of concept natively on Windows x86-64 (open item D5).
2. Run the instruction oracle on a real x86 CPU. Its output becomes the hardware reference for the regression suite. It resolves the x87 questions the VM's emulator can't answer (D7, plus the extended-precision and 24-bit-precision behaviour).

## Kit

Built by cross-compiling from this repository with MinGW (see the command at the end). The PoC executable contains code translated from your `Bug3D.exe`, so keep it for your own use.

| File | SHA-256 |
|---|---|
| `bugatron-poc.exe` (x86-64) | `7261a5218e76dd84da5d728083fd8f4f7220f3a6fa10109c7e8252ac426dd24d` |
| `bugatron-x86-oracle.exe` (32-bit x86) | `99b54a4fdd7a4eb925938c8e6b2876f157c2bc0a4780fa0f89fbc7d7ec0f4688` |
| `poc-reference.json` | `215055957cde9f604fa62c0df7d8cd32fd7b254583905eb488522144a0740dd8` |

The binaries are unsigned, so SmartScreen may ask for confirmation ("More info" → "Run anyway"). They read the game files and write only the output files you name.

## Steps (PowerShell)

Bugatron Gold 1.20 must be installed. Copy the kit to a writable folder.

```powershell
mkdir C:\bugatron-test; cd C:\bugatron-test
# copy bugatron-poc.exe, bugatron-x86-oracle.exe, poc-reference.json here

$game = "C:\Program Files (x86)\Bugatron Gold"      # adjust to your install folder
Get-FileHash .\bugatron-poc.exe, .\bugatron-x86-oracle.exe, .\poc-reference.json -Algorithm SHA256
Get-FileHash "$game\Bug3D.exe" -Algorithm SHA256
#   expected: BE581A63BCCFC7DCC78A5C55CACC02E9F7AAB9E5398540D2CC86A7ABA758919E

.\bugatron-poc.exe --exe "$game\Bug3D.exe" --data "$game" --reference .\poc-reference.json --out .\poc-results-windows.json
.\bugatron-x86-oracle.exe .\x86-oracle-windows.json > $null
Get-CimInstance Win32_Processor | Select-Object Name
```

## Expected output

`bugatron-poc.exe` prints the following. The guest base address will differ; every other line should match exactly.

```
guest base 0x...; translated functions: 347
[PASS] M1 image mapped at original addresses (.rdata strings, .data initial values, host = base + guest)
[PASS] P1 translated MSVC CRT start-up reached WinMain
[PASS] P2 translated .lev decoder matches the original on 30/30 files
[PASS] P3 translated frame timing reproduces 600/600 recorded frames bit-exactly
[PASS] P3b frame-timing edge cases match the static analysis (4-tick floor, 512-tick clamp)
[PASS] P4 translated CRT srand/rand matches the original (10,000 values x 4 seeds)
[PASS] I1 real indirect calls executed: guest->guest function pointers, COM vtable, IAT imports, host->guest calls
results written to .\poc-results-windows.json
guest exit code 0x0; overall: PASS
```

The exit code is 0. A `[FAIL]` line or a `bugatron-runtime fatal:` message is a real result: send the output as-is. If the program prints nothing after the first line, run `echo $LASTEXITCODE` and send that number too.

History: kit 1 (2026-11-15) exited silently after the first line on Windows. Reserved-but-uncommitted guest memory was being read. Fixed in kit 2 (these hashes). Kit 2 passed all proof-of-concept checks on the same PC. The oracle from kit 1 is already committed as `tests/fixtures/x86-oracle-amd-ryzen7-9800x3d.json`.

`bugatron-x86-oracle.exe` prints `wrote .\x86-oracle-windows.json` to stderr. The file holds 56 cases, including the new `x87_transcendental_sweep`. These values are already known for real x87 hardware, so they make a quick sanity check (open the JSON and find the case):

| Case | Field | Expected on real hardware | Meaning |
|---|---|---|---|
| `x87_pc64_extended_sum` | `mem` bytes 16–23 (hex chars 32–47) | `0100000000002000` (2^53 + 1) | 64-bit precision honoured. The model gives `0000000000002000`, a documented discrepancy. |
| `x87_pc24_division` | `mem` bytes 16–23 | `00000060 5555d53f` | 1/3 rounded to single precision |
| `x87_div_third_pc53` | `mem` bytes 8–15 | `555555555555d53f` | 1/3 in double precision |
| `pushad_popad` | `mem` bytes 0–3 | `20000000` | PUSHAD pushes 32 bytes |

Everything else is compared automatically. No mental checking is needed.

## Send back

- `poc-results-windows.json`
- `x86-oracle-windows.json`
- The console output of both commands, and the CPU name.

These get committed as `tests/fixtures/x86-oracle-<cpu>.json`, after which `cargo test -p bugatron-x86-tests` checks the model against real hardware on every run. Results get recorded in `docs/recompilation-feasibility.md`.

## Rebuilding the kit

```sh
docker run --rm --platform linux/arm64 -v "$PWD":/src -w /src -e CARGO_TARGET_DIR=/tmp/target rust:1-bookworm bash -c '
  apt-get -qq update && apt-get -qq install -y gcc-mingw-w64-i686 gcc-mingw-w64-x86-64 &&
  rustup target add i686-pc-windows-gnu x86_64-pc-windows-gnu &&
  cargo build --release -p bugatron-x86-oracle --target i686-pc-windows-gnu &&
  cargo build --release -p bugatron-poc --target x86_64-pc-windows-gnu'
```

`bugatron-poc` needs the game: `reference/Bugatron_Win_EN_Gold_120.zip`, or `BUGATRON_EXE`. Its recompiler hints are committed in `hints/`.
