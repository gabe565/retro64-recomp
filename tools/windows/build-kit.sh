#!/bin/bash
# Cross-build the Windows x86-64 validation kit (docs/testing/windows-validation.md).
# Run on x86-64 Linux (DXC's Linux release is x86-64 only), e.g. the OrbStack machine:
#   orbctl run -m bugatron-linux-x64 bash -lc tools/windows/build-kit.sh
# Needs: gcc-mingw-w64-x86-64, rustup target x86_64-pc-windows-gnu, and in _work/:
#   dxc/      DXC release (linux_dxc_*.tar.gz, with lib/libdxil.so for signing)
#   win-sdl/  SDL3-devel-<ver>-mingw.tar.gz unpacked
# Output: _work/winkit/ and _work/bugatron-windows-kit.zip. The kit contains code
# translated from your Bug3D.exe: keep it for your own use.
set -euo pipefail
R=$(cd "$(dirname "$0")/../.." && pwd)
cd "$R"
SDL=$(ls -d _work/win-sdl/SDL3-*/x86_64-w64-mingw32 | tail -1)
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$HOME/target-win}
export BUGATRON_DXC=$R/_work/dxc/bin/dxc
export PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_PATH=$R/$SDL/lib/pkgconfig PKG_CONFIG_LIBDIR=$R/$SDL/lib/pkgconfig
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
T=x86_64-pc-windows-gnu
cargo build --release --target $T -p bugatron
K=_work/winkit
rm -rf $K && mkdir -p $K/tests/tests/fixtures
cp $CARGO_TARGET_DIR/$T/release/bugatron.exe $SDL/bin/SDL3.dll $K/
cp tools/windows/run-validation.ps1 $K/
cp _work/frames/score-input.txt $K/
cp tests/fixtures/x86-oracle-*.json $K/tests/tests/fixtures/
# Test binaries, named after their test target.
cargo test --release --target $T --workspace --no-run --message-format=json 2>/dev/null |
  python3 -c '
import json, sys, shutil
for line in sys.stdin:
    m = json.loads(line)
    if m.get("reason") == "compiler-artifact" and m.get("executable") and m["profile"]["test"]:
        t = m["target"]; name = t["name"] if "test" in t["kind"] else t["name"] + "-unit"
        shutil.copy(m["executable"], sys.argv[1] + "/tests/" + name + ".exe")
' $K
cp $SDL/bin/SDL3.dll $K/tests/
(cd _work && rm -f bugatron-windows-kit.zip && python3 -m zipfile -c bugatron-windows-kit.zip winkit/)
ls $K $K/tests
