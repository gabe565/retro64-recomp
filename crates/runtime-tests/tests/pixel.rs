//! Pixel-format parsing and conversions.

use bugatron_runtime::ddraw::caps;
use bugatron_runtime::ddraw::pixel::{expand, pack, PixelFormat};

#[test]
fn enumerated_texture_formats_parse_to_the_known_formats() {
    let parsed: Vec<PixelFormat> = caps::TEXTURE_FORMATS.chunks(32).map(PixelFormat::from_ddpf).collect();
    assert_eq!(parsed, [PixelFormat::RGB565, PixelFormat::ARGB1555, PixelFormat::ARGB4444, PixelFormat::ARGB8888, PixelFormat::RGB555]);
    for (f, raw) in parsed.iter().zip(caps::TEXTURE_FORMATS.chunks(32)) {
        assert_eq!(&f.to_ddpf()[..], raw, "round trip of {f:?}");
    }
    let z = PixelFormat::from_ddpf(&caps::ZBUFFER_FORMAT);
    assert_eq!(z, PixelFormat::Z16);
    assert_eq!(&z.to_ddpf()[..], &caps::ZBUFFER_FORMAT[..]);
    assert_eq!(z.bytes_per_pixel(), 2);
}

#[test]
fn channel_expansion_replicates_bits() {
    assert_eq!(expand(0x1f, 0x1f), 255);
    assert_eq!(expand(0x10, 0x1f), 0x84); // 10000 → 10000100
    assert_eq!(expand(0x3f << 5, 0x7e0), 255);
    assert_eq!(expand(0x20 << 5, 0x7e0), 0x82); // 100000 → 10000010
    assert_eq!(expand(0x8, 0xf), 0x88);
    assert_eq!(expand(0x8000, 0x8000), 255);
    assert_eq!(expand(0, 0x8000), 0);
    assert_eq!(expand(0xab_0000, 0xff_0000), 0xab);
    // Every n-bit value survives expand → pack (truncation).
    for (mask, n) in [(0x1fu32, 32u32), (0x7e0, 64), (0xf000, 16), (0x8000, 2)] {
        for v in 0..n {
            let raw = v << mask.trailing_zeros();
            assert_eq!(pack(expand(raw, mask), mask), raw, "mask {mask:#x} value {v}");
        }
    }
}

#[test]
fn packing_truncates_like_the_games_own_code() {
    // Game code packs 565 as ((r & 0xf8) << 8) | ((g & 0xfc) << 3) | (b >> 3) (0x427db0 case 0).
    let f = PixelFormat::RGB565;
    for &(r, g, b) in &[(0u8, 0u8, 0u8), (255, 255, 255), (0x87, 0x43, 0x2f), (7, 3, 7)] {
        let game = ((r as u32 & 0xf8) << 8) | ((g as u32 & 0xfc) << 3) | (b as u32 >> 3);
        assert_eq!(f.from_rgba([r, g, b, 0]), game);
    }
    // 1555 alpha takes the top bit; 4444 the top nibble.
    assert_eq!(PixelFormat::ARGB1555.from_rgba([0, 0, 0, 0x80]), 0x8000);
    assert_eq!(PixelFormat::ARGB1555.from_rgba([0, 0, 0, 0x7f]), 0);
    assert_eq!(PixelFormat::ARGB4444.from_rgba([0xff, 0, 0x10, 0xf0]), 0xff01);
    // Formats without alpha read back opaque.
    assert_eq!(PixelFormat::XRGB8888.to_rgba(0x0012_3456), [0x12, 0x34, 0x56, 255]);
    assert_eq!(PixelFormat::ARGB8888.to_rgba(0x7812_3456), [0x12, 0x34, 0x56, 0x78]);
}

#[test]
fn region_conversion_honours_pitch() {
    let f = PixelFormat::RGB565;
    // 2×2 image with a 6-byte pitch (2 bytes of padding per row).
    let src = [0x00, 0xf8, 0xe0, 0x07, 0xaa, 0xaa, 0x1f, 0x00, 0xff, 0xff, 0xbb, 0xbb];
    let rgba = f.to_rgba8(&src, 2, 2, 6);
    assert_eq!(rgba, [255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255]);
    let mut back = [0xccu8; 12];
    f.from_rgba8(&rgba, 2, 2, &mut back, 6);
    assert_eq!(&back[0..4], &src[0..4]);
    assert_eq!(&back[6..10], &src[6..10]);
    assert_eq!(&back[4..6], &[0xcc, 0xcc], "padding untouched");
}
