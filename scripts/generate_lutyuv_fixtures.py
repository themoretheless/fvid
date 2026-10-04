#!/usr/bin/env python3
"""Generate a synthetic full-range LUT source without external codecs."""
from pathlib import Path
root = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
frames = b''.join(b'FRAME\n' + bytes([128] * 17) for _ in range(3))
(root / 'lutyuv-full-range.y4m').write_bytes(
    b'YUV4MPEG2 W3 H3 F2:1 Ip A1:1 C420 XCOLORRANGE=FULL\n' + frames)
