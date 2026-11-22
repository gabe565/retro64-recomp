//! FMOD 3.x (`fmod.dll`): the functions the games import, on the software mixer
//! (audio/mixer.rs) and SDL3 audio (audio/device.rs). Bugatron uses FMOD 3.61;
//! Z-Ball an older build with a few older or extra calls (`Sample_SetLoopMode`,
//! `StopSound`, `GetCurrentSample`, `FMUSIC_StopAllSongs`); Bugatron Worlds a
//! newer one that loads sounds (Ogg Vorbis) and songs from memory
//! (`Sample_Load@20`, `FMUSIC_LoadSongEx`) and sets channel volumes.
//!
//! How Bugatron uses FMOD (0x416b40–0x417320):
//! - `FSOUND_SetMaxHardwareChannels(0)` (software mixing only), then
//!   `FSOUND_Init(44100, 32, 0)`, falling back to 22050 and then 16 channels;
//!   if all fail, sound is disabled (0x438144 = 0);
//! - effects: `FSOUND_Sample_Load(FSOUND_UNMANAGED, file, 0, 0)` (format from
//!   the file), some switched to `FSOUND_LOOP_NORMAL`, played with
//!   `FSOUND_PlaySound(FSOUND_FREE, s)`. There is no per-channel stop, volume,
//!   pan or frequency call: channels end on their own, when their sample is
//!   freed, or at `FSOUND_Close`;
//! - `FSOUND_SetSFXMasterVolume(0..255)` (Home/End keys);
//! - music: `FMUSIC_LoadSong` (XM), `PlaySong`, `SetPaused`, `SetMasterVolume`
//!   (0..256, PgUp/PgDn), `FreeSong`. Songs loop (FMOD's default).
//!
//! Without an audio device (tests, headless runs) everything behaves the same
//! but nothing is output.

use crate::audio::device::Device;
use crate::audio::mixer::{Mixer, Sample, Song};
use crate::{fatal, Runtime};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub struct Fmod {
    pub initialised: bool,
    /// Open an audio device at FSOUND_Init (off for tests and headless runs).
    pub output: bool,
    pub mixer: Arc<Mutex<Mixer>>,
    device: Option<Device>,
    /// Sample and song handles → file names (diagnostics).
    pub names: HashMap<u32, String>,
    /// Without an audio device (`output` off): the virtual time of FSOUND_Init and
    /// the frames mixed since, so a recording still gets the mix (`mix_unheard`).
    unheard: Option<(u128, u64)>,
}

impl Default for Fmod {
    fn default() -> Self {
        Fmod { initialised: false, output: true, mixer: Arc::new(Mutex::new(Mixer::new(44100, 32))), device: None, names: HashMap::new(), unheard: None }
    }
}

impl Fmod {
    pub fn mixer(&self) -> std::sync::MutexGuard<'_, Mixer> {
        self.mixer.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether audio is actually being output.
    pub fn has_device(&self) -> bool {
        self.device.is_some()
    }
}

/// Handles are guest heap blocks so that they are unique, non-null pointers.
fn new_handle(rt: &mut Runtime) -> u32 {
    rt.heap_alloc(16, true)
}

/// Read a whole game file through the guest file layer (respecting the current directory).
fn read_game_file(rt: &mut Runtime, name: &str) -> Option<Vec<u8>> {
    let h = rt.files.create(name, 0x8000_0000, 3).ok()?;
    let size = rt.files.size(h)? as usize;
    let mut buf = vec![0u8; size];
    let n = rt.files.read(h, &mut buf);
    rt.files.close(h);
    (n == Some(size)).then_some(buf)
}

const FSOUND_FREE: u32 = 0xffff_ffff;
/// Every channel, for `FSOUND_StopSound`.
const FSOUND_ALL: u32 = 0xffff_fffd;
const FSOUND_LOADMEMORY: u32 = 0x8000;

