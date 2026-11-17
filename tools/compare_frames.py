#!/usr/bin/env python3
"""Compare a native capture (PNG, from `bugatron --dump-frames`) with an
original capture (raw guest pixels from tools/frida/capture_deterministic_frames.js,
saved as .json messages or converted PNG).

Usage:
  compare_frames.py NATIVE.png ORIGINAL.png [--diff OUT.png]
  compare_frames.py --from-frida MESSAGES.json OUTDIR   # write original frames as PNG
  compare_frames.py --from-raw RAW W H PITCH BPP OUT.png  # a raw capture file (flip-N.raw)

Reports the share of pixels that differ, the maximum and mean per-channel
difference, and writes an amplified difference image (|a-b| x 8) if asked.
Standard library only.
"""
import base64, json, struct, sys, zlib


def read_png(path):
    d = open(path, 'rb').read()
    assert d[:8] == b'\x89PNG\r\n\x1a\n', path
    i, idat = 8, b''
    while i < len(d):
        n = struct.unpack('>I', d[i:i + 4])[0]
        t = d[i + 4:i + 8]
        if t == b'IHDR':
            w, h, depth, ctype = struct.unpack('>IIBB', d[i + 8:i + 18])
            assert (depth, ctype) == (8, 6), 'expects 8-bit RGBA'
        elif t == b'IDAT':
            idat += d[i + 8:i + 8 + n]
        i += 12 + n
    raw = zlib.decompress(idat)
    stride = w * 4
    out, prev = bytearray(), bytearray(stride)
    for y in range(h):
        f, row = raw[y * (stride + 1)], bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for x in range(stride):
            a = row[x - 4] if x >= 4 else 0
            b, c = prev[x], (prev[x - 4] if x >= 4 else 0)
            if f == 1: row[x] = (row[x] + a) & 255
            elif f == 2: row[x] = (row[x] + b) & 255
            elif f == 3: row[x] = (row[x] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                row[x] = (row[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        out += row
        prev = row
    return w, h, bytes(out)


def write_png(path, w, h, rgba):
    def chunk(t, data):
        return struct.pack('>I', len(data)) + t + data + struct.pack('>I', zlib.crc32(t + data) & 0xffffffff)
    raw = b''.join(b'\0' + rgba[y * w * 4:(y + 1) * w * 4] for y in range(h))
    with open(path, 'wb') as f:
        f.write(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0))
                + chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b''))


def guest_to_rgba(m, raw=None):
    """Raw guest back-buffer pixels (XRGB8888 or RGB565) to RGBA."""
    raw = base64.b64decode(m['b64']) if raw is None else raw
    w, h, pitch, bpp = m['w'], m['h'], m['pitch'], m['bpp']
    out = bytearray()
    for y in range(h):
        row = raw[y * pitch:]
        for x in range(w):
            if bpp == 32:
                b, g, r = row[4 * x], row[4 * x + 1], row[4 * x + 2]
            else:
                v = row[2 * x] | (row[2 * x + 1] << 8)
                r5, g6, b5 = v >> 11, (v >> 5) & 63, v & 31
                r, g, b = (r5 << 3) | (r5 >> 2), (g6 << 2) | (g6 >> 4), (b5 << 3) | (b5 >> 2)
            out += bytes((r, g, b, 255))
    return w, h, bytes(out)


def compare(a_path, b_path, diff_path=None):
    wa, ha, a = read_png(a_path)
    wb, hb, b = read_png(b_path)
    assert (wa, ha) == (wb, hb), f'size {wa}x{ha} vs {wb}x{hb}'
    n = wa * ha
    differing, total, worst = 0, 0, 0
    diff = bytearray(n * 4)
    for i in range(n):
        d = [abs(a[4 * i + c] - b[4 * i + c]) for c in range(3)]
        m = max(d)
        if m:
            differing += 1
        total += sum(d)
        worst = max(worst, m)
        for c in range(3):
            diff[4 * i + c] = min(255, d[c] * 8)
        diff[4 * i + 3] = 255
    print(f'{a_path} vs {b_path}: {differing}/{n} pixels differ ({100 * differing / n:.2f}%), '
          f'max channel diff {worst}, mean {total / (3 * n):.3f}')
    if diff_path:
        write_png(diff_path, wa, ha, bytes(diff))


if __name__ == '__main__':
    args = sys.argv[1:]
    if args and args[0] == '--from-frida':
        msgs, outdir = json.load(open(args[1])), args[2]
        for m in msgs:
            m = m.get('payload', m)
            if m.get('type') == 'frame':
                w, h, rgba = guest_to_rgba(m)
                p = f"{outdir}/frame-{m['flip']:05d}.png"
                write_png(p, w, h, rgba)
                print(p)
    elif args and args[0] == '--from-raw':
        raw, (w, h, pitch, bpp), out = open(args[1], 'rb').read(), map(int, args[2:6]), args[6]
        write_png(out, *guest_to_rgba({'w': w, 'h': h, 'pitch': pitch, 'bpp': bpp}, raw))
        print(out)
    else:
        diff = args[args.index('--diff') + 1] if '--diff' in args else None
        compare(args[0], args[1], diff)
