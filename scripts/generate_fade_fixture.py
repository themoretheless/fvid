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

# Independent 10-bit neutral samples; colored fade endpoints have known BT.601 codes.
import struct
colored = bytearray(b"YUV4MPEG2 W4 H4 F25:1 Ip A1:1 C444p10 XCOLORRANGE=LIMITED\n")
for frame in range(3):
    colored += b"FRAME\n" + struct.pack("<48H", *([256] * 16 + [512] * 32))
(ROOT / "tests/fixtures/playback-errors/fade-colour-10.y4m").write_bytes(colored)

# Nine frames make mixed frame/time gates and temporal selection observable.
for suffix, rate in [("25", "25:1"), ("ntsc", "30000:1001")]:
    timed = bytearray(f"YUV4MPEG2 W4 H4 F{rate} Ip A1:1 C420 XCOLORRANGE=LIMITED\n".encode())
    for frame in range(9):
        timed += b"FRAME\n" + bytes([64 + frame * 8]) * 16 + bytes([100]) * 4 + bytes([150]) * 4
    (ROOT / f"tests/fixtures/playback-errors/fade-time-{suffix}.y4m").write_bytes(timed)
