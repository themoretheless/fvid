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

(root / "overlay-secondary-empty.y4m").write_bytes(b"YUV4MPEG2 W2 H2 F2:1 Ip A1:1 C420jpeg\n")

# Rates differ below comparison-clock precision; exact index flooring lags.
for name, w, h, rate, count, base in [
    ("overlay-near-clock-primary.y4m", 4, 4, (2147483646, 2147483645), 6, 10),
    ("overlay-near-clock-secondary.y4m", 2, 2, (2147483647, 2147483646), 8, 100),
    ("overlay-tied-clock-primary.y4m", 4, 4, (1000000000, 1), 6, 10),
    ("overlay-tied-clock-secondary.y4m", 2, 2, (2000000000, 1), 16, 100),
]:
    data = bytearray(f"YUV4MPEG2 W{w} H{h} F{rate[0]}:{rate[1]} Ip A1:1 C420jpeg\n".encode())
    for index in range(count):
        data.extend(b"FRAME\n" + bytes([base + index]) * (w * h) + bytes([128]) * (w * h // 2))
    (root / name).write_bytes(data)

# Dedicated reproducer for legacy-compatible chroma placement rounding.
(root / "overlay-unaligned-primary.y4m").write_bytes(main)
(root / "overlay-unaligned-secondary.y4m").write_bytes(foreground)

# Six indexed frames for temporal selection and interval-origin regression.
(root / "framestep-six-frames.y4m").write_bytes(main)
