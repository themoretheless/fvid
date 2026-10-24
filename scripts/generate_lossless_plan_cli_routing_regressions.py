#!/usr/bin/env python3
"""Generate tiny synthetic Y4M fixtures for lossless plan and CLI routing regressions."""

from __future__ import annotations

import pathlib


ROOT = pathlib.Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tests" / "fixtures" / "playback-errors"


def build_y4m(width: int, height: int, luma_values: list[int]) -> bytes:
    header = f"YUV4MPEG2 W{width} H{height} F25:1 Ip A1:1 C420\n".encode("ascii")
    chroma_len = (width // 2) * (height // 2)
    frames = bytearray(header)
    for luma in luma_values:
        frames.extend(b"FRAME\n")
        frames.extend(bytes([luma]) * (width * height))
        frames.extend(bytes([128]) * chroma_len)
        frames.extend(bytes([128]) * chroma_len)
    return bytes(frames)


def generate(output_dir: pathlib.Path = FIXTURES) -> dict[str, pathlib.Path]:
    output_dir.mkdir(parents=True, exist_ok=True)
    main_path = output_dir / "lossless-plan-routing-main.y4m"
    overlay_path = output_dir / "lossless-plan-routing-overlay.y4m"

    main_path.write_bytes(build_y4m(4, 4, [24, 88, 152]))
    overlay_path.write_bytes(build_y4m(2, 2, [210, 140, 70]))

    return {
        "main": main_path,
        "overlay": overlay_path,
    }


if __name__ == "__main__":
    written = generate()
    for name, path in written.items():
        print(f"{name}: {path} ({path.stat().st_size} bytes)")
