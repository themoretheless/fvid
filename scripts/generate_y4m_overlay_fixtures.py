#!/usr/bin/env python3
"""Generate synthetic overlay clocks/pixels; no external codec or private media."""
from pathlib import Path
root = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors"
main = bytearray(b"YUV4MPEG2 W4 H4 F4:1 Ip A1:1 C420jpeg\n")
for index in range(6):
    main.extend(b"FRAME\n" + bytes([10 + index]) * 16 + bytes([128]) * 8)
foreground = bytearray(b"YUV4MPEG2 W2 H2 F2:1 Ip A1:1 C420jpeg\n")
for y, u, v in [(50, 80, 160), (100, 90, 170)]:
    foreground.extend(b"FRAME\n" + bytes([y]) * 4 + bytes([u, v]))
(root / "overlay-primary-clock.y4m").write_bytes(main)
(root / "overlay-secondary-clock.y4m").write_bytes(foreground)
