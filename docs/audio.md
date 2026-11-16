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
- **No device:** FMOD-visible behaviour is unchanged, but nothing is output. This applies in tests, and if `FSOUND_Init` cannot open a device it fails as FMOD would, so the game disables sound itself.

Verified by scripted play. Title, menu, gameplay, game over, title, and a new game load, play, fade, and free the right songs and effects. `BUGATRON_TRACE=fmod` logs every call except the per-frame `SetPaused`.

## Approximations and open checks

| # | Item | Check |
|---|---|---|
| AU1 | Music comes from libxmp, not FMOD 3.61's own XM player. Effect quirks, interpolation, volume ramping, and the overall music level may differ | Compare captures (below) |
| AU2 | The pan law (linear, −6 dB at centre) and the channel-stealing rule follow the FMOD 3 documentation. The game never sets pan, and 32 channels are rarely exhausted | Captures; stealing is only audible with more than 32 simultaneous sounds |
| AU3 | `FSOUND_PlaySound` returns the plain channel index. FMOD 3.6 may encode extra bits; the game never uses the value | — |
| AU4 | Output is resampled by SDL from 44100 Hz to the device rate | — |
| AU5 | Mixing uses floats, then clips. FMOD mixed in integers, so there may be ±1 LSB differences | Captures |

## Comparing with the original

- **Original:** `tools/frida/capture_fmod_output.js` adds a DSP unit to the original's FMOD (priority 1100, above the clip-and-copy unit). It receives the final 16-bit stereo mix and sends 12 s of it from the first `PlaySong` onwards. Load it with `capture_deterministic_frames.js` so both runs see the same game timeline.
- **Native:** `bugatron --frames N --record-audio native.wav 12` records the mixer output from the first song onwards.
- **Results:** see below.

### Results (2026-11-15)

- **The original is silent in the reference VM.** Probed with Frida, its `FSOUND_Init` fails at 44100 Hz, again at 22050 Hz, and again with 16 channels. The game then disables sound (0x438144 = 0). The VM has no working DirectSound output, most likely because no sound device is configured in UTM. So there was no mix to capture. The DSP capture script is ready for a VM with a sound device, or for any Windows machine running Frida.
- **Native side.** The title music records at −29.3 dBFS RMS (peak 6743/32767) at the game's saved music volume of 56/256.
- **What holds without a capture:**
  - Effects play at their files' own sample rates, so pitch and speed are the files'.
  - XM tempo follows the module (libxmp).
  - The music and effects lifecycle across scenes matches the original's call sequence, which comes from the same recompiled code.
  - Still unverified: AU1 (relative music level and XM playback details) and AU2 (pan law).
