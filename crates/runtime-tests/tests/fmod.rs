//! FMOD 3.61 contracts (silent implementation).

use bugatron_runtime::testing::call_import;
use bugatron_runtime::with_rt;
use bugatron_runtime_tests::*;

#[test]
fn loads_read_real_files_and_return_null_when_missing() {
    let mut cpu = setup();
    let base = std::env::temp_dir().join(format!("bugatron-fmod-{}", std::process::id()));
    std::fs::create_dir_all(base.join("Audio")).unwrap();
    std::fs::write(base.join("Audio").join("Shot.wav"), b"RIFF....WAVE").unwrap();
    std::fs::write(base.join("Audio").join("Title.xm"), b"Extended Module: ").unwrap();
    with_rt(|rt| rt.files.data_dir = base.clone());
    assert_eq!(call_import(&mut cpu, "fmod.dll", "_FSOUND_Init@12", &[44100, 32, 0]), 1);
    let name = put_str(0, "audio\\shot.WAV");
    let s = call_import(&mut cpu, "fmod.dll", "_FSOUND_Sample_Load@16", &[0xffff_ffff, name, 0, 0]);
    assert_ne!(s, 0);
    assert_ne!(call_import(&mut cpu, "fmod.dll", "_FSOUND_PlaySound@8", &[0xffff_ffff, s]), 0xffff_ffff);
    assert_eq!(call_import(&mut cpu, "fmod.dll", "_FSOUND_PlaySound@8", &[0xffff_ffff, 0x999]), 0xffff_ffff);
    let missing = put_str(0x40, "audio\\none.wav");
    assert_eq!(call_import(&mut cpu, "fmod.dll", "_FSOUND_Sample_Load@16", &[0xffff_ffff, missing, 0, 0]), 0);
    let song = put_str(0x80, "Audio\\Title.xm");
    let m = call_import(&mut cpu, "fmod.dll", "_FMUSIC_LoadSong@4", &[song]);
    assert_ne!(m, 0);
    assert_eq!(call_import(&mut cpu, "fmod.dll", "_FMUSIC_PlaySong@4", &[m]), 1);
    assert_eq!(call_import(&mut cpu, "fmod.dll", "_FMUSIC_SetPaused@8", &[m, 1]), 1);
    assert_eq!(call_import(&mut cpu, "fmod.dll", "_FMUSIC_FreeSong@4", &[m]), 1);
    assert_eq!(call_import(&mut cpu, "fmod.dll", "_FMUSIC_PlaySong@4", &[m]), 0);
    call_import(&mut cpu, "fmod.dll", "_FSOUND_Sample_Free@4", &[s]);
    call_import(&mut cpu, "fmod.dll", "_FSOUND_Close@0", &[]);
}
