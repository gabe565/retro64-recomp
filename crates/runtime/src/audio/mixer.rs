//! A software mixer with FMOD 3.61's behaviour for the calls the game makes.
//!
//! - Channels: `FSOUND_Init`'s software channel count (the game asks for 32).
//!   `FSOUND_PlaySound(FSOUND_FREE)` takes the lowest-numbered free channel;
//!   with none free it steals the oldest channel whose priority is not higher
//!   than the new sound's (FMOD's channel stealing; all samples keep the
//!   default priority 128), and returns -1 if none qualifies.
//! - Sample defaults (FMOD's `FSOUND_Sample_SetDefaults` defaults): frequency
//!   from the file, volume 255, pan 128 (centre), priority 128, loop off (or the
//!   file's `smpl` loop). `FSOUND_Sample_SetMode` changes the loop mode; FMOD
//!   applies it to channels already playing the sample.
//! - Levels: channel amplitude × volume/255 × SFX master/255, panned linearly
//!   (left (255−pan)/255, right pan/255, so centre is −6 dB per side, FMOD 3's
//!   software pan law). Music is added at its master volume/256. The sum is
//!   clipped to [−1, 1], as FMOD's 16-bit output clips.
//! - Resampling: linear interpolation (FMOD 3's interpolating software mixer).
//! - Freeing a sample stops the channels playing it; closing stops everything.
//!
//! The mixer is deterministic and has no I/O, so it is unit-tested directly;
//! the SDL device (device.rs) only pulls rendered frames from it.

use super::wav::Pcm;
use std::collections::HashMap;
use std::sync::Arc;

pub const FSOUND_LOOP_OFF: u32 = 0x1;
pub const FSOUND_LOOP_NORMAL: u32 = 0x2;
pub const FSOUND_LOOP_BIDI: u32 = 0x4;
const LOOP_MASK: u32 = FSOUND_LOOP_OFF | FSOUND_LOOP_NORMAL | FSOUND_LOOP_BIDI;

/// Interleaved 16-bit stereo music, e.g. libxmp.
pub trait MusicSource: Send {
    fn render(&mut self, out: &mut [i16]);
    fn restart(&mut self);
}

impl MusicSource for bugatron_xmp_sys::Player {
    fn render(&mut self, out: &mut [i16]) {
        bugatron_xmp_sys::Player::render(self, out)
    }
    fn restart(&mut self) {
        bugatron_xmp_sys::Player::restart(self)
    }
}

#[derive(Debug)]
pub struct Sample {
    pub pcm: Pcm,
    pub mode: u32,
    pub volume: u32,
    pub pan: u32,
    pub priority: u32,
}

impl Sample {
    pub fn new(pcm: Pcm) -> Sample {
        let mode = if pcm.loop_points.is_some() { FSOUND_LOOP_NORMAL } else { FSOUND_LOOP_OFF };
        Sample { pcm, mode, volume: 255, pan: 128, priority: 128 }
    }

    fn loop_range(&self) -> (f64, f64) {
        let (s, e) = self.pcm.loop_points.unwrap_or((0, self.pcm.len() as u32));
        (s as f64, (e.min(self.pcm.len() as u32)).max(s + 1) as f64)
    }
}

#[derive(Debug, Clone)]
pub struct Channel {
    pub sample: u32,
    /// Position in source frames.
    pub pos: f64,
    /// Playing backwards (bidirectional loop).
    pub reverse: bool,
    pub volume: u32,
    pub pan: u32,
    pub priority: u32,
    /// Start order, for stealing the oldest.
    pub started: u64,
}

pub struct Song {
    pub source: Box<dyn MusicSource>,
    pub playing: bool,
    pub paused: bool,
    /// FMUSIC master volume, 0..=256.
    pub volume: u32,
}

/// Rendered output captured for comparison with the original (`--record-audio`).
#[derive(Debug, Default)]
pub struct Recording {
    /// Interleaved stereo frames.
    pub samples: Vec<f32>,
    /// Length to record, in seconds of output at the mix rate.
    pub seconds: f64,
    /// Capture starts at the first FMUSIC_PlaySong, like tools/frida/capture_fmod_output.js.
    pub started: bool,
}

pub struct Mixer {
    pub rate: u32,
    pub channels: Vec<Option<Channel>>,
    pub samples: HashMap<u32, Arc<Sample>>,
    pub songs: HashMap<u32, Song>,
    /// FSOUND SFX master volume, 0..=255.
    pub sfx_volume: u32,
    started: u64,
    scratch: Vec<i16>,
    /// Recording of the rendered output (diagnostics).
    pub recording: Option<Recording>,
}

impl Mixer {
    pub fn new(rate: u32, channels: usize) -> Mixer {
        Mixer { rate, channels: vec![None; channels], samples: HashMap::new(), songs: HashMap::new(), sfx_volume: 255, started: 0, scratch: Vec::new(), recording: None }
    }

