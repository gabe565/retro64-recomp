//! FMOD 3.61 (`fmod.dll`): the 13 functions the game imports, on the software
//! mixer (audio/mixer.rs) and SDL3 audio (audio/device.rs).
//!
//! How the game uses FMOD (0x416b40–0x417320):
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
use crate::audio::wav;
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
}

impl Default for Fmod {
    fn default() -> Self {
        Fmod { initialised: false, output: true, mixer: Arc::new(Mutex::new(Mixer::new(44100, 32))), device: None, names: HashMap::new() }
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
const FSOUND_LOADMEMORY: u32 = 0x8000;

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
            (0, 0)
        }
        "_FSOUND_SetSFXMasterVolume@4" => {
            rt.fmod.mixer().sfx_volume = arg(0).min(255);
            (0, 1)
        }
        "_FSOUND_Sample_Load@16" => {
            let (p, mode) = (arg(1), arg(2));
            if mode & FSOUND_LOADMEMORY != 0 {
                fatal("FSOUND_Sample_Load from memory is not emulated (the game loads files)");
            }
            let file = rt.mem.cstr(p);
            let decoded = read_game_file(rt, &file).map(|d| wav::decode(&d));
            match decoded {
                Some(Ok(pcm)) => {
                    let h = new_handle(rt);
                    let mut s = Sample::new(pcm);
                    if mode & 0x7 != 0 {
                        s.mode = mode & 0x7;
                    }
                    rt.fmod.mixer().samples.insert(h, Arc::new(s));
                    rt.fmod.names.insert(h, file);
                    (h, 4)
                }
                Some(Err(e)) => {
                    eprintln!("[fmod] {file}: {e}");
                    (0, 4)
                }
                None => (0, 4),
            }
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
        "_FSOUND_PlaySound@8" => {
            let (channel, s) = (arg(0), arg(1));
            if channel != FSOUND_FREE {
                fatal("FSOUND_PlaySound on a specific channel is not emulated (the game uses FSOUND_FREE)");
            }
            let r = rt.fmod.mixer().play(s);
            (r.map_or(0xffff_ffff, |c| c as u32), 2)
        }
        "_FMUSIC_LoadSong@4" => {
            let file = rt.mem.cstr(arg(0));
            let rate = rt.fmod.mixer().rate;
            let player = read_game_file(rt, &file).map(|d| bugatron_xmp_sys::Player::load(&d, rate));
            match player {
                Some(Ok(p)) => {
                    let h = new_handle(rt);
                    rt.fmod.mixer().songs.insert(h, Song { source: Box::new(p), playing: false, paused: false, volume: 256 });
                    rt.fmod.names.insert(h, file);
                    (h, 1)
                }
                Some(Err(e)) => {
                    eprintln!("[fmod] {file}: {e}");
                    (0, 1)
                }
                None => (0, 1),
            }
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
