#!/bin/bash
# Cross-build portable play builds for other machines, with the game embedded
# (--features embed-game), so nothing but the output needs copying:
#   linux    _work/dist/bugatron-linux-x64/bugatron   SDL3 linked statically; needs glibc >= 2.38
#            (SteamOS 3.5+ and current desktop distributions)
#   windows  _work/dist/bugatron-windows-x64/         bugatron.exe and SDL3.dll, plus a .zip
# Run on x86-64 Linux, e.g. the OrbStack machine:
#   orbctl run -m bugatron-linux-x64 bash -lc 'tools/build-portable.sh [linux] [windows]'
# Needs: cmake and SDL3's Linux build dependencies (wiki.libsdl.org/SDL3/README-linux);
# for Windows, what tools/windows/build-kit.sh needs (MinGW, _work/dxc, _work/win-sdl).
# The outputs contain the original game files and code translated from them: keep
# them for your own use.
set -euo pipefail
R=$(cd "$(dirname "$0")/.." && pwd)
cd "$R"
D=_work/dist
targets=("$@")
[ ${#targets[@]} -gt 0 ] || targets=(linux windows)
for t in "${targets[@]}"; do
  case $t in
  linux)
    export CARGO_TARGET_DIR=$HOME/target-portable-linux
    cargo build --release -p bugatron --features embed-game,bugatron-runtime/sdl3-from-source
    rm -rf $D/bugatron-linux-x64 && mkdir -p $D/bugatron-linux-x64
    cp $CARGO_TARGET_DIR/release/bugatron $D/bugatron-linux-x64/
    ;;
  windows)
    SDL=$(ls -d _work/win-sdl/SDL3-*/x86_64-w64-mingw32 | tail -1)
    (
      export CARGO_TARGET_DIR=$HOME/target-portable-win BUGATRON_DXC=$R/_work/dxc/bin/dxc
      export PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_PATH=$R/$SDL/lib/pkgconfig PKG_CONFIG_LIBDIR=$R/$SDL/lib/pkgconfig
      export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
      cargo build --release --target x86_64-pc-windows-gnu -p bugatron --features embed-game
      rm -rf $D/bugatron-windows-x64 && mkdir -p $D/bugatron-windows-x64
      cp $CARGO_TARGET_DIR/x86_64-pc-windows-gnu/release/bugatron.exe $SDL/bin/SDL3.dll $D/bugatron-windows-x64/
    )
    (cd $D && rm -f bugatron-windows-x64.zip && python3 -m zipfile -c bugatron-windows-x64.zip bugatron-windows-x64/)
    ;;
  *) echo "unknown target: $t (linux, windows)" >&2; exit 2 ;;
  esac
done
ls -la $D
