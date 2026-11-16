//! Frame capture for diagnostics and visual comparison: the guest front buffer
//! after a `Flip`, read back through the same path game code uses, as PNG.

use crate::Runtime;
use std::io::Write;
use std::path::Path;

/// Write `rgba` (w×h, tightly packed) as an uncompressed (stored-deflate) PNG.
pub fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) -> std::io::Result<()> {
    fn chunk(out: &mut Vec<u8>, ty: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend_from_slice(ty);
        out.extend_from_slice(data);
        let crc = crc32(&out[start..]);
        out.extend_from_slice(&crc.to_be_bytes());
    }
    let mut raw = Vec::with_capacity(((w * 4 + 1) * h) as usize);
    for y in 0..h as usize {
        raw.push(0); // filter: none
        raw.extend_from_slice(&rgba[y * w as usize * 4..(y + 1) * w as usize * 4]);
    }
    let mut z = vec![0x78, 0x01];
    for (i, block) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        z.push(last as u8);
        z.extend_from_slice(&(block.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    std::fs::File::create(path)?.write_all(&out)
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

/// Capture the front buffer after flip number `rt.com.flips` if requested.
pub(crate) fn maybe_capture(rt: &mut Runtime, front: u32) {
    let n = rt.com.flips;
    if !rt.capture_frames.contains(&n) {
        return;
    }
    super::sync_for_cpu(rt, front);
    let s = rt.com.surface(front).clone();
    let rgba = s.fmt.to_rgba8(rt.mem.slice(s.bits, s.size_bytes() as usize), s.w, s.h, s.pitch);
    let path = rt.capture_dir.join(format!("frame-{n:05}.png"));
    match write_png(&path, s.w, s.h, &rgba) {
        Ok(()) => eprintln!("[capture] {}", path.display()),
        Err(e) => eprintln!("[capture] {}: {e}", path.display()),
    }
}
