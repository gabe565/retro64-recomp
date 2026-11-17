# Bugatron (free version) "v1.20": analysis record

Reference only. This is not a supported target, and no addresses are shared with any other version.

## Identity

| Item | Value |
|---|---|
| Installer | `reference/Bugatron_Win_EN_v120.exe`, 2,761,448 bytes, sha256 `a3c9dffa7fe3cc92647e58b7dfefd28810b19a0de115fbfaca73dcf526fb4e49`. The same file is in OldGamesDownload's `Bugatron_v1.20_Win_Setup_EN.zip` |
| Installer format | **Ghost Installer**, UPX-packed. `upx -d` gives 2,931,432 bytes. The payload (about 2.6 MB from about 0x50000) uses Ghost Installer's own compression (`unpack.dll`): not zlib, LZMA or CAB, so no extractor reads it |
| Extraction | Installed once in the reference VM (2026-11-16), to `C:\Program Files (x86)\Bugatron (Free Version)\`, then copied to `_work/ext/v120free/app` |
| Executable | `Bug3D.exe`, 270,336 bytes, sha256 `cca842c425e19b8f8423083b6084fae1ef53dafae2697ff7703406d6a92e6170`, PE timestamp **2003-07-05**, four months after Gold 1.20 (2003-03-19) |
| `fmod.dll` | Identical to Gold 1.20's (sha256 `30ff0cb1…`) |

## Contents

- A free (shareware) build of the Gold 1.20 engine:
  - `bug1`–`bug4` plus `pack1`–`pack6` scripts and nine `.hs` files;
  - a **buy screen** in `config3.xml`: "GET BUGATRON GOLD!", "88 LEVELS OF PURE FUN!", "ORDER ONLINE, PHONE, MAIL, OR FAX!".
- **Menu links are used:** the buy screen has `LINK="LINK_BUY"` and `LINK="LINK_TO_MENU"`. So the link mechanism, which Gold 1.20 contains but never uses (function map, 0x4103a0), is live here. `LINK_BUY` passes the second token of `website3.url` to `ShellExecuteA`, as in Gold 1.20.
- **Audio** is still FMOD 3.61, but in Ogg Vorbis formats: 24 `.ogg` sound effects and 6 `.oxm` songs (XM with Vorbis-compressed samples). Gold 1.20 uses WAV and XM.
- **Code:** `.text` is 0x35df5 bytes, against 0x33965 for Gold 1.20. About 42 % of Gold 1.20's code (16-byte windows) occurs unchanged despite shifted addresses, consistent with the same engine.
  - There are two small extra sections: `.text1`, 20 bytes of ordinary code, and `.data1`, float constants (1.0, π…). They look like a toolchain artefact, not a protection stub.
- **Imports:** 119, every one also imported by Gold 1.20. The Gold 1.20 runtime implements all of them.

## What supporting it would take

- **A second translation target:**
  - its own pinned SHA-256;
  - recompiler hints (a one-time Ghidra export, committed like `hints/gold-1.20a.json`);
  - its C++ exception-handler addresses;
  - its fullscreen-projection and focus addresses, re-checked.
- **FMOD:** `.ogg` samples in `FSOUND_Sample_Load`, and `.oxm` songs. libxmp's XM loader already decodes OXM through its bundled stb_vorbis, which may need enabling in our build. The same decoder could serve the `.ogg` effects.
- **Installer:** users must install it on Windows (or Wine) and point `--exe` at the installed `Bug3D.exe`. Reading the Ghost Installer directly would mean reverse-engineering `unpack.dll`'s decompressor.
