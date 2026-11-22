# Retro64 Recompiled

Static recompilations of Retro64 games, so they run natively on macOS, Linux and Windows:

| Game | Program | State |
|---|---|---|
| **Bugatron Gold v1.20a** (2003) | `bugatron` | Playable from start to game over on macOS, Linux (Steam Deck) and Windows |
| **Z-Ball Gold** (2002) | `zball` | Playable on macOS; not yet compared against the original |
| **Bugatron Worlds** v1.01a, free trial (2006) | `bugatron-worlds` | Playable on macOS (menus, levels, sound); no settings overlay yet; not yet compared against the original |

Each game's original 32-bit x86 code is translated to C at build time and compiled for your machine. The parts of Windows the games relied on are replaced by a portable runtime written in Rust on SDL3: DirectDraw and Direct3D 6, FMOD 3 and the Win32 API. The gameplay is the original code, not a rewrite. It is not an emulator, and no original Windows code runs at run time.

The games run on versions of the same Retro64 engine, so they share the runtime. What is specific to each game (its keys and the addresses of the settings the overlay changes) lives in a small profile in its own crate. Validation against the originals is ongoing; see [docs/runtime-status.md](docs/runtime-status.md).

## You need your own copy of the game

This repository contains no game files, and builds of it must not include them. You supply the games yourself; only the executable's hash is checked when you build or run, and the other hashes let you confirm a download.

