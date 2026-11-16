# Linux validation (OrbStack, no physical Linux PC)

Linux builds are validated in OrbStack machines on the development Mac. These are Debian 13 (trixie) machines: arm64 natively, and x86_64 through Rosetta. They have no GPU. Rendering uses Mesa's lavapipe (llvmpipe), a software Vulkan driver, under Xvfb. That exercises the SPIR-V shaders and SDL's Vulkan backend exactly as a real Vulkan driver would see them, but not any vendor's driver.

Why Debian 13: it packages SDL3 (`libsdl3-dev` 3.2.10; the build links the system SDL3 through pkg-config) and a current Mesa (25.0.7). Ubuntu 24.04 LTS has no SDL3 package; testing with a source-built SDL3 there is a later step.

## Setup

```sh
orbctl create debian:trixie bugatron-linux                     # arm64
orbctl create --arch amd64 debian:trixie bugatron-linux-x64   # x86_64 (Rosetta)
orbctl run -m <machine> -u root bash -c 'apt-get update && apt-get install -y \
  build-essential pkg-config curl libsdl3-dev mesa-vulkan-drivers xvfb \
  vulkan-tools vulkan-validationlayers'
orbctl run -m <machine> bash -c 'curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal'
```

The repository is shared at the same path. Use a separate target directory so the macOS build is untouched:

```sh
cd /Users/…/bugatron-recomp
export CARGO_TARGET_DIR=$HOME/target-linux
cargo test --release --workspace
BUGATRON_REQUIRE_GPU=1 xvfb-run -a cargo test --release -p bugatron-runtime-tests --test gpu
VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation …   # optional Vulkan validation
xvfb-run -a -s "-screen 0 1280x1024x24" cargo run --release -p bugatron -- \
  --video-mode 640x480x32 --frames 710 --dump-frames 139,450 --dump-dir OUT
```

Without a display the `gpu` tests report themselves as ignored, not passed. `BUGATRON_REQUIRE_GPU=1` turns that into a failure.

## Results (2026-11-16)

### Debian 13 arm64

- **Build and tests:** the workspace builds (about 1 min). The 92 tests that pass on macOS also pass here, including the x86 instruction-semantics suite.
- **Vulkan bug found and fixed (370e22a):** under Xvfb with lavapipe, 5 of the 7 GPU tests failed at first. Clears and CPU uploads were right, but no triangle appeared where expected.
  - Cause: naga's SPIR-V writer flips Y by default (`ADJUST_COORDINATE_SPACE`), and SDL's Vulkan backend already flips the viewport so clip space is Y-up on every backend. Every triangle was drawn upside down.
  - Fix: the flag is now removed in `crates/runtime/build.rs`. All 7 pass, and the Vulkan validation layers report nothing.
  - Before the fix the tests had "passed" without a display, because skips counted as passes. They now report as ignored.
- **Game frames, Vulkan (lavapipe) vs Metal (Apple GPU), title-screen sequence, 640×480×32, same input:**

  | Frame | Pixels that differ | Max channel diff |
  |---|---|---|
  | 139 | 762 / 307 200 (0.25 %) | 4 |
  | 450 | 34 702 / 307 200 (11.3 %) | 3 |

  These differences come from rasterisation and interpolation rounding (Gouraud colour, texture filtering) between GPUs. The scene and geometry are the same; game logic is identical because the generated C is compiled with `-ffp-contract=off -fno-fast-math` (no FMA contraction) on every compiler.

### Debian 13 x86_64 (Rosetta)

- **Build and tests:** the workspace builds (2 min 14 s under Rosetta). 85 tests pass without a display, with the 7 GPU tests reported as ignored; under Xvfb with `BUGATRON_REQUIRE_GPU=1` the 7 GPU tests pass too. That is the same 92 as on macOS.
- **Game frames**, same run as above:

  | Frame | vs Metal | vs Vulkan on arm64 |
  |---|---|---|
  | 139 | 473 px (0.15 %), max 4 | 607 px (0.20 %), max 4 |
  | 450 | 13 296 px (4.33 %), max 3 | 33 668 px (10.96 %), max 4 |

  llvmpipe on x86_64 and on arm64 also differ slightly from each other, because its rounding depends on the host's vector width. All three renderers agree to within 4/255 per channel.

## Not covered

- Real GPU drivers (NVIDIA, AMD, Intel Mesa), Wayland, and a real window with keyboard, mouse and audio output. The UTM "Linux" VM or a physical PC can cover the interactive part.
- Ubuntu 24.04 with a source-built SDL3.
- Audible output: in the container, SDL opens an audio device and the game plays its music into it, but nobody listened. Mixing itself is platform-independent and covered by the `audio`/`fmod` tests.
