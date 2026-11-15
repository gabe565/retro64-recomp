# Bugatron (standard) 1.51d — analysis record (reference only)

Addresses here refer to this executable only.

## Identity

| Item | Value |
|---|---|
| Distribution | `reference/Bugatron_Win_EN_v151d.exe`, 4,652,896 bytes, sha256 `42f825a1…a96d4a`. A wrapper stub (`APPLICATION_NAME=Bugatron`, `BugatronSetup.exe`) with an Inno Setup 4.2.6 installer appended at file offset `0x2d004`. Carving the inner installer lets `innoextract` read it. |
| Game executable | `app/Bug3D.exe`, 307,200 bytes, sha256 `74956a45747c7d2329551125c2969d4f66202e48322ec511fe1c2fc8d739af26`, linked 2003-11-30 01:31:51 UTC |
| Launcher | `app/Bugatron.exe`, byte-identical to Gold 1.20's launcher |
| Extra DLLs | `fmod.dll` (newer build, UPX), `devil.dll` (DevIL image library, UPX), `ReflexiveArcade/ReflexiveArcade.dll` (2004, Reflexive Arcade DRM and trial wrapper) |

## Why this is not the primary target

The README calls this version "Totally new version 1.5! Far too many changes to list." The decoded data confirms a substantially different game:

- **New weapons and models**: `tripleshot`, `heatseeking` (guided rockets), `railgun`, `burstbomb`, `plasma1`, `rocket`, `burst`. The ship has a `SHIP_RAILGUN_FIRE_SOUND`.
- **Different asset formats.** Audio is `.ogg` and music is `.oxm`. Images go through DevIL (`ilLoadL`), and the executable references `.tga` and `.jpg` files. Most assets are packed in `smash.wad` / `smash.ndx`, an unresearched archive format.
- **Commercial wrapper.** It has a "FREE TRIAL %d" session limit, Reflexive Arcade menu sessions (`radll_*`), and ADVAPI32 `CryptVerifySignature` checks of the Reflexive DLL.
- It adds three `reflexive*.xml` configs.

Its packs list the same six level packs, and the `.lev` encoding is unchanged: all 32 files decode.

## Use in this project

Use it only to cross-check shared code idioms such as the r3d engine and XML parser. Do not import its behavior, balance, or weapons into the Gold 1.20 reconstruction.
