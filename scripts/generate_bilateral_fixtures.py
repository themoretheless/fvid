#!/usr/bin/env python3
"""Generate short synthetic bilateral-noise-edge video without external tools."""
from pathlib import Path
root = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors"
frame = bytes([60, 64, 60, 64, 200, 204, 200, 204] * 4) + bytes([128] * 16)
(root / "bilateral-noise-edge.y4m").write_bytes(
    b"YUV4MPEG2 W8 H4 F2:1 Ip A1:1 C420jpeg\n" + (b"FRAME\n" + frame) * 3)
