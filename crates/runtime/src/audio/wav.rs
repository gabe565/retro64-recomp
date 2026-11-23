//! RIFF WAVE decoding for FMOD's `FSOUND_Sample_Load` (PCM 8/16-bit, mono/stereo).

/// Decoded sample: interleaved f32 frames in [-1, 1].
#[derive(Debug, Clone, PartialEq)]
pub struct Pcm {
    pub channels: u16,
    pub rate: u32,
    pub frames: Vec<f32>,
    /// Loop points from a `smpl` chunk, in frames: (start, end exclusive).
    pub loop_points: Option<(u32, u32)>,
}

impl Pcm {
    pub fn len(&self) -> usize {
        self.frames.len() / self.channels as usize
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

pub fn decode(d: &[u8]) -> Result<Pcm, String> {
    if d.len() < 12 || &d[0..4] != b"RIFF" || &d[8..12] != b"WAVE" {
        return Err("not a RIFF WAVE file".into());
    }
    let u16_at = |o: usize| u16::from_le_bytes([d[o], d[o + 1]]);
    let u32_at = |o: usize| u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]);
    let (mut fmt, mut data, mut loop_points) = (None, None, None);
    let mut i = 12;
    while i + 8 <= d.len() {
        let id = &d[i..i + 4];
        let size = u32_at(i + 4) as usize;
        let body = &d[i + 8..(i + 8 + size).min(d.len())];
        match id {
            b"fmt " if body.len() >= 16 => fmt = Some((u16_at(i + 8), u16_at(i + 10), u32_at(i + 12), u16_at(i + 22))),
            b"data" => data = Some(body),
            b"smpl" if body.len() >= 36 + 24 && u32_at(i + 8 + 28) > 0 => {
                let lp = i + 8 + 36;
                loop_points = Some((u32_at(lp + 8), u32_at(lp + 12) + 1));
            }
            _ => {}
        }
        i += 8 + size + (size & 1);
    }
    let (format, channels, rate, bits) = fmt.ok_or("no fmt chunk")?;
    let data = data.ok_or("no data chunk")?;
    if format != 1 {
        return Err(format!("WAVE format {format} is not PCM"));
    }
    if channels == 0 || channels > 2 {
        return Err(format!("{channels} channels"));
    }
    let frames: Vec<f32> = match bits {
        8 => data.iter().map(|&b| (b as f32 - 128.0) / 128.0).collect(),
        16 => data.as_chunks::<2>().0.iter().map(|&c| i16::from_le_bytes(c) as f32 / 32768.0).collect(),
        b => return Err(format!("{b}-bit PCM")),
    };
    Ok(Pcm { channels, rate, frames, loop_points })
}
