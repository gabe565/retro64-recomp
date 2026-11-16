//! FMOD 3.61 behaviour on the software mixer: decoding, channels, levels,
//! looping, music and cleanup. No audio device is opened.

use bugatron_runtime::audio::mixer::{Mixer, MusicSource, Sample, Song, FSOUND_LOOP_NORMAL, FSOUND_LOOP_OFF};
use bugatron_runtime::audio::wav::{decode, Pcm};
use std::sync::atomic::{AtomicU32, Ordering};
use bugatron_runtime_tests::wav;
use std::sync::Arc;

/// A constant 8-bit mono sample (`n` frames of value `v`, 128 = silence).
fn dc8(n: usize, v: u8, rate: u32) -> Pcm {
    decode(&wav(1, rate, 8, &vec![v; n], None)).unwrap()
}

fn mixer_with(samples: &[(u32, Pcm)], channels: usize) -> Mixer {
    let mut m = Mixer::new(44100, channels);
    for (h, p) in samples {
        m.samples.insert(*h, Arc::new(Sample::new(p.clone())));
    }
    m
}

fn render(m: &mut Mixer, frames: usize) -> Vec<f32> {
    let mut out = vec![0f32; frames * 2];
    m.render(&mut out);
    out
}

#[test]
fn wav_decoding() {
    let p = decode(&wav(1, 22050, 8, &[0, 128, 255], None)).unwrap();
    assert_eq!((p.channels, p.rate, p.len()), (1, 22050, 3));
    assert_eq!(p.frames, [-1.0, 0.0, 127.0 / 128.0]);
    let p = decode(&wav(2, 11025, 16, &[0x00, 0x80, 0xff, 0x7f], None)).unwrap();
    assert_eq!((p.channels, p.len()), (2, 1));
    assert_eq!(p.frames, [-1.0, 32767.0 / 32768.0]);
    let p = decode(&wav(1, 44100, 8, &[128; 10], Some((2, 7)))).unwrap();
    assert_eq!(p.loop_points, Some((2, 8)), "smpl end is inclusive");
    assert_eq!(Sample::new(p).mode, FSOUND_LOOP_NORMAL, "a smpl loop makes the default mode loop");
    assert!(decode(b"RIFF\0\0\0\0WAVEjunk").is_err());
    assert!(decode(&wav(1, 22050, 24, &[0; 6], None)).is_err());
}

#[test]
fn free_channel_allocation_and_stealing_the_oldest() {
    let mut m = mixer_with(&[(1, dc8(100_000, 200, 44100))], 3);
    assert_eq!(m.play(1), Some(0), "lowest free channel");
    assert_eq!(m.play(1), Some(1));
    assert_eq!(m.play(1), Some(2));
    assert_eq!(m.play(1), Some(0), "all busy: the oldest equal-priority channel is stolen");
    assert_eq!(m.play(1), Some(1));
    assert_eq!(m.play(99), None, "unknown sample → -1");
    // A higher-priority channel is not stolen by a lower-priority sound.
    let mut s = Sample::new(dc8(100_000, 200, 44100));
    s.priority = 10;
    m.samples.insert(2, Arc::new(s));
    let mut m2 = mixer_with(&[(1, dc8(100_000, 200, 44100))], 1);
    m2.samples.insert(2, m.samples[&2].clone());
    m2.play(1);
    assert_eq!(m2.play(2), None);
}

#[test]
fn one_shot_ends_after_its_resampled_length_and_frees_the_channel() {
    // 1000 frames at 22050 Hz last 2000 frames at the 44100 Hz mix rate.
    let mut m = mixer_with(&[(1, dc8(1000, 200, 22050))], 4);
    m.play(1);
    render(&mut m, 1990);
    assert_eq!(m.playing_channels(), 1);
    render(&mut m, 20);
    assert_eq!(m.playing_channels(), 0);
    assert_eq!(m.play(1), Some(0), "the channel is free again");
}

