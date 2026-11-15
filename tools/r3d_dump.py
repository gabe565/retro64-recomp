#!/usr/bin/env python3
"""Parse and summarize Bugatron `.r3d` ("INDOTEK 2") model files.

Layout recovered from Bug3D.exe Gold v1.20a, r3d_Load3DModel (FUN_0041f4c0)
and r3d_ListAdd3DModel (FUN_0041e950). See docs/formats/r3d.md.

Validated: all 136 .r3d files in Gold 1.20 parse to exactly their file length,
every vertex normal is unit length, and every index is below its component's
vertex count.

Usage:
    tools/r3d_dump.py FILE.r3d [...]        # one-line summary per file
    tools/r3d_dump.py -v FILE.r3d           # include per-component materials
"""

from __future__ import annotations

import argparse
import struct
import sys
from dataclasses import dataclass

MAGIC = b"INDOTEK 2\0"
VERTEX_SIZE = 32  # x y z, nx ny nz, tu tv  (f32 each; matches D3DVERTEX)
TEX_NAME_LEN = 129


@dataclass
class Material:
    values: tuple[float, ...]  # 11 f32; meaning partly unresolved (see docs)
    texture: str
    texture2: str


@dataclass
class Component:
    vertices: list[tuple[float, ...]]
    indices: list[int]
    material: Material


@dataclass
class Model:
    bounds_min: tuple[float, float, float]
    bounds_max: tuple[float, float, float]
    radius: float
    components: list[Component]


def _cstr(raw: bytes) -> str:
    return raw.split(b"\0", 1)[0].decode("latin-1")


def parse(data: bytes) -> Model:
    if data[:10] != MAGIC:
        raise ValueError(f"bad magic {data[:10]!r}")
    o = 10
    *floats, count = struct.unpack_from("<7fi", data, o)
    o += 32
    vcounts = struct.unpack_from(f"<{count}I", data, o)
    o += 4 * count
    icounts = struct.unpack_from(f"<{count}I", data, o)
    o += 4 * count
    verts = [struct.unpack_from("<8f", data, o + i * VERTEX_SIZE) for i in range(sum(vcounts))]
    o += VERTEX_SIZE * sum(vcounts)
    idx = list(struct.unpack_from(f"<{sum(icounts)}H", data, o))
    o += 2 * sum(icounts)
    comps: list[Component] = []
    vi = ii = 0
    for c in range(count):
        values = struct.unpack_from("<11f", data, o)
        o += 44
        t1 = _cstr(data[o:o + TEX_NAME_LEN])
        o += TEX_NAME_LEN
        t2 = _cstr(data[o:o + TEX_NAME_LEN])
        o += TEX_NAME_LEN
        comps.append(Component(verts[vi:vi + vcounts[c]], idx[ii:ii + icounts[c]], Material(values, t1, t2)))
        vi += vcounts[c]
        ii += icounts[c]
    if o != len(data):
        raise ValueError(f"trailing/short data: parsed {o} of {len(data)} bytes")
    return Model(tuple(floats[0:3]), tuple(floats[3:6]), floats[6], comps)


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("-v", "--verbose", action="store_true")
    ap.add_argument("files", nargs="+")
    args = ap.parse_args(argv)
    status = 0
    for path in args.files:
        try:
            with open(path, "rb") as fh:
                m = parse(fh.read())
        except Exception as exc:  # noqa: BLE001
            print(f"FAIL {path}: {exc}", file=sys.stderr)
            status = 1
            continue
        nv = sum(len(c.vertices) for c in m.components)
        nt = sum(len(c.indices) for c in m.components) // 3
        print(f"{path}: {len(m.components)} components, {nv} vertices, {nt} triangles, radius {m.radius:.3f}")
        if args.verbose:
            for i, c in enumerate(m.components):
                vals = " ".join(f"{v:g}" for v in c.material.values)
                print(f"  [{i}] v={len(c.vertices)} t={len(c.indices)//3} mat=[{vals}] tex={c.material.texture!r} tex2={c.material.texture2!r}")
    return status


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
