#!/usr/bin/env python3
"""Generate the tiny synthetic clip for delayed cloud-open regression tests."""
from pathlib import Path
output = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/cloud-startup.y4m"
output.write_bytes(b"YUV4MPEG2 W2 H2 F25:1 Ip A1:1 C420jpeg\n" + (b"FRAME\n" + bytes([81, 81, 81, 81, 90, 240])) * 3)

# A short clip crossing the one-MiB source-block boundary.
(output.parent / "streaming-block-boundary.y4m").write_bytes(
    b"YUV4MPEG2 W256 H256 F25:1 Ip A1:1 C420jpeg\n"
    + (b"FRAME\n" + bytes([81]) * (256 * 256)
       + bytes([90]) * (128 * 128) + bytes([240]) * (128 * 128)) * 12)
