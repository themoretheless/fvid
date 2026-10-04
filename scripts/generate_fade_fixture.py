#!/usr/bin/env python3
"""Six synthetic planar frames for own frame-count fade acceptance."""
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
data = bytearray(b"YUV4MPEG2 W4 H4 F25:1 Ip A1:1 C420 XCOLORRANGE=LIMITED\n")
for frame in range(6):
    data += b"FRAME\n" + bytes([64 + frame * 16]) * 16 + bytes([100]) * 4 + bytes([150]) * 4
(ROOT / "tests/fixtures/playback-errors/fade-six-frames.y4m").write_bytes(data)

tie = bytearray(b"YUV4MPEG2 W4 H4 F25:1 Ip A1:1 C420 XCOLORRANGE=LIMITED\n")
for frame in range(3):
    tie += b"FRAME\n" + bytes([64]) * 16 + bytes([129]) * 4 + bytes([131]) * 4
(ROOT / "tests/fixtures/playback-errors/fade-chroma-tie.y4m").write_bytes(tie)
