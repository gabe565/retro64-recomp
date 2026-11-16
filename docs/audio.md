# Audio: FMOD 3.61 on SDL3 audio and libxmp

The game uses 13 FMOD 3.61 functions. The runtime implements them on its own software mixer (`crates/runtime/src/audio/mixer.rs`), plays XM music with the vendored libxmp-lite 4.7.4 (`third_party/libxmp-lite`, MIT), and outputs through SDL3 audio.

## How the game uses FMOD (from Bug3D.exe, 0x416b40–0x417320)

| Call | Use |
|---|---|
| `FSOUND_SetMaxHardwareChannels(0)` | Software mixing only |
| `FSOUND_Init(44100, 32, 0)` | Falls back to 22050 Hz, then 16 channels. If every attempt fails, sound is disabled (0x438144 = 0) |
| `FSOUND_Sample_Load(FSOUND_UNMANAGED, "audio\\x.wav", 0, 0)` | Format from the file. All 25 effects are mono PCM: 8 or 16-bit, 11025/22050/44100 Hz, without `smpl` loops |
| `FSOUND_Sample_SetMode(s, FSOUND_LOOP_NORMAL)` | Some effects loop |
| `FSOUND_PlaySound(FSOUND_FREE, s)` | The returned channel is stored but never used. There is no per-channel stop, volume, pan, or frequency call |
| `FSOUND_Sample_Free` | On scene changes; this stops the sample's channels |
| `FSOUND_SetSFXMasterVolume(0..255)` | Home/End, and the fades between scenes |
| `FMUSIC_LoadSong` / `PlaySong` / `SetPaused` / `SetMasterVolume(0..256)` / `FreeSong` | One XM per scene: title `ky_sfa`, gameplay `ky_ck`, game over `ky_dras`, … `SetPaused` is called every frame (pause when focus is lost); the volume ramps down over scene changes |
| `FSOUND_Close` | On exit |

## Behaviour reproduced

