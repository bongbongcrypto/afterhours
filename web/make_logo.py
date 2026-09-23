# -*- coding: utf-8 -*-
"""Draw web/logo.png (512x512) from the same shapes as web/logo.svg.

stdlib only (zlib + struct), 4x4 supersampling for smooth edges.

    python web/make_logo.py [out.png]
"""
import struct
import sys
import zlib
from pathlib import Path

SIZE = 512
SS = 4  # samples per pixel per axis
VOID = (0x08, 0x09, 0x0A)
MIST = (0xD0, 0xD6, 0xE0)
FOG = (0x8A, 0x8F, 0x98)
LIME = (0xE4, 0xF2, 0x22)

# (x1, y1, x2, y2, half-width, colour): lines with round caps, as in logo.svg
LINES = [(84, 196, 428, 196, 8, MIST)] + [(x, 282, x, 346, 8, FOG) for x in (92, 140, 172, 236)]
CIRCLES = [(394, 314, 42, LIME)]


def colour_at(x, y):
    for cx, cy, r, c in CIRCLES:
        if (x - cx) ** 2 + (y - cy) ** 2 <= r * r:
            return c
    for x1, y1, x2, y2, hw, c in LINES:
        dx, dy = x2 - x1, y2 - y1
        t = ((x - x1) * dx + (y - y1) * dy) / float(dx * dx + dy * dy)
        t = 0.0 if t < 0 else 1.0 if t > 1 else t
        px, py = x1 + t * dx, y1 + t * dy
        if (x - px) ** 2 + (y - py) ** 2 <= hw * hw:
            return c
    return VOID


def main():
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent / "logo.png"
    offs = [(i + 0.5) / SS for i in range(SS)]
    n = float(SS * SS)
    rows = bytearray()
    for py in range(SIZE):
        rows.append(0)  # filter: none
        for px in range(SIZE):
            r = g = b = 0
            for oy in offs:
                for ox in offs:
                    c = colour_at(px + ox, py + oy)
                    r += c[0]
                    g += c[1]
                    b += c[2]
            rows += bytes((round(r / n), round(g / n), round(b / n)))

    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", SIZE, SIZE, 8, 2, 0, 0, 0)) \
        + chunk(b"IDAT", zlib.compress(bytes(rows), 9)) + chunk(b"IEND", b"")
    out.write_bytes(png)
    print("wrote %s (%d bytes)" % (out, len(png)))


if __name__ == "__main__":
    main()
