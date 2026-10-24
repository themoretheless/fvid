#!/usr/bin/env python3
"""Generate tiny synthetic Y4M and SRT fixtures for Y4M concat/trim/export and burn/xfade control regressions."""

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


def build_srt() -> bytes:
    return (
        "1\n"
        "00:00:00,000 --> 00:00:00,080\n"
        "FVID\n\n"
    ).encode("utf-8")


def generate(output_dir: pathlib.Path = FIXTURES) -> dict[str, pathlib.Path]:
    output_dir.mkdir(parents=True, exist_ok=True)
    primary_path = output_dir / "y4m-burn-xfade-primary.y4m"
    secondary_path = output_dir / "y4m-burn-xfade-secondary.y4m"
    subs_path = output_dir / "y4m-burn-xfade.srt"

    primary_path.write_bytes(build_y4m(32, 16, [24, 96, 168]))
    secondary_path.write_bytes(build_y4m(32, 16, [200, 128, 56]))
    subs_path.write_bytes(build_srt())

    return {
        "primary": primary_path,
        "secondary": secondary_path,
        "subs": subs_path,
    }


if __name__ == "__main__":
    written = generate()
    for name, path in written.items():
        print(f"{name}: {path} ({path.stat().st_size} bytes)")
