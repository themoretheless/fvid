#!/usr/bin/env python3
"""Synthetic changing luma/chroma for own temporal mixing, without codec tools."""
from pathlib import Path
import struct
root = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
for depth in [8,10]:
    chroma = '420' if depth == 8 else '420p10'
    data = bytearray(f'YUV4MPEG2 W4 H4 F25:1 Ip A1:1 C{chroma} XCOLORRANGE=LIMITED\n'.encode())
    for n in range(8):
        samples = [64+16*n]*16+[100+2*n]*4+[150-3*n]*4
        payload = bytes(samples) if depth == 8 else struct.pack('<24H',*(v*4 for v in samples))
        data += b'FRAME\n' + payload
    (root / f'tmix-ramp-{depth}.y4m').write_bytes(data)