/// A sample or song's bytes: a file, or with FSOUND_LOADMEMORY guest memory
/// (`length` bytes at `name_or_data`). `offset` skips into either.
fn sample_bytes(rt: &mut Runtime, name_or_data: u32, mode: u32, offset: u32, length: u32) -> Option<(Vec<u8>, String)> {
    if mode & FSOUND_LOADMEMORY != 0 {
        let d = rt.mem.slice(name_or_data.wrapping_add(offset), length as usize).to_vec();
        Some((d, format!("memory {name_or_data:#x}+{length}")))
    } else {
        let file = rt.mem.cstr(name_or_data);
        let d = read_game_file(rt, &file)?;
        let start = (offset as usize).min(d.len());
        let end = if length == 0 { d.len() } else { (start + length as usize).min(d.len()) };
        Some((d[start..end].to_vec(), file))
    }
}

/// Load a sample (FSOUND_Sample_Load); its handle, or 0.
fn load_sample(rt: &mut Runtime, name_or_data: u32, mode: u32, offset: u32, length: u32) -> u32 {
    let Some((data, name)) = sample_bytes(rt, name_or_data, mode, offset, length) else { return 0 };
    match crate::audio::ogg::decode_sample(&data) {
        Ok(pcm) => {
            let h = new_handle(rt);
            let mut s = Sample::new(pcm);
            if mode & 0x7 != 0 {
                s.mode = mode & 0x7;
            }
            rt.fmod.mixer().samples.insert(h, Arc::new(s));
            rt.fmod.names.insert(h, name);
            h
        }
        Err(e) => {
            eprintln!("[fmod] {name}: {e}");
            0
        }
    }
}

/// Load a song (FMUSIC_LoadSong/LoadSongEx); its handle, or 0.
fn load_song(rt: &mut Runtime, name_or_data: u32, mode: u32, offset: u32, length: u32) -> u32 {
    let Some((data, name)) = sample_bytes(rt, name_or_data, mode, offset, length) else { return 0 };
    let rate = rt.fmod.mixer().rate;
    match bugatron_xmp_sys::Player::load(&data, rate) {
        Ok(p) => {
            let h = new_handle(rt);
            rt.fmod.mixer().songs.insert(h, Song { source: Box::new(p), playing: false, paused: false, volume: 256 });
            rt.fmod.names.insert(h, name);
            h
        }
        Err(e) => {
            eprintln!("[fmod] {name}: {e}");
            0
        }
    }
}

