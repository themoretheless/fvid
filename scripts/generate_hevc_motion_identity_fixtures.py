#!/usr/bin/env python3
"""Original three-frame atlas for layer/POC motion-operation regression.

No encoder, external library, network or private media. This exercises motion
reference selection, not coded scalable HEVC playback.
"""
from pathlib import Path

root = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors"
out = bytearray(b"YUV4MPEG2 W64 H32 F25:1 Ip A1:1 C420\n")
# Equal POC in layers 0 and 2 must still select distinct luma samples.
for luma in (31, 97, 163):
    out += b"FRAME\n" + bytes([luma]) * 2048 + bytes([128]) * 1024
(root / "hevc-motion-layer-identity.y4m").write_bytes(out)
