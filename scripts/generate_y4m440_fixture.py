#!/usr/bin/env python3
"""Write a tiny, wholly synthetic vertical-chroma Y4M control without codecs."""
from pathlib import Path
import struct
root = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors"
output = bytearray(b"YUV4MPEG2 W4 H2 F30:1 Ip C440p10\n")
for frame in range(2):
    output.extend(b"FRAME\n")
    for value in range(16):
        output.extend(struct.pack("<H", value * 17 + frame * 31))
(root / "y4m-vertical-chroma-10.y4m").write_bytes(output)

metadata = output.replace(b"F30:1 Ip", b"F30:1 Ip A16:15 XCOLORRANGE=FULL", 1)
(root / "y4m-aspect-full-10.y4m").write_bytes(metadata)