/// Returns (value, argument dwords popped), or None if `name` is not an FMOD import.
pub(crate) fn call(rt: &mut Runtime, name: &str, arg: &dyn Fn(u32) -> u32) -> Option<(u32, u32)> {
    let r = match name {
        "_FSOUND_SetMaxHardwareChannels@4" => (1, 1),
        "_FSOUND_Init@12" => {
            let (rate, channels) = (arg(0), arg(1));
            if rt.fmod.initialised {
                (0, 3) // FMOD refuses a second Init without Close
            } else {
                {
                    let mut m = rt.fmod.mixer();
                    let recording = m.recording.take();
                    *m = Mixer::new(rate, channels.clamp(1, 256) as usize);
                    m.recording = recording;
                }
                if rt.fmod.output {
                    rt.fmod.device = Device::open(rt.fmod.mixer.clone(), rate);
                    if rt.fmod.device.is_none() {
                        // FMOD reports an output failure; the game retries, then disables sound.
                        return Some((0, 3));
                    }
                }
                rt.fmod.initialised = true;
                let now = rt.clock.cadence.as_ref().map(|c| c.now_ns());
                rt.fmod.unheard = now.filter(|_| rt.fmod.device.is_none()).map(|t| (t, 0));
                (1, 3)
            }
        }
        "_FSOUND_Close@0" => {
            rt.fmod.device = None;
            // A recording still in progress is saved as it stands (the game is exiting).
            if let Some(p) = rt.audio_record_path.take() {
                match write_recording(rt, &p) {
                    Ok(n) => eprintln!("[audio] recorded {n} frames to {}", p.display()),
                    Err(e) => eprintln!("[audio] {}: {e}", p.display()),
                }
            }
            rt.fmod.mixer().stop_all();
            rt.fmod.initialised = false;
            rt.fmod.unheard = None;
            (0, 0)
        }
        "_FSOUND_SetSFXMasterVolume@4" => {
            rt.fmod.mixer().sfx_volume = arg(0).min(255);
            (0, 1)
        }
        // FMOD 3.6: (index, name_or_data, mode, memlength).
        "_FSOUND_Sample_Load@16" => (load_sample(rt, arg(1), arg(2), 0, arg(3)), 4),
        // FMOD 3.7: (index, name_or_data, mode, offset, length).
        "_FSOUND_Sample_Load@20" => (load_sample(rt, arg(1), arg(2), arg(3), arg(4)), 5),
        // (pool, poollen, alloc, realloc, free): FMOD's memory is the host's here.
        "_FSOUND_SetMemorySystem@20" => (1, 5),
        // (channel, volume 0..255): FSOUND_ALL sets every channel.
        "_FSOUND_SetVolume@8" => {
            let mut m = rt.fmod.mixer();
            let (c, v) = (arg(0), arg(1).min(255));
            let ok = if c == FSOUND_ALL {
                m.channels.iter_mut().flatten().for_each(|c| c.volume = v);
                true
            } else {
                m.channels.get_mut(c as usize).and_then(|c| c.as_mut()).map(|c| c.volume = v).is_some()
            };
            (ok as u32, 2)
        }
        "_FSOUND_Sample_Free@4" => {
            let h = arg(0);
            if rt.fmod.mixer().free_sample(h) {
                rt.fmod.names.remove(&h);
                rt.heap.free(h);
            }
            (0, 1)
        }
        "_FSOUND_Sample_SetMode@8" => {
            let ok = rt.fmod.mixer().set_mode(arg(0), arg(1));
            (ok as u32, 2)
        }
        // FMOD before 3.5's name for the loop bits of Sample_SetMode (same values).
        "_FSOUND_Sample_SetLoopMode@8" => {
            let ok = rt.fmod.mixer().set_mode(arg(0), arg(1) & crate::audio::mixer::LOOP_MASK);
            (ok as u32, 2)
        }
        // (channel): stop it; FSOUND_ALL stops every channel. False for a channel
        // index out of range.
        "_FSOUND_StopSound@4" => {
            let mut m = rt.fmod.mixer();
            let c = arg(0);
            let ok = if c == FSOUND_ALL {
                m.channels.iter_mut().for_each(|c| *c = None);
                true
            } else {
                m.channels.get_mut(c as usize).map(|c| *c = None).is_some()
            };
            (ok as u32, 1)
        }
        // (channel): the sample playing on it, or 0 (none, or it has ended).
        "_FSOUND_GetCurrentSample@4" => {
            let m = rt.fmod.mixer();
            let s = m.channels.get(arg(0) as usize).and_then(|c| c.as_ref()).map_or(0, |c| c.sample);
            (s, 1)
        }
        "_FSOUND_PlaySound@8" => {
            let (channel, s) = (arg(0), arg(1));
            if channel != FSOUND_FREE {
                fatal("FSOUND_PlaySound on a specific channel is not emulated (the game uses FSOUND_FREE)");
            }
            let r = rt.fmod.mixer().play(s);
            (r.map_or(0xffff_ffff, |c| c as u32), 2)
        }
        "_FMUSIC_LoadSong@4" => (load_song(rt, arg(0), 0, 0, 0), 1),
        // (name_or_data, offset, length, mode, samplelist, samplelistnum).
        "_FMUSIC_LoadSongEx@24" => (load_song(rt, arg(0), arg(3), arg(1), arg(2)), 6),
        "_FMUSIC_StopSong@4" => {
            let mut m = rt.fmod.mixer();
            let ok = m.songs.get_mut(&arg(0)).map(|s| s.playing = false).is_some();
            (ok as u32, 1)
        }
        "_FMUSIC_FreeSong@4" => {
            let h = arg(0);
            let ok = rt.fmod.mixer().songs.remove(&h).is_some();
            if ok {
                rt.fmod.names.remove(&h);
                rt.heap.free(h);
            }
            (ok as u32, 1)
        }
        "_FMUSIC_PlaySong@4" => {
            // Starts from the beginning (also when already playing).
            let mut m = rt.fmod.mixer();
            if let Some(r) = m.recording.as_mut() {
                r.started = true;
            }
            let ok = match m.songs.get_mut(&arg(0)) {
                Some(s) => {
                    s.source.restart();
                    s.playing = true;
                    s.paused = false;
                    true
                }
                None => false,
            };
            (ok as u32, 1)
        }
        "_FMUSIC_StopAllSongs@0" => {
            rt.fmod.mixer().songs.values_mut().for_each(|s| s.playing = false);
            (0, 0)
        }
        "_FMUSIC_SetPaused@8" => {
            let mut m = rt.fmod.mixer();
            let ok = m.songs.get_mut(&arg(0)).map(|s| s.paused = arg(1) != 0).is_some();
            (ok as u32, 2)
        }
        "_FMUSIC_SetMasterVolume@8" => {
            let mut m = rt.fmod.mixer();
            let ok = m.songs.get_mut(&arg(0)).map(|s| s.volume = arg(1).min(256)).is_some();
            (ok as u32, 2)
        }
        _ => return None,
    };
    if name != "_FMUSIC_SetPaused@8" && crate::trace_enabled("fmod") {
        let a: Vec<String> = (0..r.1).map(|i| format!("{:#x}", arg(i))).collect();
        let what = rt.fmod.names.get(&arg(0)).or_else(|| rt.fmod.names.get(&arg(1))).or_else(|| rt.fmod.names.get(&r.0)).cloned().unwrap_or_default();
        eprintln!("[fmod] flip {} {name}({}) = {:#x} {what}", rt.com.flips, a.join(", "), r.0);
    }
    Some(r)
}