#[test]
fn looping_continues_and_set_mode_applies_to_playing_channels() {
    let mut m = mixer_with(&[(1, dc8(100, 200, 44100))], 4);
    assert!(m.set_mode(1, FSOUND_LOOP_NORMAL));
    m.play(1);
    let out = render(&mut m, 10_000);
    assert_eq!(m.playing_channels(), 1, "a looping sample keeps playing");
    assert!(out[2 * 9_999] > 0.0, "and keeps producing sound");
    m.set_mode(1, FSOUND_LOOP_OFF);
    render(&mut m, 200);
    assert_eq!(m.playing_channels(), 0, "loop off: it ends at the sample's end");
    assert!(!m.set_mode(7, FSOUND_LOOP_NORMAL));
}

#[test]
fn levels_follow_volume_pan_and_sfx_master() {
    // 8-bit value 192 = +0.5.
    let mut m = mixer_with(&[(1, dc8(1000, 192, 44100))], 4);
    m.play(1);
    let out = render(&mut m, 10);
    let (l, r) = (out[10], out[11]);
    assert!((l - 0.5 * 127.0 / 255.0).abs() < 1e-6, "centre pan 128: left (255-128)/255, got {l}");
    assert!((r - 0.5 * 128.0 / 255.0).abs() < 1e-6, "right 128/255, got {r}");
    m.sfx_volume = 51;
    let out = render(&mut m, 10);
    assert!((out[10] - 0.5 * 127.0 / 255.0 * 0.2).abs() < 1e-6, "SFX master volume scales linearly");
    // Two overlapping channels add, and the sum clips like FMOD's 16-bit output.
    let mut m = mixer_with(&[(1, dc8(1000, 255, 44100))], 8);
    for _ in 0..5 {
        m.play(1);
    }
    let out = render(&mut m, 4);
    assert_eq!(out[4], 1.0);
}

#[test]
fn linear_interpolation_when_resampling() {
    // 0 → +1 ramp of two frames at half the mix rate: the midpoint is interpolated.
    let p = decode(&wav(1, 22050, 16, &[0, 0, 0xff, 0x7f, 0xff, 0x7f], None)).unwrap();
    let mut m = mixer_with(&[(1, p)], 1);
    let mut s = Sample::new(m.samples[&1].pcm.clone());
    s.pan = 255; // full right, unit gain
    m.samples.insert(1, Arc::new(s));
    m.play(1);
    let out = render(&mut m, 3);
    let right: Vec<f32> = out.chunks(2).map(|f| f[1]).collect();
    assert_eq!(right[0], 0.0);
    assert!((right[1] - 0.5 * 32767.0 / 32768.0).abs() < 1e-6, "{right:?}");
}

#[test]
fn freeing_a_sample_stops_its_channels_and_close_stops_all() {
    let mut m = mixer_with(&[(1, dc8(100_000, 200, 44100)), (2, dc8(100_000, 200, 44100))], 4);
    m.play(1);
    m.play(2);
    m.play(1);
    assert!(m.free_sample(1));
    assert_eq!(m.playing_channels(), 1);
    assert!(!m.free_sample(1));
    m.stop_all();
    assert_eq!(m.playing_channels(), 0);
}

/// Music source producing a constant level and counting restarts.
struct Constant(i16, Arc<AtomicU32>);
impl MusicSource for Constant {
    fn render(&mut self, out: &mut [i16]) {
        out.fill(self.0);
    }
    fn restart(&mut self) {
        self.1.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn music_volume_pause_and_mixing_with_effects() {
    let restarts = Arc::new(AtomicU32::new(0));
    let mut m = Mixer::new(44100, 4);
    m.songs.insert(9, Song { source: Box::new(Constant(16384, restarts.clone())), playing: true, paused: false, volume: 256 });
    assert_eq!(render(&mut m, 2)[0], 0.5);
    m.songs.get_mut(&9).unwrap().volume = 128;
    assert_eq!(render(&mut m, 2)[0], 0.25, "FMUSIC master volume /256");
    m.songs.get_mut(&9).unwrap().paused = true;
    assert_eq!(render(&mut m, 2)[0], 0.0);
    m.songs.get_mut(&9).unwrap().paused = false;
    m.sfx_volume = 0;
    assert_eq!(render(&mut m, 2)[0], 0.25, "the SFX master volume does not affect music");
    m.stop_all();
    assert_eq!(render(&mut m, 2)[0], 0.0);
}