- **Channels:** `FSOUND_FREE` takes the lowest free channel. With all channels busy, it steals the oldest channel whose priority is not higher than the new sound's (all are 128), and returns −1 if none qualifies. Channels free themselves when a one-shot sound ends.
- **Sample defaults:** frequency from the file (so pitch and speed are the file's), volume 255, pan 128, priority 128, loop off. A `smpl` loop makes the default mode loop. `SetMode` changes the loop mode, including for channels already playing.
- **Levels:**
  - A sound's amplitude is scaled by volume/255 × SFX master/255, then panned linearly: left (255 − pan)/255, right pan/255.
  - Music is added at its master volume/256 and is not affected by the SFX master.
  - The sum is clipped to the 16-bit range.
- **Resampling:** linear interpolation.
- **Music:** songs loop. `PlaySong` restarts from the start and unpauses. Freeing a song stops it.
- **Lifecycle:** freeing a sample stops its channels. `FSOUND_Close` stops everything and allows a new `Init`.
- **Timing:** mixing runs on SDL's audio thread, pulled at FMOD's mix rate (44100 Hz). The game thread only takes a short lock to issue commands, so audio never blocks the simulation, and the simulation clock never depends on the audio device.
- **Background (focus):** the game calls `FSOUND_Init(44100, 32, 0)`, so it does not ask for global focus, and DirectSound silenced all of its output (music and effects) while another application had focus. The owner observed this in the VM. FMOD kept mixing meanwhile.
  - The runtime matches this: while the window is unfocused, the device outputs silence, but mixing and `--record-audio` continue.
  - Separately, the game itself pauses its music and stops updating on `WM_KILLFOCUS` (window procedure 0x407d10, see runtime-status.md).
- **No device:** FMOD-visible behaviour is unchanged, but nothing is output. This applies in tests, and if `FSOUND_Init` cannot open a device it fails as FMOD would, so the game disables sound itself.

Verified by scripted play. Title, menu, gameplay, game over, title, and a new game load, play, fade, and free the right songs and effects. `BUGATRON_TRACE=fmod` logs every call except the per-frame `SetPaused`. `BUGATRON_TRACE=audio` reports audio callbacks that arrive late or wait for the mixer lock.

## Approximations and open checks

| # | Item | Check |
|---|---|---|
| AU1 | Music comes from libxmp, not FMOD 3.61's own XM player. Tempo and pitch match a capture of the original; the original's music is 2.1 dB louder at volume 56/256 (open). Effect quirks, interpolation, and volume ramping may differ | Captures (below) |
| AU2 | The pan law (linear, −6 dB at centre) and the channel-stealing rule follow the FMOD 3 documentation. The game never sets pan, and 32 channels are rarely exhausted | Captures; stealing is only audible with more than 32 simultaneous sounds |
| AU3 | `FSOUND_PlaySound` returns the plain channel index. FMOD 3.6 may encode extra bits; the game never uses the value | — |
| AU4 | Output is resampled by SDL from 44100 Hz to the device rate | — |
| AU5 | Mixing uses floats, then clips. FMOD mixed in integers, so there may be ±1 LSB differences | Captures |

## Comparing with the original

- **Original:** `tools/frida/capture_fmod_output.js` adds a DSP unit to the original's FMOD (priority 1100, above the clip-and-copy unit). It receives the final 16-bit stereo mix and sends 12 s of it from the first `PlaySong` onwards. Spawn the game with this script plus only a `DialogBoxParamA` hook that returns IDOK for calls from Bug3D.exe. Do not combine it with `capture_deterministic_frames.js`: that script makes `FSOUND_Init` fail.
- **Native:** `bugatron --frames N --record-audio native.wav 12` records the mixer output from the first song onwards.
- **Results:** see below.

### Results (2026-11-15)

- **Original in the reference VM.** Launched normally, the original plays its music; the owner listened to the menu music. There are occasional pops and skips, most likely because the VM cannot keep up with real-time audio under x86 emulation.
  - When the game loses focus its sound stops. This is expected: DirectSound buffers are silent while their window is in the background, and the game pauses its music on `WM_KILLFOCUS` (`FMUSIC_SetPaused`).
  - Separately, the guest's audio went silent after pausing and resuming the VM with `utmctl`; quitting UTM fixed it.
- **Instrumented launches.** When the original is *spawned under Frida*, its `FSOUND_Init` fails at 44100 Hz, 22050 Hz, and with 16 channels, and the game disables sound (0x438144 = 0). DirectSound itself works in the VM, as a 32-bit probe confirmed. Unverified hypothesis: FMOD 3 binds DirectSound to the foreground window at `Init` when `FSOUND_SetHWND` was not called, and a spawned process is not in the foreground.
  - Fix to try: call `FSOUND_SetHWND` with the game's window from the capture script before `Init`, or launch normally and attach the capture after `Init`.
  - The DSP tap records FMOD's internal mix, so VM playback dropouts do not appear in a capture.
- **Correction (2026-11-15, later):** a spawn with only a dialog auto-OK hook initialised FMOD fine without being in the foreground. The `Init` failures came from the clock/timeline script (`capture_deterministic_frames.js`), not from focus; which hook is responsible is not yet known.
- **Capture attempts that hurt the VM.**
  - The first tap used a stdcall callback. FMOD 3.61 calls DSP units with cdecl (verified in the unpacked fmod.dll: its own units end in a plain `ret`), so the game crashed.
  - The second tap copied buffers through JS arrays and base64-encoded 2 MB inside FMOD's mixer thread. Mixing stalled, DirectSound looped its last buffer, and the game could not exit; the VM was rebooted.
  - The script now does only a native copy in the callback and sends chunks from a timer.
- **Native side.** The title music records at −29.3 dBFS RMS (peak 6743/32767) at the game's saved music volume of 56/256.
- **What holds without a capture:**
  - Effects play at their files' own sample rates, so pitch and speed are the files'.
  - XM tempo follows the module (libxmp).
  - The music and effects lifecycle across scenes matches the original's call sequence, which comes from the same recompiled code.
  - Still unverified: AU1 (relative music level and XM playback details) and AU2 (pan law).

### Results: capture of the original (2026-11-15, later)

The current capture script ran once without harming the game or the VM. It delivered all 12 s of the title music: 2 116 800 bytes at 44100 Hz. The native counterpart is `--video-mode 640x480x32 --record-audio native.wav 12`. Both recordings start at the first `PlaySong`, with the music volume at 56/256.

| Measure | Original (FMOD 3.61) | Native (libxmp) |
|---|---|---|
| RMS level | −27.0 dBFS | −29.3 dBFS |
| Peak | 8205 | 6743 |
| L / R RMS | 1433 / 1479 | 1128 / 1127 |
| Side/mid ratio | 0.023 | 0.015 |

- **Tempo and pitch match (AU1, partly confirmed).** The best alignment is at lag 0. Over 1 s windows from 0 to 9 s, the lag stays at 0, with a normalised correlation of 0.96–0.98. There is no drift, so the module tempo, row timing, and the playback rate match.
- **Music level differs (AU1, open).** A least-squares fit puts the original at 1.272× the native output (+2.1 dB). The residual after that gain is −12 dB below the signal, which leaves room for interpolation, ramping, and effect-detail differences. The level comes from a single setting (56/256), so a constant offset cannot yet be told apart from a different volume curve.
  - Candidate causes: libxmp's internal mixing scale (`XMP_PLAYER_AMP`/`MIX` defaults), and how FMOD 3.61's XM player applies `FMUSIC_SetMasterVolume`. Next step: read FMOD's music mixer in the unpacked DLL, or capture a second volume setting.
  - Unchanged for now: correcting the gain blindly could make other volumes worse.
- **Stereo (AU2).** The title music is almost mono in both recordings: the side signal is about 2 % of the mid. The original is slightly wider and slightly right-weighted. That fits XM panning that libxmp renders with less separation (libxmp's default stereo mix is 70 %), but the signal is too small to settle the pan law, and the effects are not in this capture.

### Exit stutter on macOS (2026-11-16): not ours

The owner heard about 2 s of stuttering when the game exits. We checked the game's own output in two ways:
- the mixer output, recorded up to `FSOUND_Close`;
- the device output, captured with SDL's disk driver.

Both are clean up to the close; closing the CoreAudio device then takes about 150 ms. The same stutter then happened when the owner paused a YouTube video, so it comes from the host's audio system. Two experimental workarounds (pausing the device before closing, and flushing it with silence) were reverted.
