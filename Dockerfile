# syntax=docker/dockerfile:1
# Builds Linux x86-64 and Windows x86-64 binaries from your own copy of the game,
# with nothing but Docker installed (README.md, "Building with Docker"):
#
#   docker build --build-context game=path/to/folder-with-the-zip \
#     --output bugatron https://github.com/gabe565/retro64-recomp.git
#
# --target linux or --target windows builds just one. The game folder holds the
# downloaded zip, the installer or the installed game. Nothing is kept as an image:
# the binaries go to --output, and compiled dependencies stay in BuildKit's cache.
#
# Everything runs natively on the build machine (also on Apple Silicon) and
# cross-compiles to x86-64; only DXC, the Windows shader compiler, is an x86-64
# program and runs emulated there.

# Replaced by --build-context game=DIR.
FROM scratch AS game

FROM --platform=$BUILDPLATFORM tonistiigi/xx:1.10.0 AS xx

FROM --platform=$BUILDPLATFORM rust:1.100-slim-trixie AS base
COPY --from=xx / /
RUN rm -f /etc/apt/apt.conf.d/docker-clean
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt/lists,sharing=locked \
    apt-get update && apt-get install -y --no-install-recommends clang cmake g++ lld make pkg-config zip
COPY . /src
WORKDIR /src
RUN cargo fetch --locked

# Linux x86-64, with SDL3 linked statically. Its build needs SDL3's Linux
# dependencies for the target (wiki.libsdl.org/SDL3/README-linux); SDL loads the
# libraries themselves at run time, so ones missing on the player's machine are
# skipped. Debian trixie: the binary needs glibc 2.38 or newer.
FROM base AS linux-build
ENV TARGETPLATFORM=linux/amd64
# wayland-scanner runs on the build machine.
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt/lists,sharing=locked \
    apt-get update && apt-get install -y --no-install-recommends libwayland-bin
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt/lists,sharing=locked \
    xx-apt-get update && xx-apt-get install -y --no-install-recommends xx-c-essentials xx-cxx-essentials \
      libasound2-dev libdbus-1-dev libdecor-0-dev libdrm-dev libegl1-mesa-dev libgbm-dev \
      libgl1-mesa-dev libibus-1.0-dev libjack-dev libpipewire-0.3-dev libpulse-dev \
      libsndio-dev libudev-dev libusb-1.0-0-dev libwayland-dev libx11-dev libxcursor-dev \
      libxext-dev libxfixes-dev libxi-dev libxinerama-dev libxkbcommon-dev libxrandr-dev \
      libxss-dev libxtst-dev wayland-protocols
RUN xx-cargo --setup-target-triple
RUN --mount=type=bind,from=game,target=/game \
    --mount=type=cache,target=/build/linux \
    tools/docker/build.sh linux

# Windows x86-64: MinGW, SDL3's MinGW package (SDL3.dll), and DXC for the shaders.
# DXC's Linux release is x86-64 only, so it gets the x86-64 C runtime (multiarch;
# binfmt runs it on other build machines, as Docker Desktop and OrbStack set up).
FROM base AS windows-build
RUN dpkg --add-architecture amd64
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt/lists,sharing=locked \
    apt-get update && apt-get install -y --no-install-recommends \
      g++-mingw-w64-x86-64 gcc-mingw-w64-x86-64 libc6:amd64 libgcc-s1:amd64 libstdc++6:amd64 zlib1g:amd64
RUN rustup target add x86_64-pc-windows-gnu
ADD --checksum=sha256:96faadc7f5c282d2ffda49804beb4c3ee38127bc252b723234e3c5cdf7aa39a1 \
    https://github.com/microsoft/DirectXShaderCompiler/releases/download/v1.9.2609/linux_dxc_2026_09_28.x86_x64.tar.gz /tmp/dxc.tar.gz
ADD --checksum=sha256:d2d3bf3fb97a90b22e908a306d7362631ef1bab229d988fe6669595dc29cd9ed \
    https://github.com/libsdl-org/SDL/releases/download/release-3.4.20/SDL3-devel-3.4.20-mingw.tar.gz /tmp/sdl.tar.gz
RUN mkdir /opt/dxc /opt/win-sdl \
    && tar -xzf /tmp/dxc.tar.gz -C /opt/dxc \
    && tar -xzf /tmp/sdl.tar.gz -C /opt/win-sdl --strip-components=1 \
    && rm /tmp/dxc.tar.gz /tmp/sdl.tar.gz
RUN --mount=type=bind,from=game,target=/game \
    --mount=type=cache,target=/build/windows \
    tools/docker/build.sh windows

FROM scratch AS linux
COPY --from=linux-build /out/ /

FROM scratch AS windows
COPY --from=windows-build /out/ /

FROM scratch AS all
COPY --from=linux-build /out/ /
COPY --from=windows-build /out/ /