**Bugatron Gold v1.20a**, listed on [MyAbandonware](https://www.myabandonware.com/game/bugatron-g23) as "Bugatron Gold version 1.20". The other versions listed there are different releases and are not supported.

| File | SHA-256 |
|---|---|
| `Bugatron_Win_EN_Gold_120.zip` (the download) | `f3ee37e54c998eb8167c997f95b9dd4c81c482633c454bb2d4d5ba2f52e894f9` |
| `buggold_120.exe` (the installer inside it) | `4519e88049371d562d4a932f4701a81da6360f84d30a8215a1401ef94ce4669b` |
| `Bug3D.exe` (installed) | `be581a63bccfc7dcc78a5c55cacc02e9f7aab9e5398540d2cc86a7aba758919e` |

**Z-Ball Gold** v1.07, the March 2002 build:

| File | SHA-256 |
|---|---|
| `Z-Ball_Win_EN_Full-Rip.zip` (the installed game folder, zipped) | `2b733a874bcf16d2a301c27e64438739402e36c7216cb6a7aa8d0638381ebcad` |
| `Z-Ball.exe` (installed) | `599539b2cbf0b2235921b5ee486be9af4502c715eeb750c1aeed807d2e6f841c` |

**Bugatron Worlds** v1.01a, the free trial (Retro64's own download; the full version is not known to survive). Its 30-minute time limit is the game's own and is left as it is.

| File | SHA-256 |
|---|---|
| `bugatron_worlds.exe` (the installer) | `fb5ee4a6a877f382dab311f6569e119562a50d28a652c3f29148a7a8aba97e7b` |
| `BugatronWorlds.exe` (installed) | `08ed700fe1f40804444683a08a20740cdb660bcc7ffcc08eb6f3e804f3ca0e17` |

## Building

Prerequisites:

- **Rust** 1.93 or later and a **C compiler** (clang or GCC). The translated game code is C.
- **SDL3**, found through pkg-config:
  - macOS: `brew install sdl3`
  - Debian 13: `apt install libsdl3-dev`

Put the game's zip or installer in `reference/`, then build its program:

```sh
mkdir -p reference
cp /path/to/Bugatron_Win_EN_Gold_120.zip /path/to/Z-Ball_Win_EN_Full-Rip.zip /path/to/bugatron_worlds.exe reference/
cargo build --release -p bugatron   # or -p zball, -p bugatron-worlds
```

The build reads the game straight from the zip or installer, unpacking it in memory, and translates the executable to C. `BUGATRON_EXE`, `ZBALL_EXE` or `BUGATRON_WORLDS_EXE` can point at the zip, the installer or the installed executable instead. `reference/`, `_work/` and `target/` are gitignored, and the games' files and the generated code are never committed.

The recompiler starts from a committed list of function and jump-table addresses for each game ([`hints/`](hints/README.md)), produced once by Ghidra and tied to the executable's hash. Ghidra is only needed to regenerate them. `tools/extract_reference.sh [--ghidra]` (innoextract, Ghidra 12.x) sets up the analysis workspace in `_work/`.

### Building with Docker (Bugatron)

With only [Docker](https://docs.docker.com/get-docker/) installed, this builds Linux x86-64 and Windows x86-64 binaries of Bugatron with the game built in. Put the downloaded zip (or the installer, or the installed game folder's contents) in a folder of its own (Docker reads the whole folder), then:

```sh
docker build --build-context game=path/to/that/folder --output bugatron \
  https://github.com/gabe565/retro64-recomp.git
```

- The binaries land in `./bugatron`: `bugatron-linux-x64/bugatron` (glibc 2.38 or newer, so the Steam Deck and current distributions) and `bugatron-windows-x64/` (`bugatron.exe` with `SDL3.dll`, also zipped). Add `--target linux` or `--target windows` to build just one.
- It builds natively on the machine you run it on, Apple Silicon included, and cross-compiles. A first build takes a few minutes; compiled dependencies stay in Docker's build cache, so later builds are faster. No image is left behind; `docker builder prune` clears the cache.
- macOS binaries can't be built in Docker; use the steps above.
- The binaries contain the original game files, so keep them to your own devices.

### Other builds

```sh
cargo build --release -p bugatron --features embed-game   # or -p zball, -p bugatron-worlds
```

`embed-game` builds the whole game folder (about 11 MB for Bugatron) into the binary, so you can copy one file to another machine of the same platform. It then contains the original game files, so keep it to your own devices.

`tools/build-portable.sh` cross-builds Bugatron for Linux x86-64 (SDL3 linked in; glibc 2.38 or newer, which covers the Steam Deck) and Windows x86-64 (`bugatron.exe` with `SDL3.dll`) into `_work/dist/`. Run it on x86-64 Linux, such as an OrbStack machine; the script lists what it needs. Windows builds are cross-compiled with MinGW, and their Direct3D 12 shaders compiled with DXC; see [docs/testing/windows-validation.md](docs/testing/windows-validation.md).

## Running

```sh
cargo run --release -p bugatron   # or -p zball, -p bugatron-worlds
```

The game is found automatically, in this order of folders:

1. the current directory;
2. the program's own folder;
3. on Windows, `C:\Program Files (x86)\Bugatron Gold`, `Z-Ball Gold` or `Bugatron Worlds`;
4. the folder of the copy the program was built from.

In each folder, the files are tried in the order of the tables above: `Bug3D.exe`, then `buggold_120.exe`, then `Bugatron_Win_EN_Gold_120.zip`; `Z-Ball.exe`, then `Z-Ball_Win_EN_Full-Rip.zip`; or `BugatronWorlds.exe`, then `bugatron_worlds.exe`. Use `--exe FILE` (or `BUGATRON_EXE`, `ZBALL_EXE`, `BUGATRON_WORLDS_EXE`) to choose one. An installer or zip is unpacked in memory at startup. Nothing is written except your saves.

| Option | |
|---|---|
| `--exe FILE` | The executable, the installer, or the zip |
| `--data DIR` | The game folder, if it isn't next to the executable |
| `--save DIR` | Where prefs, high scores and `config.toml` go. The default is the game's per-user data folder, printed at startup: `~/Library/Application Support/<game>` on macOS, `~/.local/share/<game>` on Linux, `%APPDATA%\<game>` on Windows, where `<game>` is `bugatron`, `zball` or `bugatron-worlds`. Game files are never modified |
| `--windowed` | Start in a window. The game is fullscreen by default, as the original was |
| `--run-in-background` | Keep running when another app has focus (the original paused) |
| `--mute` | Run without sound: no audio device is opened. `--record-audio` still records the mix |
| `--video-mode WxHxBPP` | For example `1600x1200x32` or `640x480x16`. The default is the largest 4:3 mode that fits your display. Bugatron Worlds starts at 800x600; its Options > Video Prefs switches to this mode, and the game keeps it for next time |

`--help` lists everything, including the testing options (`--input`, `--frames`, `--dump-frames`, `--record-audio`).

In Bugatron and Z-Ball, **F10** (or a controller's **Back** button) opens the [settings overlay](#settings-overlay). In every game, **Alt+Enter** (or **Ctrl+Cmd+F** on macOS) toggles fullscreen.

### Bugatron controls

| Key | Action | Also |
|---|---|---|
| Arrows | Move | |
| Ctrl, Space | Fire (Space can't be remapped: names use it) | |
| F1 | Start game | any fire button |
| F2 | High scores | |
| F3 | Difficulty | |
| P | Pause | controller Start |
| Esc | Exit | |
| Page Up / Page Down | Music volume | `=` / `-` |
| Home / End | Sound volume | `]` / `[` |
| Insert | Screenshot (`screenshot<N>.bmp` in the save folder) | |
| Delete | Show FPS and render/logic times | |

Any controller SDL recognises works through the game's own joystick support: left stick or D-pad to steer, A/B/X/Y or the right trigger to fire. Menus still need the keyboard or mouse.

### Z-Ball controls

| Key | Action | Also |
|---|---|---|
| Mouse (up and down) | Move the paddle, on the right edge | Up / Down arrows |
| Ctrl | Launch the ball | |
| F1 | Start game, then F1 to F9 pick a level set | |
| F2 | High scores | |
| F3 | Difficulty | |
| P | Pause | controller Start |
| Esc | Exit | |
| Page Up / Page Down | Music volume | `=` / `-` |
| Home / End | Sound volume | `]` / `[` |
| F12 | Screenshot (in the save folder) | |
| F11 | Show FPS, polygons and render/logic times | |

### Bugatron Worlds controls

Its menus use the mouse. In play, the keys are Bugatron's (the same key table): arrows to move, Ctrl or Space to fire, P to pause, Esc to exit, F2 for the high scores, Page Up/Down and Home/End for the volumes, Insert for a screenshot. They can be rebound in `config.toml` (the same action names as Bugatron's).

### Settings overlay

F10 (or controller Back, rebindable as `settings`) pauses the game as P does, with the music still playing, so volume changes can be heard. Bugatron Worlds doesn't have it yet: its font and settings aren't mapped. In Bugatron and Z-Ball it:

- changes the difficulty and the music and sound volumes, on any screen;
- sets the resolution (from the next start) and fullscreen;
- shows the game's FPS counter;
- opens the high scores from any screen. A game in progress ends: Bugatron doesn't record its score;
- quits;
- edits the key and button bindings on its Controls page.

For Bugatron it also skips the "LOADING" screen between screens (on by default) and can swap the title logo for the free version's plain "BUGATRON", recreated from the Gold logo. Z-Ball has no loading screen to skip.

### config.toml

The first run writes `config.toml` to the save folder, with every setting and a comment on each. The overlay rewrites it; you can also edit it while the game is closed. Its `[bindings]` list the keys and controller buttons for each action (SDL key names, or `Pad ` and an SDL button name). Actions missing from the file keep their defaults, and a key you remove from its action stops working. For example:

```toml
[bindings]
start = ["F1", "Return"]
high_scores = ["F2", "Tab"]
pause = ["P", "Pad start"]
```

The action names are in the file:
- **Bugatron:** `left`, `right`, `up`, `down`, `fire`, `start`, `high_scores`, `difficulty`, `pause`, `exit`, `music_up`, `music_down`, `sound_up`, `sound_down`, `screenshot`, `frame_time`.
- **Z-Ball:** `up`, `down` and `launch`, then the same from `start` on.

**Bug fixes:** Bugatron's title screen no longer reverts to "NORMAL" difficulty after a game or the high scores. For the original's behaviour, set `difficulty_label = false` under `[fixes]`. Z-Ball doesn't have this bug.

## Platforms

| Platform | Graphics backend | State |
|---|---|---|
| macOS (Apple Silicon) | Metal | All three games played and tested |
| Linux x86-64 / arm64 | Vulkan | Bugatron played on a Steam Deck (x86-64, 2026-11-17). Tests pass; frames match Metal (Mesa lavapipe) |
| Windows x86-64 | Direct3D 12 | Bugatron played on a hardware GPU (2026-11-17). Tests pass; frames and audio match macOS (WARP, in a VM) |

## Testing

```sh
cargo test --workspace
BUGATRON_REQUIRE_GPU=1 cargo test -p bugatron-runtime-tests --test gpu   # fail rather than skip without a GPU
```

Building the whole workspace needs every game. Without one, leave its crates out: `--exclude bugatron --exclude bugatron-poc`, `--exclude zball` or `--exclude bugatron-worlds`.

The tests cover:
- x86 instruction semantics, checked against results recorded on real x86 hardware;
- the runtime's Win32, DirectDraw, Direct3D and FMOD behaviour;
- rendering, through the real GPU backend.

Comparisons against the original game use Frida in a Windows VM; see `tools/frida/` and [docs/testing/](docs/testing/).

**CI** ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) runs on Linux (Debian 13, Vulkan on Mesa lavapipe), macOS and Windows (Direct3D 12 on WARP):
- **Game-free tests** run on every push and pull request.
- **Each game's tests and a short scripted play session** run when its repository secret points to a private copy of its download, checked against the hash above: `BUGATRON_GAME_URL` (`Bugatron_Win_EN_Gold_120.zip`), `ZBALL_GAME_URL` (`Z-Ball_Win_EN_Full-Rip.zip`) and `BUGATRON_WORLDS_GAME_URL` (`bugatron_worlds.exe`).

  Set `BUGATRON_GAME_TOKEN`, `ZBALL_GAME_TOKEN` or `BUGATRON_WORLDS_GAME_TOKEN` too if the URL needs a Bearer token. Between runs each download is kept in the Actions cache encrypted with its secrets, because pull requests from forks can restore a public repository's caches (but get no secrets to decrypt them). The games are never published as artifacts.

## Repository layout

| Path | |
|---|---|
| `crates/recomp` | The static recompiler: x86 → C |
| `crates/runtime` | The portable runtime: memory model, Win32, DirectDraw/Direct3D on SDL3 GPU, FMOD on SDL3 audio, libxmp and lewton (Ogg Vorbis), DevIL's image loading, the settings overlay |
| `crates/game` | The `bugatron` program: Bugatron's profile (keys, addresses) and build |
| `crates/zball` | The `zball` program: Z-Ball's profile and build |
| `crates/worlds` | The `bugatron-worlds` program: Bugatron Worlds' profile (the free trial) and build |
| `crates/launcher`, `crates/build`, `crates/unpack` | Shared by the game programs: the command line, the build-time translation, reading installers and zips |
| `crates/runtime-tests`, `crates/x86-*` | Runtime tests and the x86 semantics suite, with its hardware oracle |
| `crates/xmp-sys`, `third_party/libxmp-lite` | XM music playback (vendored libxmp-lite, MIT) |
| `hints/` | Recompiler hints (addresses only) for `Bug3D.exe`, `Z-Ball.exe` and `BugatronWorlds.exe` |
| `tools/` | Analysis workspace setup, Ghidra scripts, Frida probes, comparison and Windows kit scripts |
| `docs/` | Findings, formats, function map, runtime status, validation results ([index](docs/README.md)) |

## License

[GPL-3.0](LICENSE). Vendored libxmp-lite is MIT, which is compatible. Bugatron, Z-Ball and Bugatron Worlds belong to their rights holders; none of the games is included here.
