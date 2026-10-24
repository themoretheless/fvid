#!/usr/bin/env python3
"""Generate synthetic MP4 and Y4M fixtures for custom container/stream metadata tags and stream metadata deletions (FVID-0227)."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_mp4_matroska_remux_rss_regressions import build_two_track_mp4

ROOT = Path(__file__).resolve().parents[1]
SEED_DIR = ROOT / "tests/fixtures/playback-errors"
OUT = SEED_DIR / "mp4-remux-custom-tags"


def build_y4m_two_frames() -> bytes:
    header = b"YUV4MPEG2 W4 H4 F25:1 Ip A1:1 C420\n"
    y0 = bytes([32 + (i % 16) for i in range(16)])
    u0 = bytes([120, 122, 124, 126])
    v0 = bytes([132, 134, 136, 138])
    y1 = bytes([64 + (i % 16) for i in range(16)])
    u1 = bytes([118, 120, 122, 124])
    v1 = bytes([130, 132, 134, 136])
    return header + b"FRAME\n" + y0 + u0 + v0 + b"FRAME\n" + y1 + u1 + v1


def generate() -> dict[str, bytes]:
    files = {
        "two-track-avc.mp4": build_two_track_mp4(),
        "tiny-two-frames.y4m": build_y4m_two_frames(),
    }
    manifest = {
        "batch_id": "FVID-0227",
        "sha256": {name: hashlib.sha256(data).hexdigest() for name, data in files.items()},
    }
    files["manifest.json"] = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode("utf-8")
    return files


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)


if __name__ == "__main__":
    main()
