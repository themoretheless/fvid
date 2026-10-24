#!/usr/bin/env python3
"""Generate synthetic Y4M fixtures for lossless planner and aspect regressions."""

from __future__ import annotations

import pathlib

ROOT = pathlib.Path(__file__).resolve().parents[1]
FIXTURE_DIR = ROOT / "tests" / "fixtures" / "playback-errors" / "lossless-plan"


def build_y4m(width: int = 8, height: int = 4) -> bytes:
    header = f"YUV4MPEG2 W{width} H{height} F25:1 Ip A1:1 C420jpeg\n".encode("ascii")
    y_plane = bytes((16 + (x * 13 + y * 29) % 200) for y in range(height) for x in range(width))
    chroma_w = (width + 1) // 2
    chroma_h = (height + 1) // 2
    cb_plane = bytes([120] * (chroma_w * chroma_h))
    cr_plane = bytes([136] * (chroma_w * chroma_h))
    return header + b"FRAME\n" + y_plane + cb_plane + cr_plane


def generate(out_dir: pathlib.Path = FIXTURE_DIR) -> dict[str, pathlib.Path]:
    out_dir.mkdir(parents=True, exist_ok=True)
    path = out_dir / "source-8x4.y4m"
    path.write_bytes(build_y4m(8, 4))
    return {"source-8x4.y4m": path}


if __name__ == "__main__":
    generate()
