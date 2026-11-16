//! DirectDraw pixel formats (`DDPIXELFORMAT`) and exact conversions.
//!
//! Guest surfaces keep their pixels in guest memory in the format the game
//! chose, so game code that locks a surface reads and writes exactly the bits
//! DirectDraw would expose. Conversion to and from 8-bit-per-channel RGBA (for
//! the GPU) expands channels by bit replication, which is exact at the ends of
//! the range (0 → 0, max → 255) and round-trips every n-bit value.

/// `DDPF_*` flags used by the game.
pub const DDPF_ALPHAPIXELS: u32 = 0x1;
pub const DDPF_RGB: u32 = 0x40;
pub const DDPF_ZBUFFER: u32 = 0x400;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelFormat {
    pub flags: u32,
    /// Bits per pixel (`dwRGBBitCount`, or `dwZBufferBitDepth` for depth formats).
    pub bpp: u32,
    pub r: u32,
    pub g: u32,
    pub b: u32,
    /// Alpha mask (`dwRGBAlphaBitMask`), or the Z mask for depth formats.
    pub a: u32,
}

impl PixelFormat {
    pub const RGB565: PixelFormat = PixelFormat { flags: DDPF_RGB, bpp: 16, r: 0xf800, g: 0x07e0, b: 0x001f, a: 0 };
    pub const RGB555: PixelFormat = PixelFormat { flags: DDPF_RGB, bpp: 16, r: 0x7c00, g: 0x03e0, b: 0x001f, a: 0 };
    pub const ARGB1555: PixelFormat = PixelFormat { flags: DDPF_RGB | DDPF_ALPHAPIXELS, bpp: 16, r: 0x7c00, g: 0x03e0, b: 0x001f, a: 0x8000 };
    pub const ARGB4444: PixelFormat = PixelFormat { flags: DDPF_RGB | DDPF_ALPHAPIXELS, bpp: 16, r: 0x0f00, g: 0x00f0, b: 0x000f, a: 0xf000 };
    pub const XRGB8888: PixelFormat = PixelFormat { flags: DDPF_RGB, bpp: 32, r: 0xff_0000, g: 0xff00, b: 0xff, a: 0 };
    pub const ARGB8888: PixelFormat = PixelFormat { flags: DDPF_RGB | DDPF_ALPHAPIXELS, bpp: 32, r: 0xff_0000, g: 0xff00, b: 0xff, a: 0xff00_0000 };
    pub const Z16: PixelFormat = PixelFormat { flags: DDPF_ZBUFFER, bpp: 16, r: 0, g: 0, b: 0, a: 0xffff };

    pub fn bytes_per_pixel(&self) -> u32 {
        self.bpp.div_ceil(8)
    }

    pub fn is_depth(&self) -> bool {
        self.flags & DDPF_ZBUFFER != 0
    }

    pub fn has_alpha(&self) -> bool {
        self.flags & DDPF_ALPHAPIXELS != 0 && self.a != 0
    }

    /// Parse a 32-byte `DDPIXELFORMAT`.
    pub fn from_ddpf(b: &[u8]) -> PixelFormat {
        let d = |i: usize| u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap());
        let flags = d(1);
        if flags & DDPF_ZBUFFER != 0 {
            // dwZBufferBitDepth at +0xc (union with dwRGBBitCount), dwZBitMask at +0x14
            return PixelFormat { flags, bpp: d(3), r: 0, g: 0, b: 0, a: d(5) };
        }
        PixelFormat { flags, bpp: d(3), r: d(4), g: d(5), b: d(6), a: d(7) }
    }

    /// Serialise as a 32-byte `DDPIXELFORMAT`.
    pub fn to_ddpf(&self) -> [u8; 32] {
        let mut v = [0u32; 8];
        v[0] = 32;
        v[1] = self.flags;
        if self.is_depth() {
            v[3] = self.bpp;
            v[5] = self.a;
        } else {
            v[3] = self.bpp;
            v[4] = self.r;
            v[5] = self.g;
            v[6] = self.b;
            v[7] = self.a;
        }
        let mut out = [0u8; 32];
        for (i, x) in v.iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&x.to_le_bytes());
        }
        out
    }

    /// Read one pixel (little-endian) from `p`.
    pub fn load(&self, p: &[u8]) -> u32 {
        match self.bytes_per_pixel() {
            2 => u16::from_le_bytes([p[0], p[1]]) as u32,
            3 => u32::from_le_bytes([p[0], p[1], p[2], 0]),
            4 => u32::from_le_bytes([p[0], p[1], p[2], p[3]]),
            1 => p[0] as u32,
            n => panic!("unsupported pixel size {n}"),
        }
    }

    pub fn store(&self, p: &mut [u8], v: u32) {
        let b = v.to_le_bytes();
        let n = self.bytes_per_pixel() as usize;
        p[..n].copy_from_slice(&b[..n]);
    }

    /// Expand a pixel to (r, g, b, a) bytes. Formats without alpha give a = 255.
    pub fn to_rgba(&self, v: u32) -> [u8; 4] {
        [expand(v, self.r), expand(v, self.g), expand(v, self.b), if self.has_alpha() { expand(v, self.a) } else { 255 }]
    }

    /// Pack (r, g, b, a) bytes, truncating each channel to its width (as the
    /// game's own packing code does, e.g. `r & 0xf8` for 5-bit red).
    pub fn from_rgba(&self, c: [u8; 4]) -> u32 {
        pack(c[0], self.r) | pack(c[1], self.g) | pack(c[2], self.b) | if self.has_alpha() { pack(c[3], self.a) } else { 0 }
    }

    /// Convert a `w`×`h` region with row pitch `pitch` to tightly packed RGBA8.
    pub fn to_rgba8(&self, src: &[u8], w: u32, h: u32, pitch: u32) -> Vec<u8> {
        let bpp = self.bytes_per_pixel() as usize;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            let row = &src[y * pitch as usize..];
            for x in 0..w as usize {
                out.extend_from_slice(&self.to_rgba(self.load(&row[x * bpp..])));
            }
        }
        out
    }

    /// Convert tightly packed RGBA8 into this format (row pitch `pitch`).
    pub fn from_rgba8(&self, rgba: &[u8], w: u32, h: u32, dst: &mut [u8], pitch: u32) {
        let bpp = self.bytes_per_pixel() as usize;
        for y in 0..h as usize {
            for x in 0..w as usize {
                let i = (y * w as usize + x) * 4;
                let v = self.from_rgba([rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]);
                self.store(&mut dst[y * pitch as usize + x * bpp..], v);
            }
        }
    }
}

/// Extract the channel under `mask` and widen it to 8 bits by bit replication.
pub fn expand(v: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let bits = mask.count_ones();
    let c = (v & mask) >> shift;
    if bits >= 8 {
        return (c >> (bits - 8)) as u8;
    }
    let mut out = 0u32;
    let mut filled = 0;
    while filled < 8 {
        out = (out << bits) | c;
        filled += bits;
    }
    (out >> (filled - 8)) as u8
}

/// Narrow an 8-bit channel to the width of `mask` by truncation and place it.
pub fn pack(c: u8, mask: u32) -> u32 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let bits = mask.count_ones();
    let v = if bits >= 8 { (c as u32) << (bits - 8) } else { c as u32 >> (8 - bits) };
    (v << shift) & mask
}
