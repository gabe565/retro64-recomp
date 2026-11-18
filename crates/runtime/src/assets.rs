//! Runtime edits of original game assets, made in memory when the game reads them
//! (files.rs `substitute`); the files themselves are never changed.

/// The title logo without "GOLD", from Gold's `m/junk2.bmp` (512×512, 24-bit): the
/// "BUGATRON" lettering is stretched to rows 43–466 (of 512), where the free
/// version's logo has the same artwork; the two then differ only by resampling
/// (0.4% of pixels by more than 16 levels). None if the file is not that bitmap.
pub fn plain_logo(bmp: &[u8]) -> Option<Vec<u8>> {
    let u32_at = |o: usize| Some(u32::from_le_bytes(bmp.get(o..o + 4)?.try_into().ok()?));
    let off = u32_at(10)? as usize;
    let (w, h) = (u32_at(18)? as i32, u32_at(22)? as i32);
    let bpp = u16::from_le_bytes(bmp.get(28..30)?.try_into().ok()?);
    if bpp != 24 || w <= 0 || h <= 16 {
        return None;
    }
    let (w, h) = (w as usize, h as usize);
    let stride = (w * 3 + 3) & !3;
    if bmp.len() < off + stride * h {
        return None;
    }
    // Rows top to bottom (BMP rows are stored bottom-up).
    let row = |y: usize| &bmp[off + (h - 1 - y) * stride..][..w * 3];
    let lit = |y: usize| row(y).chunks(3).filter(|p| p.iter().any(|&c| c > 40)).count();
    let top = (0..h).find(|&y| lit(y) > 0)?;
    let bottom = (0..h).rev().find(|&y| lit(y) > 0)?;
    // "GOLD" sits below the lettering: split at the emptiest row of the lower part.
    let split = (top + (bottom - top) * 6 / 10..bottom).min_by_key(|&y| lit(y))?;
    if split <= top + 8 {
        return None;
    }
    // Where the free version's logo has the lettering.
    let (t0, t1) = ((43 * h + 256) / 512, (466 * h + 256) / 512);
    let mut out = bmp.to_vec();
    for y in 0..h {
        let dst = off + (h - 1 - y) * stride;
        let pixels = &mut out[dst..dst + w * 3];
        if y < t0 || y > t1 {
            pixels.fill(0);
            continue;
        }
        // y in t0..=t1 maps to the lettering top..=split (linear).
        let sy = top as f32 + (y - t0) as f32 * (split - top) as f32 / (t1 - t0) as f32;
        let (y0, t) = (sy.floor() as usize, sy.fract());
        let (a, b) = (row(y0), row((y0 + 1).min(split)));
        for (i, p) in pixels.iter_mut().enumerate() {
            *p = (a[i] as f32 * (1.0 - t) + b[i] as f32 * t).round() as u8;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 24-bit BMP of `w`×`h` from rows top to bottom.
    fn bmp(w: usize, rows: &[Vec<[u8; 3]>]) -> Vec<u8> {
        let h = rows.len();
        let stride = (w * 3 + 3) & !3;
        let mut d = vec![0u8; 54 + stride * h];
        d[..2].copy_from_slice(b"BM");
        d[10..14].copy_from_slice(&54u32.to_le_bytes());
        d[14..18].copy_from_slice(&40u32.to_le_bytes());
        d[18..22].copy_from_slice(&(w as u32).to_le_bytes());
        d[22..26].copy_from_slice(&(h as u32).to_le_bytes());
        d[26..28].copy_from_slice(&1u16.to_le_bytes());
        d[28..30].copy_from_slice(&24u16.to_le_bytes());
        for (y, r) in rows.iter().enumerate() {
            let o = 54 + (h - 1 - y) * stride;
            for (x, p) in r.iter().enumerate() {
                d[o + x * 3..o + x * 3 + 3].copy_from_slice(p);
            }
        }
        d
    }

    #[test]
    fn the_lower_word_is_removed_and_the_lettering_stretched() {
        // 40 rows: lettering 4..=23 (green), a gap at 24..=25, "GOLD" 26..=35 (red).
        let rows: Vec<Vec<[u8; 3]>> = (0..40)
            .map(|y| {
                let c = match y {
                    4..=23 => [0, 200, 0],
                    26..=35 => [0, 0, 200],
                    _ => [0, 0, 0],
                };
                vec![c; 8]
            })
            .collect();
        let out = plain_logo(&bmp(8, &rows)).expect("a 24-bit bitmap");
        let px = |y: usize| out[54 + (39 - y) * 24..][..3].to_vec();
        // The free version's span, rows 43..=466 of 512, is rows 3..=36 of 40.
        assert_eq!(px(2), [0, 0, 0], "above stays empty");
        assert!((4..=35).all(|y| px(y)[1] > 100 && px(y)[2] < 60), "the lettering now spans the height");
        assert_eq!(px(38), [0, 0, 0], "below stays empty");
        assert_eq!(plain_logo(b"not a bitmap"), None);
    }
}
