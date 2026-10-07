#!/usr/bin/env python3
"""Write the Apark icon (same design as widgets::icon) as a PNG, no dependencies."""
import math, struct, sys, zlib

def icon(size):
    s = float(size)
    radius = s * 0.22
    rows = []
    for y in range(size):
        row = bytearray([0])
        for x in range(size):
            fx, fy = x + 0.5, y + 0.5
            cx, cy = min(max(fx, radius), s - radius), min(max(fy, radius), s - radius)
            if math.hypot(fx - cx, fy - cy) > radius:
                row += b"\0\0\0\0"
                continue
            t = fy / s
            c = [int(20 + 20 * t), int(120 - 30 * t), int(255 - 40 * t), 255]
            l, r, top, bot = s * 0.2, s * 0.8, s * 0.3, s * 0.7
            th = max(s * 0.045, 1.0)
            if l <= fx <= r and top <= fy <= bot:
                border = fx - l < th or r - fx < th or fy - top < th or bot - fy < th
                mid = s * 0.5
                flap_y = top + min(fx - l, r - fx) * ((s * 0.52 - top) / (mid - l))
                flap = abs(fy - flap_y) < th * 1.1 and fy <= s * 0.53 + th
                if border or flap:
                    c = [255, 255, 255, 255]
            row += bytes(c)
        rows.append(bytes(row))
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(b"".join(rows), 9)) + chunk(b"IEND", b"")

if __name__ == "__main__":
    size = int(sys.argv[2]) if len(sys.argv) > 2 else 512
    open(sys.argv[1], "wb").write(icon(size))
