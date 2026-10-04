#!/usr/bin/env python3
"""Own synthetic planar frames for animated hue regression, without a codec tool."""
from pathlib import Path
root = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
data = bytearray(b'YUV4MPEG2 W4 H4 F25:1 Ip A1:1 C420 XCOLORRANGE=LIMITED\n')
for n in range(9):
    data += b'FRAME\n' + bytes([64 + 8*n])*16 + bytes([100])*4 + bytes([150])*4
(root / 'hue-time-25.y4m').write_bytes(data)