/// Without an audio device, mix what is due by the virtual clock, unheard, while a
/// recording is in progress (`--mute` with `--record-audio`); after every vertical
/// blank. The device's callback does this otherwise.
pub(crate) fn mix_unheard(rt: &mut Runtime) {
    let Some((start, mixed)) = rt.fmod.unheard else { return };
    let Some(now) = rt.clock.cadence.as_ref().map(|c| c.now_ns()) else { return };
    let mut m = rt.fmod.mixer();
    if m.recording.is_none() {
        return;
    }
    let due = (now.saturating_sub(start) * m.rate as u128 / 1_000_000_000) as u64;
    let n = due.saturating_sub(mixed);
    if n > 0 {
        let mut buf = vec![0f32; n as usize * 2];
        m.render(&mut buf);
    }
    drop(m);
    rt.fmod.unheard = Some((start, mixed + n));
}

/// Write the recorded output as a 16-bit stereo WAV.
pub fn write_recording(rt: &Runtime, path: &std::path::Path) -> std::io::Result<usize> {
    let m = rt.fmod.mixer();
    let Some(r) = m.recording.as_ref() else { return Ok(0) };
    let pcm: Vec<u8> = r.samples.iter().flat_map(|v| ((v.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes()).collect();
    let mut w = b"RIFF".to_vec();
    w.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    for v in [16u32, 0x0002_0001, m.rate, m.rate * 4, 0x0010_0004] {
        w.extend_from_slice(&v.to_le_bytes());
    }
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    w.extend_from_slice(&pcm);
    std::fs::write(path, w)?;
    Ok(r.samples.len() / 2)
}
