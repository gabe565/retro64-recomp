//! DevIL (`devil.dll`), the image library Bugatron Worlds loads its images with:
//! the eight functions it imports (stdcall), for the formats its files use.
//!
//! The game loads an image from memory (`ilLoadL` with IL_TYPE_UNKNOWN), converts
//! it to RGBA bytes and copies them out of `ilGetData`. Like DevIL, the rows stay
//! in the file's order (BMPs and most TGAs bottom-up): the game never asks DevIL
//! to fix the origin, and flips the rows itself.

use crate::{fatal, Runtime};
use std::collections::HashMap;

const IL_IMAGE_WIDTH: u32 = 0x0de4;
const IL_IMAGE_HEIGHT: u32 = 0x0de5;
const IL_RGBA: u32 = 0x1908;
const IL_UNSIGNED_BYTE: u32 = 0x1401;

#[derive(Default)]
pub struct Devil {
    /// Image names → their image (None until loaded).
    images: HashMap<u32, Option<Image>>,
    next: u32,
    bound: u32,
}

struct Image {
    w: u32,
    h: u32,
    /// RGBA8 in the file's row order, in guest memory (`ilGetData`).
    data: u32,
}

/// Returns (value, argument dwords popped), or None if `name` is not a DevIL import.
pub(crate) fn call(rt: &mut Runtime, name: &str, arg: &dyn Fn(u32) -> u32) -> Option<(u32, u32)> {
    Some(match name {
        "ilInit" => (0, 0),
        // (count, names out)
        "ilGenImages" => {
            for i in 0..arg(0) {
                rt.devil.next += 1;
                let id = rt.devil.next;
                rt.devil.images.insert(id, None);
                rt.mem.w32(arg(1) + 4 * i, id);
            }
            (0, 2)
        }
        "ilBindImage" => {
            rt.devil.bound = arg(0);
            (0, 1)
        }
        // (count, names)
        "ilDeleteImages" => {
            for i in 0..arg(0) {
                let id = rt.mem.r32(arg(1) + 4 * i);
                if let Some(Some(img)) = rt.devil.images.remove(&id) {
                    rt.heap.free(img.data);
                }
            }
            (0, 2)
        }
        // (type, data, size) → IL_TRUE when it loaded.
        "ilLoadL" => {
            if arg(0) != 0 {
                fatal(&format!("ilLoadL: image type {:#x} not emulated (the game passes IL_TYPE_UNKNOWN)", arg(0)));
            }
            let bytes = rt.mem.slice(arg(1), arg(2) as usize).to_vec();
            let decoded = decode(&bytes);
            if let Err(e) = &decoded {
                eprintln!("[devil] ilLoadL: {e}");
            }
            let ok = decoded.is_ok();
            if let Ok((w, h, rgba)) = decoded {
                let data = rt.heap_alloc(rgba.len() as u32, false);
                rt.mem.write(data, &rgba);
                if let Some(Some(old)) = rt.devil.images.insert(rt.devil.bound, Some(Image { w, h, data })) {
                    rt.heap.free(old.data);
                }
            }
            (ok as u32, 3)
        }
        "ilGetInteger" => {
            let img = rt.devil.images.get(&rt.devil.bound).and_then(|i| i.as_ref());
            let v = match arg(0) {
                IL_IMAGE_WIDTH => img.map_or(0, |i| i.w),
                IL_IMAGE_HEIGHT => img.map_or(0, |i| i.h),
                m => fatal(&format!("ilGetInteger({m:#x}) not emulated")),
            };
            (v, 1)
        }
        // Images are kept as RGBA bytes, the only conversion the game asks for.
        "ilConvertImage" => {
            if (arg(0), arg(1)) != (IL_RGBA, IL_UNSIGNED_BYTE) {
                fatal(&format!("ilConvertImage({:#x}, {:#x}) not emulated", arg(0), arg(1)));
            }
            (1, 2)
        }
        "ilGetData" => (rt.devil.images.get(&rt.devil.bound).and_then(|i| i.as_ref()).map_or(0, |i| i.data), 0),
        _ => return None,
    })
}

