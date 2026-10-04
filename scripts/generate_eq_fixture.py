#!/usr/bin/env python3
"""Synthetic one-frame equalization ramps; no external codecs."""
from pathlib import Path
root = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
samples = bytes(range(256)) + bytes(reversed(range(256))) + bytes(v ^ 85 for v in range(256))
(root / 'eq-ramp.y4m').write_bytes(b'YUV4MPEG2 W16 H16 F30:1 Ip C444\nFRAME\n'+samples)

# Nine synthetic frames for animated equalization and temporal selection.
data = bytearray(b'YUV4MPEG2 W4 H4 F25:1 Ip A1:1 C420 XCOLORRANGE=LIMITED\n')
for n in range(9):
    data += b'FRAME\n' + bytes([64+8*n])*16 + bytes([100])*4 + bytes([150])*4
(root / 'eq-time-25.y4m').write_bytes(data)
