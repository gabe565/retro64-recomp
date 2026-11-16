//! FMOD 3.61 import contracts, driven through the import dispatch path.

use bugatron_runtime::testing::call_import;
use bugatron_runtime::with_rt;
use bugatron_runtime_tests::*;


/// A minimal valid XM module: 2 channels, one empty one-row pattern, no instruments.
fn tiny_xm() -> Vec<u8> {
    let mut x = b"Extended Module: ".to_vec();
    x.extend_from_slice(&[b' '; 20]);
    x.push(0x1a);
    x.extend_from_slice(&[b' '; 20]);
    x.extend_from_slice(&0x0104u16.to_le_bytes());
    x.extend_from_slice(&276u32.to_le_bytes());
    for v in [1u16, 0, 2, 1, 0, 1, 6, 125] {
        x.extend_from_slice(&v.to_le_bytes()); // length, restart, channels, patterns, instruments, flags, tempo, bpm
    }
    x.extend_from_slice(&[0u8; 256]); // order table
    x.extend_from_slice(&9u32.to_le_bytes()); // pattern header length
    x.push(0); // packing type
    x.extend_from_slice(&1u16.to_le_bytes()); // rows
    x.extend_from_slice(&0u16.to_le_bytes()); // packed size: empty
    x
}

fn game_dir(tag: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!("bugatron-fmod-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(base.join("Audio")).unwrap();
    std::fs::write(base.join("Audio").join("Shot.wav"), wav(1, 22050, 8, &[200; 2205], None)).unwrap();
    std::fs::write(base.join("Audio").join("Hum.wav"), wav(1, 22050, 16, &[0x10; 400], None)).unwrap();
    std::fs::write(base.join("Audio").join("Title.xm"), tiny_xm()).unwrap();
    std::fs::write(base.join("Audio").join("Broken.xm"), b"Extended Module: ").unwrap();
    with_rt(|rt| rt.files.data_dir = base.clone());
    base
}

#[test]
fn init_load_play_free_close_contracts() {
    let mut cpu = setup();
    game_dir("contracts");
    assert_eq!(call_import(&mut cpu, "fmod.dll", "_FSOUND_SetMaxHardwareChannels@4", &[0]), 1);
    assert_eq!(call_import(&mut cpu, "fmod.dll", "_FSOUND_Init@12", &[44100, 32, 0]), 1);
    assert_eq!(call_import(&mut cpu, "fmod.dll", "_FSOUND_Init@12", &[44100, 32, 0]), 0, "no second Init without Close");
    assert_eq!(with_rt(|rt| {
        let m = rt.fmod.mixer();
        (m.rate, m.channels.len())
    }), (44100, 32));
    let name = put_str(0, "audio\\shot.WAV");
    let s = call_import(&mut cpu, "fmod.dll", "_FSOUND_Sample_Load@16", &[0xffff_fffe, name, 0, 0]);
    assert_ne!(s, 0);
    let missing = put_str(0x40, "audio\\none.wav");
    assert_eq!(call_import(&mut cpu, "fmod.dll", "_FSOUND_Sample_Load@16", &[0xffff_fffe, missing, 0, 0]), 0);
    // FSOUND_FREE takes the lowest free channels; an unknown sample gives -1.
    assert_eq!(call_import(&mut cpu, "fmod.dll", "_FSOUND_PlaySound@8", &[0xffff_ffff, s]), 0);
    assert_eq!(call_import(&mut cpu, "fmod.dll", "_FSOUND_PlaySound@8", &[0xffff_ffff, s]), 1);
    assert_eq!(call_import(&mut cpu, "fmod.dll", "_FSOUND_PlaySound@8", &[0xffff_ffff, 0x999]), 0xffff_ffff);
    // SetSFXMasterVolume clamps to 255.
    call_import(&mut cpu, "fmod.dll", "_FSOUND_SetSFXMasterVolume@4", &[300]);
    assert_eq!(with_rt(|rt| rt.fmod.mixer().sfx_volume), 255);
    // Looping mode, then freeing the sample stops its channels.
    let hum = put_str(0x80, "Audio\\Hum.wav");
    let h = call_import(&mut cpu, "fmod.dll", "_FSOUND_Sample_Load@16", &[0xffff_fffe, hum, 0, 0]);
    assert_eq!(call_import(&mut cpu, "fmod.dll", "_FSOUND_Sample_SetMode@8", &[h, 2]), 1);
    assert_eq!(call_import(&mut cpu, "fmod.dll", "_FSOUND_PlaySound@8", &[0xffff_ffff, h]), 2);
    let mut buf = vec![0f32; 2 * 44100];
    with_rt(|rt| rt.fmod.mixer().render(&mut buf));
    assert_eq!(with_rt(|rt| rt.fmod.mixer().playing_channels()), 1, "the one-shots ended; the loop plays on");
    call_import(&mut cpu, "fmod.dll", "_FSOUND_Sample_Free@4", &[h]);
    assert_eq!(with_rt(|rt| rt.fmod.mixer().playing_channels()), 0);
    call_import(&mut cpu, "fmod.dll", "_FSOUND_Close@0", &[]);
    assert_eq!(call_import(&mut cpu, "fmod.dll", "_FSOUND_Init@12", &[22050, 16, 0]), 1, "Init again after Close");
}

#[test]
fn music_lifecycle() {
    let mut cpu = setup();
    game_dir("music");
    call_import(&mut cpu, "fmod.dll", "_FSOUND_Init@12", &[44100, 32, 0]);
    let name = put_str(0, "Audio\\Title.xm");
    let m = call_import(&mut cpu, "fmod.dll", "_FMUSIC_LoadSong@4", &[name]);
    assert_ne!(m, 0, "a valid XM loads through libxmp");
    let broken = put_str(0x40, "Audio\\Broken.xm");
    assert_eq!(call_import(&mut cpu, "fmod.dll", "_FMUSIC_LoadSong@4", &[broken]), 0, "invalid modules fail as in FMOD");
    let playing = |m| with_rt(|rt| rt.fmod.mixer().songs.get(&m).map(|s| (s.playing, s.paused, s.volume)));
    assert_eq!(playing(m), Some((false, false, 256)), "loaded songs do not start by themselves");
    assert_eq!(call_import(&mut cpu, "fmod.dll", "_FMUSIC_PlaySong@4", &[m]), 1);
    assert_eq!(call_import(&mut cpu, "fmod.dll", "_FMUSIC_SetPaused@8", &[m, 1]), 1);
    assert_eq!(call_import(&mut cpu, "fmod.dll", "_FMUSIC_SetMasterVolume@8", &[m, 999]), 1);
    assert_eq!(playing(m), Some((true, true, 256)), "volume clamps to 256");
    assert_eq!(call_import(&mut cpu, "fmod.dll", "_FMUSIC_PlaySong@4", &[m]), 1);
    assert_eq!(playing(m), Some((true, false, 256)), "PlaySong restarts and unpauses");
    let mut buf = vec![0f32; 4096];
    with_rt(|rt| rt.fmod.mixer().render(&mut buf));
    assert_eq!(call_import(&mut cpu, "fmod.dll", "_FMUSIC_FreeSong@4", &[m]), 1);
    assert_eq!(playing(m), None);
    assert_eq!(call_import(&mut cpu, "fmod.dll", "_FMUSIC_PlaySong@4", &[m]), 0);
    assert_eq!(call_import(&mut cpu, "fmod.dll", "_FMUSIC_SetPaused@8", &[m, 0]), 0);
}

/// The game's own title music renders through libxmp (skipped without the game files).
#[test]
fn game_xm_renders() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_work/ext/gold120/app/Audio/KY_SFA.XM");
    let Ok(data) = std::fs::read(&p) else {
        eprintln!("game files not present: skipping");
        return;
    };
    let mut player = bugatron_xmp_sys::Player::load(&data, 44100).expect("load KY_SFA.XM");
    let mut buf = vec![0i16; 2 * 44100 * 2];
    player.render(&mut buf);
    let peak = buf.iter().map(|v| v.unsigned_abs()).max().unwrap();
    assert!(peak > 1000, "audible output (peak {peak})");
}
