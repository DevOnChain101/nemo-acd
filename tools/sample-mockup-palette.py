#!/usr/bin/env python3
"""Dominant colours of the TUI reference mockups (design/tui-reference/*.png), stdlib only.

Decodes the PNG (zlib + filters, 8-bit RGB/RGBA, non-interlaced), quantises each pixel to 8 levels per
channel step (default 4), and prints the most frequent colours, split into dark surfaces and the rest.
Usage: python3 tools/sample-mockup-palette.py [--top N] [--region x0,y0,x1,y1] file.png...
"""
import colorsys
import struct
import sys
import zlib
from collections import Counter


def decode(path):
    d = open(path, "rb").read()
    assert d[:8] == b"\x89PNG\r\n\x1a\n"
    i, idat, w = 8, b"", None
    while i < len(d):
        n, t = struct.unpack(">I4s", d[i:i + 8])
        c = d[i + 8:i + 8 + n]
        if t == b"IHDR":
            w, h, depth, ctype, _, _, inter = struct.unpack(">IIBBBBB", c)
            assert depth == 8 and ctype in (2, 6) and inter == 0, (depth, ctype, inter)
        elif t == b"IDAT":
            idat += c
        i += 12 + n
    bpp = 3 if ctype == 2 else 4
    raw = zlib.decompress(idat)
    stride = w * bpp
    rows, prev = [], bytearray(stride)
    p = 0
    for _ in range(h):
        f = raw[p]
        line = bytearray(raw[p + 1:p + 1 + stride])
        p += 1 + stride
        for x in range(stride):
            a = line[x - bpp] if x >= bpp else 0
            b = prev[x]
            c = prev[x - bpp] if x >= bpp else 0
            if f == 1:
                line[x] = (line[x] + a) & 255
            elif f == 2:
                line[x] = (line[x] + b) & 255
            elif f == 3:
                line[x] = (line[x] + ((a + b) >> 1)) & 255
            elif f == 4:
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                pr = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                line[x] = (line[x] + pr) & 255
        rows.append(line)
        prev = line
    return w, h, bpp, rows


def main(argv):
    top, region, files = 14, None, []
    it = iter(argv)
    for a in it:
        if a == "--top":
            top = int(next(it))
        elif a == "--region":
            region = [int(v) for v in next(it).split(",")]
        else:
            files.append(a)
    for f in files:
        w, h, bpp, rows = decode(f)
        x0, y0, x1, y1 = region or (0, 0, w, h)
        cnt = Counter()
        for y in range(y0, min(y1, h)):
            r = rows[y]
            for x in range(x0, min(x1, w)):
                px = r[x * bpp:x * bpp + 3]
                cnt[(px[0] // 4 * 4 + 2, px[1] // 4 * 4 + 2, px[2] // 4 * 4 + 2)] += 1
        total = sum(cnt.values())
        print(f"{f}: {w}x{h}")
        dark = [(c, n) for c, n in cnt.most_common() if max(c) < 60]
        bright = [(c, n) for c, n in cnt.most_common() if max(c) >= 60 and max(c) - min(c) >= 40]
        soft = [(c, n) for c, n in cnt.most_common() if min(c) >= 150 and max(c) - min(c) < 40]
        # vivid pixels grouped by hue (30 degree buckets): the most frequent 16-level colour of each
        hue = {}
        for y in range(y0, min(y1, h), 2):
            r = rows[y]
            for x in range(x0, min(x1, w), 2):
                R, G, B = r[x * bpp:x * bpp + 3]
                mx, mn = max(R, G, B), min(R, G, B)
                if mx < 140 or (mx - mn) < 0.45 * mx:
                    continue
                hh = colorsys.rgb_to_hsv(R / 255, G / 255, B / 255)[0] * 360
                hue.setdefault(int(hh // 30) * 30, Counter())[(R // 16 * 16 + 8, G // 16 * 16 + 8, B // 16 * 16 + 8)] += 1
        print("  vivid by hue bucket:")
        for hb in sorted(hue, key=lambda k: -sum(hue[k].values())):
            c, n = hue[hb].most_common(1)[0]
            print(f"    hue {hb:3d}-{hb + 30:3d}: {sum(hue[hb].values()):6d} px, most common #{c[0]:02X}{c[1]:02X}{c[2]:02X} ({n})")
        for name, lst in (("surfaces", dark), ("text", soft)):
            print(f"  {name}:")
            for c, n in lst[:top]:
                print(f"    #{c[0]:02X}{c[1]:02X}{c[2]:02X}  {n / total * 100:5.2f}%")


if __name__ == "__main__":
    main(sys.argv[1:])
