#!/usr/bin/env python3
"""Synthetic three-frame high-depth chroma rotation videos, no media dependencies."""
from pathlib import Path
root = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors"
for depth in (12, 16):
    center = 1 << (depth - 1)
    samples = [16, 100, 200, 300, 400, 500, 600, 700, 800] + [center + 100] * 9 + [center - 200] * 9
    payload = b"".join(sample.to_bytes(2, "little") for sample in samples)
    header = f"YUV4MPEG2 W3 H3 F2:1 Ip A1:1 C444p{depth}\n".encode()
    (root / f"hue-chroma-{depth}.y4m").write_bytes(header + (b"FRAME\n" + payload) * 3)
