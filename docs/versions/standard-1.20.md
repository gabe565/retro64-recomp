# Bugatron (standard) "v120" installer — analysis record (not extracted)

## Identity

| Item | Value |
|---|---|
| File | `reference/Bugatron_Win_EN_v120.exe`, 2,761,448 bytes, sha256 `a3c9dffa7fe3cc92647e58b7dfefd28810b19a0de115fbfaca73dcf526fb4e49` |
| Outer format | PE32 GUI, **UPX-packed**. `upx -d` on a copy gives 2,931,432 bytes. |
| Installer | **Ghost Installer**, identified by strings such as `GIPresetupWindow`, `Function "%s" is not available in this version of Ghost Installer.`, and the embedded `unpack.dll` (lcc build path `d:\work\cvs\sources\gi\base\common\unpack\lcc\unpack.dll`) |

## Status

- Neither `innoextract` nor `7z` can read the payload. No Ghost Installer extractor is installed.
- Extraction would need either running the installer under Windows or Wine, or reversing `unpack.dll`.
- The owner has said Gold 1.20 is the focus and the other releases are for reference. **This version is therefore parked.** It is probably the standard (non-Gold) 1.20 build, which would make it the closest sibling of Gold 1.20, but that is unverified.

## If needed later

Run the installer once in a disposable Windows VM and copy out the installed directory. Do not run it on the development host.
