//! FMOD 3.61 (`fmod.dll`), silent for now.
//!
//! Implements the 13 functions the game imports with FMOD's return contracts:
//! initialisation succeeds, samples and songs are loaded from the game's files
//! (NULL when the file is missing, as FMOD returns), and playback hands out
//! channel numbers. No audio is produced yet: playback is the audio milestone
//! (SDL3 audio + libxmp; docs/runtime-status.md RD8).

use crate::{fatal, Runtime};
use std::collections::HashMap;

#[derive(Default)]
pub struct Fmod {
    pub initialised: bool,
    /// Sample handle → (file name, mode, byte length).
    pub samples: HashMap<u32, (String, u32, usize)>,
    /// Song handle → (file name, byte length).
    pub songs: HashMap<u32, (String, usize)>,
    pub sfx_volume: u32,
    pub playing_song: Option<u32>,
    next_channel: u32,
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

/// Returns (value, argument bytes popped), or None if `name` is not an FMOD import.
pub(crate) fn call(rt: &mut Runtime, name: &str, arg: &dyn Fn(u32) -> u32) -> Option<(u32, u32)> {
    let r = match name {
        "_FSOUND_SetMaxHardwareChannels@4" => (1, 1),
        "_FSOUND_Init@12" => {
            rt.fmod.initialised = true;
            (1, 3) // TRUE
        }
        "_FSOUND_Close@0" => {
            rt.fmod.initialised = false;
            (0, 0)
        }
        "_FSOUND_SetSFXMasterVolume@4" => {
            rt.fmod.sfx_volume = arg(0);
            (0, 1)
        }
        "_FSOUND_Sample_Load@16" => {
            // (index, name_or_data, mode, memlength); FSOUND_LOADMEMORY (0x8000) is not used by the game.
            let (p, mode) = (arg(1), arg(2));
            if mode & 0x8000 != 0 {
                fatal("FSOUND_Sample_Load from memory is not emulated");
            }
            let file = rt.mem.cstr(p);
            match read_game_file(rt, &file) {
                Some(data) => {
                    let h = new_handle(rt);
                    rt.fmod.samples.insert(h, (file, mode, data.len()));
                    (h, 4)
                }
                None => (0, 4),
            }
        }
        "_FSOUND_Sample_Free@4" => {
            let h = arg(0);
            if rt.fmod.samples.remove(&h).is_some() {
                rt.heap.free(h);
            }
            (0, 1)
        }
        "_FSOUND_Sample_SetMode@8" => {
            let (h, mode) = (arg(0), arg(1));
            let ok = rt.fmod.samples.get_mut(&h).map(|s| s.1 = mode).is_some();
            (ok as u32, 2)
        }
        "_FSOUND_PlaySound@8" => {
            // (channel or FSOUND_FREE, sample) → channel index, or -1 on failure.
            let s = arg(1);
            if rt.fmod.samples.contains_key(&s) {
                rt.fmod.next_channel = (rt.fmod.next_channel + 1) % 32;
                (rt.fmod.next_channel, 2)
            } else {
                (0xffff_ffff, 2)
            }
        }
        "_FMUSIC_LoadSong@4" => {
            let file = rt.mem.cstr(arg(0));
            match read_game_file(rt, &file) {
                Some(data) => {
                    let h = new_handle(rt);
                    rt.fmod.songs.insert(h, (file, data.len()));
                    (h, 1)
                }
                None => (0, 1),
            }
        }
        "_FMUSIC_FreeSong@4" => {
            let h = arg(0);
            if rt.fmod.songs.remove(&h).is_some() {
                rt.heap.free(h);
            }
            if rt.fmod.playing_song == Some(h) {
                rt.fmod.playing_song = None;
            }
            (1, 1)
        }
        "_FMUSIC_PlaySong@4" => {
            let h = arg(0);
            let ok = rt.fmod.songs.contains_key(&h);
            if ok {
                rt.fmod.playing_song = Some(h);
            }
            (ok as u32, 1)
        }
        "_FMUSIC_SetPaused@8" => ((rt.fmod.songs.contains_key(&arg(0))) as u32, 2),
        "_FMUSIC_SetMasterVolume@8" => ((rt.fmod.songs.contains_key(&arg(0))) as u32, 2),
        _ => return None,
    };
    Some(r)
}
