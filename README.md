# Bugatron Recompiled

A static recompilation of **Bugatron Gold v1.20a** (Retro64, 2003), so it runs natively on macOS, Linux and Windows.

The game's original 32-bit x86 code is translated to C at build time and compiled for your machine. The parts of Windows it relied on are replaced by a portable runtime written in Rust on SDL3: DirectDraw and Direct3D 6, FMOD 3.61 and the Win32 API. The gameplay is the original code, not a rewrite. It is not an emulator, and no original Windows code runs at run time.

> **Status:** the game is playable from start to game over on macOS. Linux and Windows builds pass the test suite and render matching frames, but so far only in VMs and with software rendering. Validation against the original is ongoing; see [docs/runtime-status.md](docs/runtime-status.md).

## You need your own copy of the game

This repository contains no game files, and builds of it must not include them. You supply **Bugatron Gold v1.20a** (Retro64, Inc.) yourself, in any of these forms:

| File | SHA-256 |
|---|---|
| `Bugatron_Win_EN_Gold_120.zip` (the download) | `f3ee37e54c998eb8167c997f95b9dd4c81c482633c454bb2d4d5ba2f52e894f9` |
| `buggold_120.exe` (the installer inside it) | `4519e88049371d562d4a932f4701a81da6360f84d30a8215a1401ef94ce4669b` |
| `Bug3D.exe` (installed) | `be581a63bccfc7dcc78a5c55cacc02e9f7aab9e5398540d2cc86a7aba758919e` |