    /// `FSOUND_PlaySound(FSOUND_FREE, sample)`: the channel used, or None (-1).
    pub fn play(&mut self, handle: u32) -> Option<usize> {
        let s = self.samples.get(&handle)?.clone();
        let slot = match self.channels.iter().position(Option::is_none) {
            Some(i) => i,
            None => {
                let (i, _) = self
                    .channels
                    .iter()
                    .enumerate()
                    .filter_map(|(i, c)| c.as_ref().map(|c| (i, c)))
                    .filter(|(_, c)| c.priority <= s.priority)
                    .min_by_key(|(_, c)| c.started)?;
                i
            }
        };
        self.started += 1;
        self.channels[slot] = Some(Channel { sample: handle, pos: 0.0, reverse: false, volume: s.volume, pan: s.pan, priority: s.priority, started: self.started });
        Some(slot)
    }

    /// `FSOUND_Sample_SetMode`: the loop bits (other mode bits describe data formats and are kept).
    pub fn set_mode(&mut self, handle: u32, mode: u32) -> bool {
        let Some(s) = self.samples.get_mut(&handle) else { return false };
        let loop_bits = if mode & LOOP_MASK != 0 { mode & LOOP_MASK } else { s.mode & LOOP_MASK };
        let new = Sample { pcm: s.pcm.clone(), mode: (s.mode & !LOOP_MASK) | loop_bits, volume: s.volume, pan: s.pan, priority: s.priority };
        *s = Arc::new(new);
        true
    }

    /// `FSOUND_Sample_Free`: stops the channels playing it.
    pub fn free_sample(&mut self, handle: u32) -> bool {
        for c in self.channels.iter_mut() {
            if c.as_ref().is_some_and(|c| c.sample == handle) {
                *c = None;
            }
        }
        self.samples.remove(&handle).is_some()
    }

    pub fn playing_channels(&self) -> usize {
        self.channels.iter().filter(|c| c.is_some()).count()
    }

    /// `FSOUND_Close`.
    pub fn stop_all(&mut self) {
        self.channels.iter_mut().for_each(|c| *c = None);
        self.songs.values_mut().for_each(|s| s.playing = false);
    }

    /// Render interleaved stereo f32 frames (`out.len()` even), replacing its contents.
    pub fn render(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        let frames = out.len() / 2;
        let master = self.sfx_volume.min(255) as f32 / 255.0;
        for slot in self.channels.iter_mut() {
            let Some(ch) = slot.as_mut() else { continue };
            let Some(s) = self.samples.get(&ch.sample) else {
                *slot = None;
                continue;
            };
            let (pcm, nch) = (&s.pcm, s.pcm.channels as usize);
            let len = pcm.len() as f64;
            let step = pcm.rate as f64 / self.rate as f64;
            let vol = ch.volume.min(255) as f32 / 255.0 * master;
            let (lv, rv) = ((255 - ch.pan.min(255)) as f32 / 255.0 * vol, ch.pan.min(255) as f32 / 255.0 * vol);
            let (ls, le) = s.loop_range();
            let lm = s.mode & LOOP_MASK;
            let mut done = false;
            for f in 0..frames {
                let i = ch.pos.floor() as usize;
                let frac = (ch.pos - i as f64) as f32;
                let at = |k: usize, c: usize| -> f32 {
                    let k = if lm & (FSOUND_LOOP_NORMAL | FSOUND_LOOP_BIDI) != 0 && k as f64 >= le { ls as usize } else { k };
                    pcm.frames.get(k * nch + c.min(nch - 1)).copied().unwrap_or(0.0)
                };
                let (l, r) = if nch == 1 {
                    let v = at(i, 0) + (at(i + 1, 0) - at(i, 0)) * frac;
                    (v, v)
                } else {
                    (at(i, 0) + (at(i + 1, 0) - at(i, 0)) * frac, at(i, 1) + (at(i + 1, 1) - at(i, 1)) * frac)
                };
                out[2 * f] += l * lv;
                out[2 * f + 1] += r * rv;
                // Advance.
                if ch.reverse {
                    ch.pos -= step;
                    if ch.pos < ls {
                        ch.pos = ls + (ls - ch.pos);
                        ch.reverse = false;
                    }
                } else {
                    ch.pos += step;
                }
                if !ch.reverse && ch.pos >= le && lm & (FSOUND_LOOP_NORMAL | FSOUND_LOOP_BIDI) != 0 {
                    if lm & FSOUND_LOOP_BIDI != 0 {
                        ch.pos = le - (ch.pos - le) - 1.0;
                        ch.reverse = true;
                    } else {
                        ch.pos = ls + (ch.pos - le) % (le - ls);
                    }
                } else if ch.pos >= len {
                    done = true;
                    break;
                }
            }
            if done {
                *slot = None;
            }
        }
        for song in self.songs.values_mut() {
            if !song.playing || song.paused {
                continue;
            }
            self.scratch.resize(frames * 2, 0);
            song.source.render(&mut self.scratch);
            let g = song.volume.min(256) as f32 / 256.0 / 32768.0;
            for (o, &v) in out.iter_mut().zip(&self.scratch) {
                *o += v as f32 * g;
            }
        }
        for o in out.iter_mut() {
            *o = o.clamp(-1.0, 1.0);
        }
        if let Some(r) = self.recording.as_mut().filter(|r| r.started) {
            let limit = (r.seconds * self.rate as f64) as usize;
            let room = (limit * 2).saturating_sub(r.samples.len());
            r.samples.extend_from_slice(&out[..room.min(out.len())]);
        }
    }
}
