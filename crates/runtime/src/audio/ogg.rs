//! Ogg Vorbis decoding for FMOD's `FSOUND_Sample_Load` (Bugatron Worlds' sound
//! effects), with the pure-Rust lewton decoder. Ogg files carry no loop points;
//! a sample set to loop loops whole.

use super::wav::Pcm;
use lewton::inside_ogg::OggStreamReader;

pub fn is_ogg(d: &[u8]) -> bool {
    d.starts_with(b"OggS")
}

pub fn decode(d: &[u8]) -> Result<Pcm, String> {
    let mut r = OggStreamReader::new(std::io::Cursor::new(d)).map_err(|e| format!("Ogg Vorbis: {e}"))?;
    let (channels, rate) = (r.ident_hdr.audio_channels as u16, r.ident_hdr.audio_sample_rate);
    if channels == 0 {
        return Err("Ogg Vorbis: no channels".into());
    }
    let mut frames = Vec::new();
    while let Some(packet) = r.read_dec_packet_itl().map_err(|e| format!("Ogg Vorbis: {e}"))? {
        frames.extend(packet.into_iter().map(|s| s as f32 / 32768.0));
    }
    Ok(Pcm { channels, rate, frames, loop_points: None })
}

/// A sample file in either format FMOD loads here: Ogg Vorbis or RIFF WAVE.
pub fn decode_sample(d: &[u8]) -> Result<Pcm, String> {
    if is_ogg(d) { decode(d) } else { super::wav::decode(d) }
}
