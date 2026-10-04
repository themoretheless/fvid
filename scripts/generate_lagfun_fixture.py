#!/usr/bin/env python3
"""Synthetic bright-to-dark frames for persistent lagfun acceptance."""
from pathlib import Path
import struct
root = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
for depth in [8,10]:
    chroma = '420' if depth == 8 else '420p10'
    data = bytearray(f'YUV4MPEG2 W4 H4 F25:1 Ip A1:1 C{chroma} XCOLORRANGE=LIMITED\n'.encode())
    for n in range(8):
        y,u,v = (100,140,200) if n == 0 else (16,64,64)
        samples = ([y]*16+[u]*4+[v]*4)
        if depth == 10:
            payload = struct.pack('<24H',*(s*4 for s in samples))
        else:
            payload = bytes(samples)
        data += b'FRAME\n' + payload
    (root / f'lagfun-dark-{depth}.y4m').write_bytes(data)
