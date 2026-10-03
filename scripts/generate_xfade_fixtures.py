#!/usr/bin/env python3
"""Generate tiny owned cross-fade reader regression inputs, without external tools."""
from pathlib import Path

root = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors"
header = b"YUV4MPEG2 W2 H2 F3:1 Ip A1:1 C420\n"
# A 2x2 4:2:0 frame needs six bytes. The marker is valid; only its payload is short.
(root / "xfade-secondary-truncated.y4m").write_bytes(header + b"FRAME\n" + bytes([10, 20]))
for name, values in [("xfade-primary", [0, 0]), ("xfade-secondary", [100, 200, 240])]:
    data = b"YUV4MPEG2 W2 H2 F2:1 Ip A1:1 C420\n"
    for value in values:
        data += b"FRAME\n" + bytes([value]) * 6
    (root / f"{name}.y4m").write_bytes(data)
