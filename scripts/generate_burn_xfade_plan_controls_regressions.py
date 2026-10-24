#!/usr/bin/env python3
"""Generate deterministic synthetic Y4M and SRT fixtures for Batch 218 (`FVID-0225`).

Verifies `plan_burn_subtitles`, `plan_xfade`, `burn-subtitles`, and `xfade`
budget preflight, metadata steps, stream representation, and CLI flag parsing.
"""

from __future__ import annotations

import pathlib

FIXTURE_DIR = (
    pathlib.Path(__file__).resolve().parents[1]
    / "tests"
    / "fixtures"
    / "playback-errors"
    / "burn-xfade-plan-controls"
)


def make_y4m(width: int, height: int, luma_values: list[int]) -> bytes:
    header = f"YUV4MPEG2 W{width} H{height} F2:1 Ip A1:1 C420\n".encode("ascii")
    chroma_size = (width // 2) * (height // 2)
    out = bytearray(header)
    for y_val in luma_values:
        out.extend(b"FRAME\n")
        out.extend(bytes([y_val]) * (width * height))
        out.extend(bytes([128]) * (2 * chroma_size))
    return bytes(out)


def make_srt() -> bytes:
    return (
        "1\n"
        "00:00:00,000 --> 00:00:00,500\n"
        "Hello\n\n"
        "2\n"
        "00:00:00,500 --> 00:00:01,000\n"
        "World\n"
    ).encode("utf-8")


def generate_all(dest_dir: pathlib.Path = FIXTURE_DIR) -> dict[str, bytes]:
    dest_dir.mkdir(parents=True, exist_ok=True)
    files = {
        "burn-main-96x64.y4m": make_y4m(96, 64, [24, 48, 72]),
        "burn-cues.srt": make_srt(),
        "xfade-main-4x4.y4m": make_y4m(4, 4, [16, 32, 48]),
        "xfade-other-4x4.y4m": make_y4m(4, 4, [160, 192, 224]),
    }
    for name, payload in files.items():
        (dest_dir / name).write_bytes(payload)
    return files


if __name__ == "__main__":
    generate_all()
