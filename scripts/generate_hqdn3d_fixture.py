#!/usr/bin/env python3
"""Synthetic temporal noise without external codec tools or private samples."""
from pathlib import Path
root = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
for depth in (8, 10):
    chroma = '420' if depth == 8 else '420p10'
    data = bytearray(f'YUV4MPEG2 W4 H4 F25:1 Ip C{chroma}\n'.encode())
    for delta in (0, -4, 4, -4, 4, -4, 4, 0):
        data += b'FRAME\n'
        for base, count in ((100, 16), (140, 4), (200, 4)):
            value = (base+delta) << (depth-8)
            data += (bytes([value]) if depth == 8 else value.to_bytes(2,'little'))*count
    (root / f'hqdn3d-noise-{depth}.y4m').write_bytes(data)

# Small odd-sized spatial/temporal grid catches first-column rounding drift.
data = bytearray(b'YUV4MPEG2 W5 H3 F25:1 Ip C420\n')
for n in range(8):
    data += b'FRAME\n' + bytes((i*7+n*3)%32+90 for i in range(27))
(root / 'hqdn3d-grid-8.y4m').write_bytes(data)

# Full-range 14-bit grid exposes reference fixed-point intermediate overflow.
data = bytearray(b'YUV4MPEG2 W5 H3 F25:1 Ip C420p14\n')
for n in range(8):
    data += b'FRAME\n'
    for i in range(27):
        value = (16383 if i%2==n%2 else 0) if n%3==0 else ((i*7+n*3)%32+90)*64
        data += value.to_bytes(2,'little')
(root / 'hqdn3d-extreme-14.y4m').write_bytes(data)
