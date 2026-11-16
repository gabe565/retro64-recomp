#!/usr/bin/env bash
# Rebuild the local analysis workspace (_work/, gitignored) from reference/.
#
# Only reference/Bugatron_Win_EN_Gold_120.zip is required; the other releases
# (Bugatron_Win_EN_Gold.zip, Bugatron104Setup.exe, Bugatron_Win_EN_v151d.exe) are
# extracted if present.
# Never modifies anything under reference/. Requires: unzip, innoextract (>=1.9),
# python3. Optional: Ghidra 12.x (set GHIDRA_HOME or install via Homebrew) for
# the headless decompile export.
#
#   tools/extract_reference.sh            # extract installers + decode .lev files
#   tools/extract_reference.sh --ghidra   # also run the Ghidra export for Gold 1.20
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REF="$ROOT/reference"
W="$ROOT/_work"   # NOTE: Ghidra rejects project paths with a component starting with '.'

mkdir -p "$W/zips" "$W/carve" "$W/ext/gold120" "$W/ext/gold115" "$W/ext/v151d"

echo "== Gold 1.20 (Inno Setup inside zip): required"
unzip -o -q "$REF/Bugatron_Win_EN_Gold_120.zip" -d "$W/zips"
innoextract -s -d "$W/ext/gold120" "$W/zips/buggold_120.exe"

# The other releases are reference material only; skipped when absent.
versions=(gold120)
if [[ -f "$REF/Bugatron_Win_EN_Gold.zip" ]]; then
  echo "== Gold 1.15 (Inno Setup inside zip)"
  unzip -o -q "$REF/Bugatron_Win_EN_Gold.zip" -d "$W/zips"
  innoextract -s -d "$W/ext/gold115" "$W/zips/Setup.exe"
  versions+=(gold115)
fi

if [[ -f "$REF/Bugatron104Setup.exe" ]]; then
  echo "== Standard 1.04 (Inno Setup 1.3.25)"
  innoextract -s -d "$W/ext/v104" "$REF/Bugatron104Setup.exe"
  versions+=(v104)
fi

if [[ -f "$REF/Bugatron_Win_EN_v151d.exe" ]]; then
echo "== Standard 1.51d (Inno Setup 4.2.6 appended to a wrapper stub at 0x2d004)"
python3 - "$REF/Bugatron_Win_EN_v151d.exe" "$W/carve/v151d_inner_setup.exe" <<'EOF'
import re, struct, sys
data = open(sys.argv[1], 'rb').read()
for m in re.finditer(b'MZ', data):
    o = m.start()
    if o == 0 or o + 0x40 > len(data):
        continue
    pe = struct.unpack_from('<I', data, o + 0x3c)[0]
    if 0x40 <= pe < 0x400 and data[o + pe:o + pe + 4] == b'PE\0\0':
        open(sys.argv[2], 'wb').write(data[o:])
        print(f'inner installer at {o:#x}')
        break
else:
    sys.exit('inner installer not found')
EOF
innoextract -s -d "$W/ext/v151d" "$W/carve/v151d_inner_setup.exe" || true  # warns about the carved name
versions+=(v151d)
fi

echo "== Standard 1.20: Ghost Installer (UPX-packed); no extractor available, skipped"

echo "== Decoding .lev files"
for v in "${versions[@]}"; do
  python3 "$ROOT/tools/lev_decode.py" -o "$W/decoded/$v" "$W/ext/$v/app/"*.lev >/dev/null
  echo "   $v: $(ls "$W/decoded/$v" | wc -l | tr -d ' ') files"
done

if [[ "${1:-}" == "--ghidra" ]]; then
  GH="${GHIDRA_HOME:-$(brew --prefix ghidra 2>/dev/null)/libexec}"
  rm -rf "$W/ghidra" && mkdir -p "$W/ghidra/proj" "$W/ghidra/out"
  "$GH/support/analyzeHeadless" "$W/ghidra/proj" gold120 \
    -import "$W/ext/gold120/app/Bug3D.exe" -overwrite \
    -scriptPath "$ROOT/tools/ghidra" -postScript ExportDecomp.java "$W/ghidra/out" \
    > "$W/ghidra/headless.log" 2>&1
  "$GH/support/analyzeHeadless" "$W/ghidra/proj" gold120 -process Bug3D.exe -noanalysis -readOnly \
    -scriptPath "$ROOT/tools/ghidra" -postScript ExportRecompHints.java "$W/ghidra/out/recomp_hints.json" \
    > "$W/ghidra/hints.log" 2>&1
  echo "   Ghidra export and recompiler hints: $W/ghidra/out"
fi

echo "== Executable hashes"
find "$W/ext" -maxdepth 3 -iname '*.exe' -exec shasum -a 256 {} \;
