# Bugatron (standard) 1.04: analysis record

Reference only. This is not a supported target, and no addresses are shared with any other version.

## Identity

| Item | Value |
|---|---|
| Installer | `reference/Bugatron104Setup.exe`, 3,095,229 bytes, sha256 `eab75e0ba188d616202a232bc0f0da36258afae9468e89802d0977d72a136915` |
| Installer format | Inno Setup 1.3.25 (ISX). `innoextract` reads it (`tools/extract_reference.sh`) |
| Executable | `bugatron.exe`, 241,664 bytes, sha256 `78788eaea1e2e8c75f2d7af2f34e8b13a2084d852187660506fb6c562aae2cbc`, PE timestamp 2001-04-04 |
| Copyright | "Bugatron, Copyright 2000, Retro64, Inc." (readme). The title screen says "Copyright 2001". Program by Michael W Boeh, music by Robert Odman |

## Contents

- A shareware build: `bug1`–`bug4` scripts with 4–5 levels each, plus one level pack. Music and sound volume are on Page Up/Down and Home/End, P pauses, F12 takes a screenshot (`ss.bmp`).
- **Audio: BASS** (`bass.dll`) with `.mo3` music. Gold 1.20 uses FMOD 3.61 with XM.
- **Graphics:** the same DirectDraw/Direct3D approach ("requires DirectX 7.0 or later").
- **Imports:** 124, from BASS, DDRAW, GDI32, KERNEL32, SHELL32, USER32, WININET and WINMM.
- **Purchase link:** the string `http://www.retro64.com/purchase.html` is hard-coded and appears alongside `InternetGetConnectedState`. It is presumably opened only when the machine is online. That is not confirmed, and the call site has not been traced.
- **No menu links:** the scripts have no `LINK=` items. That mechanism (Gold 1.20's 0x4103a0) is later.
- It ships a `WWW/` folder of GIF and JPEG images (an order page's art) and the same three `website*.url` shortcuts as Gold 1.20.

## Assessment

This is an earlier generation of the game, not a variant of Gold 1.20. Supporting it would be a separate port, with its own recompile and a BASS audio layer.
