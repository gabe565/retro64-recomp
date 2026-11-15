# Documentation index

- [recompilation-feasibility.md](recompilation-feasibility.md) is the **current plan**: static-recompilation feasibility, challenges, and the proof-of-concept definition.
- [initial-investigation.md](initial-investigation.md) is the original findings report. Its architecture and milestone sections are superseded.
- [open-questions.md](open-questions.md) tracks unresolved questions, assumptions, and known deviations.
- `versions/` has one analysis record per release. Addresses never cross between files.
  - [gold-1.20.md](versions/gold-1.20.md) is the primary target.
  - [gold-1.15.md](versions/gold-1.15.md)
  - [standard-1.51d.md](versions/standard-1.51d.md)
  - [standard-1.20.md](versions/standard-1.20.md) (not extracted)
- `formats/` documents the data formats.
  - [lev.md](formats/lev.md) covers encoded config and level scripts and their XML-like grammar.
  - [r3d.md](formats/r3d.md) covers the model format.
  - [misc.md](formats/misc.md) covers high scores, prefs, audio, fonts, and sprites.
- [function-map/gold-1.20.md](function-map/gold-1.20.md) maps original addresses to roles and planned Rust names.

## Tools (`tools/`)

| Tool | Purpose |
|---|---|
| `extract_reference.sh [--ghidra]` | Rebuild `_work/` (gitignored) from `reference/`: extract installers, decode `.lev` files, and optionally run the Ghidra export |
| `lev_decode.py` | Decode `.lev` files (NOT + JC decompression) |
| `r3d_dump.py` | Parse and summarize `.r3d` models |
| `ghidra/RecompSurvey.java` | Headless Ghidra post-script that counts instruction classes, indirect branches, x87 use, and code outside functions |
| `ghidra/ExportRecompHints.java` | Headless Ghidra post-script that exports function entries and jump tables as recompiler hints |
| `frida/*.js` | Runtime probes for the original game in the VM. `capture_poc_reference.js` produces `tests/fixtures/poc-reference.json`. |
| `ghidra/ExportDecomp.java` | Headless Ghidra post-script that dumps every function's decompilation plus an index |

Original game files live in `reference/` (gitignored) and are never modified. Extracted and decoded copies live in `_work/` (gitignored). No copyrighted game data is committed.

## Code (`crates/`)

| Crate | Purpose |
|---|---|
| `pe` | Minimal PE32 reader |
| `recomp` | Static recompiler (x86 → C) and the `bugatron-recomp` CLI |
| `runtime` | Rust runtime plus the C machine model (`c/recomp.h`) |
| `poc` | Proof of concept: `cargo run -p bugatron-poc` (needs your `Bug3D.exe` and hints) |
