#!/bin/bash
# One Docker build stage (Dockerfile): build `linux` or `windows` from the game
# mounted at /game into /out. Cargo's target directory is a cache mount under /build.
set -euo pipefail
names=(Bug3D.exe buggold_120.exe Bugatron_Win_EN_Gold_120.zip)
exe=
for n in "${names[@]}"; do
  exe=$(find /game -maxdepth 1 -iname "$n" | head -1)
  [ -n "$exe" ] && break
done
if [ -z "$exe" ]; then
  echo "No game found: pass --build-context game=DIR, where DIR holds Bugatron_Win_EN_Gold_120.zip," >&2
  echo "buggold_120.exe, or the installed game (Bug3D.exe and its folders)." >&2
  exit 1
fi
export BUGATRON_EXE=$exe
mkdir -p /out
case $1 in
linux)
  # xx-cargo cross-compiles to linux/amd64 (TARGETPLATFORM) with clang, or builds natively.
  t=$(xx-cargo --print-target-triple)
  export CARGO_TARGET_DIR=/build/linux
  export "CXX_${t//-/_}=$t-clang++"
  # SDL's CMake finds Wayland, PipeWire, PulseAudio, D-Bus... with pkg-config: the
  # target's (multiarch) .pc files, not the build machine's.
  export PKG_CONFIG_LIBDIR=/usr/lib/x86_64-linux-gnu/pkgconfig:/usr/share/pkgconfig
  xx-cargo build --release --locked -p bugatron --features embed-game,bugatron-runtime/sdl3-from-source
  mkdir -p /out/bugatron-linux-x64
  cp "$CARGO_TARGET_DIR/$t/release/bugatron" /out/bugatron-linux-x64/
  ;;
windows)
  SDL=/opt/win-sdl/x86_64-w64-mingw32
  export CARGO_TARGET_DIR=/build/windows BUGATRON_DXC=/opt/dxc/bin/dxc
  export PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_PATH=$SDL/lib/pkgconfig PKG_CONFIG_LIBDIR=$SDL/lib/pkgconfig
  export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
  cargo build --release --locked --target x86_64-pc-windows-gnu -p bugatron --features embed-game
  mkdir -p /out/bugatron-windows-x64
  cp "$CARGO_TARGET_DIR/x86_64-pc-windows-gnu/release/bugatron.exe" $SDL/bin/SDL3.dll /out/bugatron-windows-x64/
  (cd /out && zip -qr bugatron-windows-x64.zip bugatron-windows-x64)
  ;;
*) echo "usage: $0 linux|windows" >&2; exit 2 ;;
esac
ls -la /out
