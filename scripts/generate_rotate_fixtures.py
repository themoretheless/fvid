#!/usr/bin/env python3
"""Synthetic YUV444 frames for owned rotation; no external media tools."""
from pathlib import Path
root = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors"
frame = bytes([1,2,3,4,5,6]) + bytes([128] * 12)
(root / "rotate-grid.y4m").write_bytes(b"YUV4MPEG2 W3 H2 F2:1 Ip A1:1 C444\n" + (b"FRAME\n" + frame) * 3)