/// Decode a BMP (8-bit palettised, 24- or 32-bit) or an uncompressed true-colour
/// TGA (24- or 32-bit) to RGBA8, rows in the file's order: (width, height, pixels).
pub fn decode(d: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let u16_at = |o: usize| d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as u32).ok_or("truncated");
    let u32_at = |o: usize| d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok_or("truncated");
    if d.starts_with(b"BM") {
        let (off, hsize, w, h) = (u32_at(10)? as usize, u32_at(14)? as usize, u32_at(18)?, u32_at(22)? as i32);
        let (bpp, comp) = (u16_at(28)?, u32_at(30)?);
        if comp != 0 || h <= 0 || w == 0 || !matches!(bpp, 8 | 24 | 32) {
            return Err(format!("BMP: {bpp}-bit, compression {comp}, height {h} not emulated"));
        }
        let h = h as u32;
        let stride = ((w * bpp).div_ceil(32) * 4) as usize;
        let colors = if bpp == 8 { match u32_at(46)? { 0 => 256, n => n as usize } } else { 0 };
        let palette = d.get(14 + hsize..14 + hsize + 4 * colors).ok_or("BMP: truncated palette")?;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            let row = d.get(off + y * stride..off + y * stride + stride).ok_or("BMP: truncated pixels")?;
            for x in 0..w as usize {
                let (b, g, r, a) = match bpp {
                    8 => {
                        let p = &palette[4 * (row[x] as usize).min(colors - 1)..];
                        (p[0], p[1], p[2], 255)
                    }
                    24 => (row[3 * x], row[3 * x + 1], row[3 * x + 2], 255),
                    _ => (row[4 * x], row[4 * x + 1], row[4 * x + 2], row[4 * x + 3]),
                };
                out.extend([r, g, b, a]);
            }
        }
        return Ok((w, h, out));
    }
    // TGA: no colour map, image type 2 (uncompressed true colour).
    let (id_len, cmap, ty) = (*d.first().ok_or("empty")? as usize, d.get(1), d.get(2));
    if cmap != Some(&0) || ty != Some(&2) {
        return Err(format!("not a BMP or an uncompressed true-colour TGA (TGA type {ty:?})"));
    }
    let (w, h, bpp) = (u16_at(12)?, u16_at(14)?, *d.get(16).ok_or("truncated")? as u32);
    if !matches!(bpp, 24 | 32) {
        return Err(format!("TGA: {bpp}-bit not emulated"));
    }
    let n = (bpp / 8) as usize;
    let px = d.get(18 + id_len..18 + id_len + (w * h) as usize * n).ok_or("TGA: truncated pixels")?;
    let out = px.chunks(n).flat_map(|p| [p[2], p[1], p[0], if n == 4 { p[3] } else { 255 }]).collect();
    Ok((w, h, out))
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn bmp_and_tga_decode_in_file_row_order() {
        // 2×2 24-bit BMP, bottom-up: rows (blue, green) then (red, white), padded to 8 bytes.
        let mut bmp = vec![0u8; 54];
        bmp[..2].copy_from_slice(b"BM");
        bmp[10] = 54;
        bmp[14] = 40;
        bmp[18] = 2;
        bmp[22] = 2;
        bmp[28] = 24;
        bmp.extend([255, 0, 0, 0, 255, 0, 0, 0, 0, 0, 255, 255, 255, 255, 0, 0]);
        let (w, h, px) = decode(&bmp).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(px, [0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255, 255, 255, 255, 255]);
        // 1×1 32-bit TGA (BGRA).
        let mut tga = vec![0u8; 18];
        tga[2] = 2;
        tga[12] = 1;
        tga[14] = 1;
        tga[16] = 32;
        tga.extend([10, 20, 30, 40]);
        assert_eq!(decode(&tga).unwrap(), (1, 1, vec![30, 20, 10, 40]));
        assert!(decode(b"nope").is_err());
    }
}
