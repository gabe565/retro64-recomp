# Recompiler hints

`gold-1.20a.json` lists, for Bugatron Gold v1.20a's `Bug3D.exe` only (SHA-256 in the file), the
function entry addresses and switch jump tables the recompiler starts from. The build checks the
hash and refuses any other executable.

It holds addresses only: no code or data from the game. It was produced by Ghidra 12.2's analysis
(`tools/extract_reference.sh --ghidra`, `tools/ghidra/ExportRecompHints.java`) and is committed so
that building needs no Ghidra. The names are Ghidra's own labels and appear only as comments in the
generated code. Regenerate it with the script and `BUGATRON_HINTS=_work/ghidra/out/recomp_hints.json`,
or copy the new export over this file.
