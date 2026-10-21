"""S0-12: validate the downloaded screenshots (stdlib only).

usage: python3 -I check_images.py <out_dir>
- headless.png: PNG signature + IHDR size + share of the page colour (#76b900).
- xvfb.xwd: X11 window dump (ZPixmap, 32 bpp) -> xvfb.png, plus the same colour check.
"""
import struct
import sys
import zlib
from pathlib import Path

GREEN = (0x76, 0xB9, 0x00)


def near(px, ref, tol=24):
    return all(abs(a - b) <= tol for a, b in zip(px, ref))


def png_rgb_rows(data: bytes):
    """Decode an 8-bit RGB/RGBA non-interlaced PNG (what Chrome writes)."""
    pos, idat, w = 8, b"", 0
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos:pos + 4])
        kind = data[pos + 4:pos + 8]
        body = data[pos + 8:pos + 8 + length]
        if kind == b"IHDR":
            w, h, depth, ctype, _, _, interlace = struct.unpack(">IIBBBBB", body)
            if depth != 8 or ctype not in (2, 6) or interlace:
                raise SystemExit(f"unsupported PNG: depth={depth} type={ctype} interlace={interlace}")
            bpp = 3 if ctype == 2 else 4
        elif kind == b"IDAT":
            idat += body
        pos += 12 + length
    raw = zlib.decompress(idat)
    stride = w * bpp
    prev = bytearray(stride)
    rows, i = [], 0
    while i < len(raw):
        f, line = raw[i], bytearray(raw[i + 1:i + 1 + stride])
        i += 1 + stride
        for x in range(stride):
            a = line[x - bpp] if x >= bpp else 0
            b = prev[x]
            c = prev[x - bpp] if x >= bpp else 0
            if f == 1:
                line[x] = (line[x] + a) & 255
            elif f == 2:
                line[x] = (line[x] + b) & 255
            elif f == 3:
                line[x] = (line[x] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[x] = (line[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append([tuple(line[k:k + 3]) for k in range(0, stride, bpp)])
        prev = line
    return w, len(rows), rows


def write_png(path: Path, w: int, h: int, rows):
    raw = b"".join(b"\x00" + bytes(c for px in row for c in px) for row in rows)
    chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
                     + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b""))


def report(name, w, h, rows):
    total = w * h
    green = sum(1 for row in rows for px in row if near(px, GREEN))
    distinct = len({px for row in rows[:: max(1, h // 50)] for px in row[:: max(1, w // 50)]})
    print(f"{name}: {w}x{h}, page-colour share {green / total:.1%}, distinct colours (sampled) {distinct}")
    return green / total


def main():
    out = Path(sys.argv[1])
    ok = True
    png = out / "headless.png"
    if png.exists() and png.read_bytes()[:8] == b"\x89PNG\r\n\x1a\n":
        w, h, rows = png_rgb_rows(png.read_bytes())
        ok &= report("headless.png", w, h, rows) > 0.5
    else:
        print("headless.png: missing or not a PNG")
        ok = False
    xwd = out / "xvfb.xwd"
    if xwd.exists():
        d = xwd.read_bytes()
        hdr = struct.unpack(">25I", d[:100])
        header_size, fmt, depth, w, h, bpp, bpl, ncolors = hdr[0], hdr[2], hdr[3], hdr[4], hdr[5], hdr[11], hdr[12], hdr[19]
        if fmt != 2 or bpp != 32:
            print(f"xvfb.xwd: unsupported format={fmt} bpp={bpp}")
            ok = False
        else:
            base = header_size + ncolors * 12
            little = hdr[7] == 0  # byte_order 0 = LSBFirst
            rows = []
            for y in range(h):
                line = d[base + y * bpl: base + y * bpl + w * 4]
                rows.append([(line[x + 2], line[x + 1], line[x]) if little else (line[x + 1], line[x + 2], line[x + 3])
                             for x in range(0, w * 4, 4)])
            write_png(out / "xvfb.png", w, h, rows)
            ok &= report(f"xvfb.xwd (depth {depth}) -> xvfb.png", w, h, rows) > 0.5
    else:
        print("xvfb.xwd: missing")
        ok = False
    print("images:", "PASS" if ok else "FAIL")


main()
