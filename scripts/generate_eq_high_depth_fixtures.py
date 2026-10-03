#!/usr/bin/env python3
"""Synthetic full-precision equalizer video controls; no external tools."""
from pathlib import Path
root = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors"
for depth in (12, 16):
    count = 1 << depth
    samples = [0, 1, 2, count // 4, count - 1, 7, 17, 257, 1000] + [count // 2] * 18
    payload = b"".join(sample.to_bytes(2, "little") for sample in samples)
    header = f"YUV4MPEG2 W3 H3 F2:1 Ip A1:1 C444p{depth}\n".encode()
    (root / f"eq-precision-{depth}.y4m").write_bytes(header + (b"FRAME\n" + payload) * 3)