The game is listed on [MyAbandonware](https://www.myabandonware.com/game/bugatron-g23) as "Bugatron Gold version 1.20". The other versions listed there are different releases and are not supported. Only `Bug3D.exe` is checked when you build or run; the other hashes let you confirm a download.

## Building

Prerequisites:

- **Rust** 1.93 or later and a **C compiler** (clang or GCC). The translated game code is C.
- **SDL3**, found through pkg-config:
  - macOS: `brew install sdl3`
  - Debian 13: `apt install libsdl3-dev`

Steps:

```sh
mkdir -p reference && cp /path/to/Bugatron_Win_EN_Gold_120.zip reference/
cargo build --release -p bugatron
```

The build reads the game straight from the download: it unpacks the installer in memory and translates `Bug3D.exe` to C. `BUGATRON_EXE` can point at the zip, the installer or an installed `Bug3D.exe` instead. `reference/`, `_work/` and `target/` are gitignored, and the game's files and the generated code are never committed.

The recompiler starts from a committed list of function and jump-table addresses ([`hints/`](hints/README.md)), produced once by Ghidra and tied to the exe's hash. Ghidra is only needed to regenerate it. `tools/extract_reference.sh [--ghidra]` (innoextract, Ghidra 12.x) sets up the analysis workspace in `_work/`.

### Building with Docker

With only [Docker](https://docs.docker.com/get-docker/) installed, this builds Linux x86-64 and Windows x86-64 binaries with the game built in. Put the downloaded zip (or the installer, or the installed game folder's contents) in a folder of its own (Docker reads the whole folder), then:

```sh
docker build --build-context game=path/to/that/folder --output bugatron \
  https://github.com/gabe565/bugatron-recomp.git
```

- The binaries land in `./bugatron`: `bugatron-linux-x64/bugatron` (glibc 2.38 or newer, so the Steam Deck and current distributions) and `bugatron-windows-x64/` (`bugatron.exe` with `SDL3.dll`, also zipped). Add `--target linux` or `--target windows` to build just one.
- It builds natively on the machine you run it on, Apple Silicon included, and cross-compiles. A first build takes a few minutes; compiled dependencies stay in Docker's build cache, so later builds are faster. No image is left behind; `docker builder prune` clears the cache.
- macOS binaries can't be built in Docker; use the steps above.
- The binaries contain the original game files, so keep them to your own devices.

Windows builds are currently cross-compiled from x86-64 Linux with MinGW: `tools/windows/build-kit.sh` (see [docs/testing/windows-validation.md](docs/testing/windows-validation.md)). Direct3D 12 shaders are compiled with DXC.

## Running

```sh
cargo run --release -p bugatron
```

The game is found automatically. Each of these folders is checked for `Bug3D.exe`, then `buggold_120.exe`, then `Bugatron_Win_EN_Gold_120.zip`:

1. the current directory;
2. the program's own folder;
3. on Windows, `C:\Program Files (x86)\Bugatron Gold`;
4. the folder of the copy the program was built from.

Use `--exe FILE` (or `BUGATRON_EXE`) to choose one. An installer or zip is unpacked in memory at startup. Nothing is written except your saves.

| Option | |
|---|---|
| `--exe FILE` | `Bug3D.exe`, the installer, or the download zip |
| `--data DIR` | The game folder, if it isn't next to `Bug3D.exe` |
| `--save DIR` | Where prefs and high scores go. The default is your per-user data folder, printed at startup (`~/Library/Application Support/bugatron` on macOS, `~/.local/share/bugatron` on Linux, `%APPDATA%\bugatron` on Windows). Game files are never modified |
| `--windowed` | Start in a window. The game is fullscreen by default, as the original was |
| `--run-in-background` | Keep running when another app has focus (the original paused) |
| `--video-mode WxHxBPP` | For example `1600x1200x32` or `640x480x16`. The default is the largest 4:3 mode that fits your display |

`bugatron --help` lists everything, including the testing options (`--input`, `--frames`, `--dump-frames`, `--record-audio`).

**Controllers:** any controller SDL recognises works through the game's own joystick support: left stick or D-pad to steer, A/B/X/Y or the right trigger to fire. Menus still need the keyboard or mouse.

**Keys:** the game's own controls, plus **Alt+Enter** (or **Ctrl+Cmd+F** on macOS) to toggle fullscreen. Some game functions use Page Up/Down, Home/End and Insert/Delete. Keyboards without those keys can't reach them yet; remapping is planned.

### Single-file build for testing

```sh
cargo build --release -p bugatron --features embed-game
```

This builds the whole game folder (about 11 MB) into the binary, so you can copy one file to another machine of the same platform. It then contains the original game files, so keep it to your own devices.

To build for other machines without installing Rust on them, use [Docker](#building-with-docker), or `tools/build-portable.sh`, which cross-builds a Linux x86-64 binary (SDL3 linked in; glibc 2.38 or newer, which covers the Steam Deck) and a Windows x86-64 build (`bugatron.exe` with `SDL3.dll`) into `_work/dist/`. Run it on x86-64 Linux, such as an OrbStack machine; the script lists what it needs.

## Platforms

| Platform | Graphics backend | State |
|---|---|---|
| macOS (Apple Silicon) | Metal | Played and tested |
| Linux x86-64 / arm64 | Vulkan | Played on a Steam Deck (x86-64, 2026-11-17). Tests pass; frames match Metal (Mesa lavapipe) |
| Windows x86-64 | Direct3D 12 | Played on a hardware GPU (2026-11-17). Tests pass; frames and audio match macOS (WARP, in a VM) |

## Testing

```sh
cargo test --workspace
BUGATRON_REQUIRE_GPU=1 cargo test -p bugatron-runtime-tests --test gpu   # fail rather than skip without a GPU
```

The tests cover:
- x86 instruction semantics, checked against results recorded on real x86 hardware;
- the runtime's Win32, DirectDraw, Direct3D and FMOD behaviour;
- rendering, through the real GPU backend.

Comparisons against the original game use Frida in a Windows VM; see `tools/frida/` and [docs/testing/](docs/testing/).

**CI** ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) runs on Linux (Debian 13, Vulkan on Mesa lavapipe), macOS and Windows (Direct3D 12 on WARP). Each job runs one of two sets:
- **Game-free tests** run on every push and pull request.
- **The full suite plus a short scripted play session** runs when the repository secret `BUGATRON_GAME_URL` points to a private copy of `Bugatron_Win_EN_Gold_120.zip`. Set `BUGATRON_GAME_TOKEN` too if that URL needs a Bearer token. The download is checked against the hash above. Between runs it's kept in the Actions cache encrypted with those secrets, because pull requests from forks can restore a public repository's caches (but get no secrets to decrypt them). The game is never published as an artifact.

## Repository layout

| Path | |
|---|---|
| `crates/recomp` | The static recompiler: x86 → C |
| `crates/runtime` | The portable runtime: memory model, Win32, DirectDraw/Direct3D on SDL3 GPU, FMOD on SDL3 audio and libxmp |
| `crates/game` | The `bugatron` program: translates `Bug3D.exe` at build time and runs it |
| `crates/runtime-tests`, `crates/x86-*` | Runtime tests and the x86 semantics suite, with its hardware oracle |
| `crates/xmp-sys`, `third_party/libxmp-lite` | XM music playback (vendored libxmp-lite, MIT) |
| `hints/` | Recompiler hints (addresses only) for Gold v1.20a's `Bug3D.exe` |
| `tools/` | Analysis workspace setup, Ghidra scripts, Frida probes, comparison and Windows kit scripts |
| `docs/` | Findings, formats, function map, runtime status, validation results ([index](docs/README.md)) |

## License

[GPL-3.0](LICENSE). Vendored libxmp-lite is MIT, which is compatible. Bugatron itself belongs to its rights holders; none of it is included here.
