#!/usr/bin/env python3
"""Generate Gaussian-filter acceptance media entirely from synthetic samples."""
from pathlib import Path
root = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors"
frame = bytes([255 if x == 4 else 0 for y in range(4) for x in range(10)]) + bytes([123] * 20)
(root / "gblur-impulse.y4m").write_bytes(b"YUV4MPEG2 W10 H4 F2:1 Ip A1:1 C420jpeg\n" + (b"FRAME\n" + frame) * 3)
