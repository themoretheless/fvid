#!/usr/bin/env python3
"""Synthetic four-frame guidance/edge inputs; no external reference tools."""
from pathlib import Path
root = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
root.mkdir(parents=True, exist_ok=True)
frames = [bytes((i * 37 + n * 23 + 101) % 256 for i in range(288)) for n in range(4)]
(root / 'sab-guidance-edges.y4m').write_bytes(
    b'YUV4MPEG2 W16 H12 F25:1 Ip A1:1 C420jpeg\n' +
    b''.join(b'FRAME\n' + data for data in frames))
(root / 'sab-single-pixel.y4m').write_bytes(
    b'YUV4MPEG2 W1 H1 F25:1 Ip A1:1 C420jpeg\n' +
    b''.join(b'FRAME\n' + bytes([64 + n * 10, 128, 128]) for n in range(4)))
