//! SDL3 audio output: a stream at FMOD's mix rate whose callback pulls frames
//! from the shared mixer on SDL's audio thread. The game thread only takes the
//! mixer lock briefly to issue commands, so audio never blocks the simulation
//! and the simulation's timing never depends on the audio device.

use super::mixer::Mixer;
use sdl3_sys::audio::*;
use std::ffi::c_void;
use std::sync::{Arc, Mutex};

pub struct Device {
    stream: *mut SDL_AudioStream,
    shared: *const Mutex<Mixer>,
}

unsafe extern "C" fn pull(userdata: *mut c_void, stream: *mut SDL_AudioStream, additional: i32, _total: i32) {
    if additional <= 0 {
        return;
    }
    // SAFETY: userdata is the Arc<Mutex<Mixer>> kept alive by Device until the stream is destroyed.
    let mixer = unsafe { &*(userdata as *const Mutex<Mixer>) };
    let floats = (additional as usize / 4) & !1;
    let mut buf = vec![0f32; floats];
    let t0 = std::time::Instant::now();
    if let Ok(mut m) = mixer.lock() {
        let waited = t0.elapsed();
        m.render(&mut buf);
        if m.output_muted {
            buf.fill(0.0);
        }
        if crate::trace_enabled("audio") {
            trace_callback(floats / 2, m.rate, waited, t0.elapsed());
        }
    }
    // SAFETY: buf holds `floats` f32 values.
    unsafe { SDL_PutAudioStreamData(stream, buf.as_ptr() as *const c_void, (floats * 4) as i32) };
}

/// `BUGATRON_TRACE=audio`: callbacks that came late (a gap longer than the audio
/// the previous one supplied) or waited for the mixer lock.
fn trace_callback(frames: usize, rate: u32, waited: std::time::Duration, total: std::time::Duration) {
    use std::sync::Mutex as M;
    static LAST: M<Option<(std::time::Instant, f64)>> = M::new(None);
    let now = std::time::Instant::now();
    let mut last = LAST.lock().unwrap();
    if let Some((t, supplied)) = *last {
        let gap = now.duration_since(t).as_secs_f64();
        if gap > supplied * 1.5 + 0.005 || waited.as_micros() > 1000 {
            eprintln!("[audio] late callback: gap {:.1} ms after {:.1} ms of audio; lock wait {:.2} ms, mix {:.2} ms",
                gap * 1e3, supplied * 1e3, waited.as_secs_f64() * 1e3, total.as_secs_f64() * 1e3);
        }
    }
    *last = Some((now, frames as f64 / rate as f64));
}

impl Device {
    /// Open the default playback device at `rate` Hz, stereo f32. None if unavailable.
    pub fn open(mixer: Arc<Mutex<Mixer>>, rate: u32) -> Option<Device> {
        // SAFETY: SDL calls with valid arguments; the Arc is released in Drop after the stream.
        unsafe {
            use sdl3_sys::init::{SDL_Init, SDL_WasInit, SDL_INIT_AUDIO};
            if SDL_WasInit(SDL_INIT_AUDIO) == 0u32 && !SDL_Init(SDL_INIT_AUDIO) {
                eprintln!("[audio] SDL_Init(AUDIO): {}", std::ffi::CStr::from_ptr(sdl3_sys::error::SDL_GetError()).to_string_lossy());
                return None;
            }
            let spec = SDL_AudioSpec { format: SDL_AUDIO_F32, channels: 2, freq: rate as i32 };
            let shared = Arc::into_raw(mixer);
            let stream = SDL_OpenAudioDeviceStream(SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK, &spec, Some(pull), shared as *mut c_void);
            if stream.is_null() {
                eprintln!("[audio] SDL_OpenAudioDeviceStream: {}", std::ffi::CStr::from_ptr(sdl3_sys::error::SDL_GetError()).to_string_lossy());
                drop(Arc::from_raw(shared));
                return None;
            }
            SDL_ResumeAudioStreamDevice(stream);
            Some(Device { stream, shared })
        }
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        // SAFETY: destroying the stream stops the callback before the Arc is released.
        unsafe {
            SDL_DestroyAudioStream(self.stream);
            drop(Arc::from_raw(self.shared));
        }
    }
}
