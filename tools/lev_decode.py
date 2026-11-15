#!/usr/bin/env python3
"""Decode Bugatron Gold `*.lev` files to their plaintext (XML-like / text) form.

Encoding, as recovered from Bug3D.exe, Bugatron Gold v1.20a
(SHA-256 be581a63bccfc7dcc78a5c55cacc02e9f7aab9e5398540d2cc86a7aba758919e):

1. Every byte of the file is bitwise-NOT'ed.
   - Loop inside FUN_00417fe0 (level-pack scan, ~0x4180fe), gated by the
     global at 0x4370a4 == 1 ("encrypted files" flag, .data initial value 1).
2. The result is a JCALG1-style compressed block:
     u16 magic 0x434A ("JC"), u32 decompressed_size, then a 32-bit-word,
     MSB-first bitstream.
   - FUN_0041ce10: validate magic, return decompressed_size
   - FUN_0041e916: allocation size = decompressed_size + 4
   - FUN_0041cfae: decompressor (ported below)
   - FUN_004076a0: wrapper that calls the three above and frees the input

This is an analysis tool. It never writes to the input files.

Usage:
    tools/lev_decode.py INPUT.lev [INPUT2.lev ...]          # verify only
    tools/lev_decode.py -o OUTDIR INPUT.lev [...]           # write decoded files
"""

from __future__ import annotations

import argparse
import os
import struct
import sys

JC_MAGIC = b"JC"


class _BitReader:
    """MSB-first bit reader that refills a 32-bit little-endian word at a time.

    Mirrors the `add reg,reg / adc` idiom in FUN_0041cfae: the tag register
    starts at 0x80000000; when shifting leaves zero, the next dword is loaded
    and a sentinel 1 bit is shifted in at the bottom.
    """

    def __init__(self, data: bytes, pos: int) -> None:
        self.data = data
        self.pos = pos
        self.tag = 0x80000000

    def bit(self) -> int:
        carry = (self.tag >> 31) & 1
        self.tag = (self.tag << 1) & 0xFFFFFFFF
        if self.tag == 0:
            if self.pos + 4 <= len(self.data):
                word = struct.unpack_from("<I", self.data, self.pos)[0]
            else:
                word = 0
            self.pos += 4
            carry = (word >> 31) & 1
            self.tag = ((word << 1) | 1) & 0xFFFFFFFF
        return carry

    def bits(self, count: int) -> int:
        value = 0
        for _ in range(count):
            value = (value << 1) | self.bit()
        return value

    def gamma(self) -> int:
        value = 1
        while True:
            value = (value << 1) | self.bit()
            if not self.bit():
                return value


def jc_decompress(src: bytes) -> tuple[int, bytes]:
    """Port of FUN_0041cfae. Returns (declared_size, output)."""
    if src[:2] != JC_MAGIC:
        raise ValueError(f"bad magic {src[:2]!r}, expected {JC_MAGIC!r}")
    declared = struct.unpack_from("<I", src, 2)[0]
    br = _BitReader(src, 6)
    out = bytearray()

    index_bits = 8   # local_8
    last_offset = 1  # uVar3
    literal_bits = 8  # local_c
    literal_bias = 0  # local_10

    def copy(offset: int, length: int) -> None:
        for _ in range(length):
            out.append(out[-offset])

    while True:
        if br.bit():  # literal
            out.append((br.bits(literal_bits) + literal_bias) & 0xFF)
            continue
        if br.bit():  # normal match
            hi = br.gamma()
            if hi == 2:  # repeat last offset
                copy(last_offset, br.gamma())
                continue
            offset = br.bits(index_bits) | ((hi - 3) << index_bits)
            length = br.gamma()
            if offset >= 0x10000:
                length += 3
            elif offset >= 0x37FF:
                length += 2
            elif offset > 0x27E:
                length += 1
            elif offset > 0x7F:
                pass
            else:
                length += 4
            copy(offset, length)
            last_offset = offset
            continue
        if br.bit():  # one-byte / control
            v = br.bits(4) - 1
            if v == 0:
                out.append(0)
            elif v > 0:
                copy(v, 1)
            elif br.bit():  # raw 256-byte literal blocks
                while True:
                    for _ in range(256):
                        out.append(br.bits(8))
                    if not br.bit():
                        break
            else:  # change literal width / bias
                literal_bits = br.bit() + 7
                literal_bias = 0
                if literal_bits != 8:
                    literal_bias = br.bits(8)
            continue
        # short match
        offset = br.bits(7)
        length = br.bits(2) + 2
        if offset:
            copy(offset, length)
            last_offset = offset
            continue
        if length != 2:
            index_bits = br.bits(length + 1)
            continue
        break  # end of stream
    return declared, bytes(out)


def decode_lev(raw: bytes) -> bytes:
    inverted = bytes((~b) & 0xFF for b in raw)
    declared, out = jc_decompress(inverted)
    if declared != len(out):
        raise ValueError(f"size mismatch: header {declared}, decoded {len(out)}")
    return out


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("-o", "--outdir", help="write decoded files here (name minus .lev, lowercased)")
    ap.add_argument("inputs", nargs="+")
    args = ap.parse_args(argv)
    status = 0
    for path in args.inputs:
        try:
            with open(path, "rb") as fh:
                out = decode_lev(fh.read())
        except Exception as exc:  # noqa: BLE001 - report and continue
            print(f"FAIL {path}: {exc}", file=sys.stderr)
            status = 1
            continue
        print(f"ok   {path}: {len(out)} bytes")
        if args.outdir:
            os.makedirs(args.outdir, exist_ok=True)
            name = os.path.basename(path)
            if name.lower().endswith(".lev"):
                name = name[:-4]
            with open(os.path.join(args.outdir, name.lower()), "wb") as fh:
                fh.write(out)
    return status


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
