#!/usr/bin/env python3
"""Tiny synthetic planar hue controls; no external codecs."""
from pathlib import Path
import struct
root = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
for depth in [8, 10]:
    values = [16, 64, 128, 235, 160, 96]
    payload = bytes(values) if depth == 8 else b''.join(struct.pack('<H', v * 4) for v in values)
    chroma = '420jpeg' if depth == 8 else '420p10'
    (root / f'hue-{depth}.y4m').write_bytes(f'YUV4MPEG2 W2 H2 F30:1 Ip C{chroma}\n'.encode()+b'FRAME\n'+payload)
