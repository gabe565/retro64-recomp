# Windows validation on a physical x86-64 PC

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

`bugatron-poc` needs `_work/ext/gold120/app/Bug3D.exe` and `_work/ghidra/out/recomp_hints.json` (`tools/extract_reference.sh --ghidra`).
