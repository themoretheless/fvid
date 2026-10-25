#!/usr/bin/env python3
"""Generate synthetic Y4M, Matroska V_MPEG4/ISO/AVC, and MP4 avc1 fixtures for Y4M export, concat, and trim packet-budget and limit validation parity regressions (FVID-0253)."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_ffv1_gray_fixtures import element, uint
from generate_rotated_mp4_decode_export_probe_regressions import (
    build_avc_32x16_ipcm,
    build_rotated_avc_video_track,
    build_single_rotated_mp4,
)


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/y4m-export-budget-parity"


def build_y4m_32x16() -> bytes:
    # 32x16 YUV420p (frame_len = 32*16 + 2*16*8 = 768 bytes)
    header = b"YUV4MPEG2 W32 H16 F2:1 Ip A1:1 C420\n"
    y0 = bytes([40] * (32 * 16))
    uv0 = bytes([128] * (2 * 16 * 8))
    y1 = bytes([80] * (32 * 16))
    uv1 = bytes([128] * (2 * 16 * 8))
    return header + b"FRAME\n" + y0 + uv0 + b"FRAME\n" + y1 + uv1


def build_mkv_avc_32x16(avcc: bytes, avc_packets: list[bytes]) -> bytes:
    header = element(
        0x1A45DFA3,
        element(0x4282, b"matroska") + uint(0x4287, 4) + uint(0x4285, 2),
    )
    info = element(
        0x1549A966,
        uint(0x2AD7B1, 1_000_000) + element(0x4489, struct.pack(">d", 1000.0)),
    )
    v_track = element(
        0xAE,
        uint(0xD7, 1)
        + uint(0x73C5, 1)
        + uint(0x83, 1)
        + element(0x86, b"V_MPEG4/ISO/AVC")
        + element(0x63A2, avcc)
        + uint(0x23E383, 500_000_000)
        + element(0xE0, uint(0xB0, 32) + uint(0xBA, 16)),
    )
    tracks = element(0x1654AE6B, v_track)
    blocks = (
        uint(0xE7, 0)
        + element(0xA3, b"\x81\x00\x00\x80" + avc_packets[0])
        + element(0xA3, b"\x81\x01\xF4\x00" + avc_packets[1])
    )
    cluster = element(0x1F43B675, blocks)
    return header + element(0x18538067, info + tracks + cluster)


def generate() -> dict[str, bytes]:
    avcc, avc_packets = build_avc_32x16_ipcm()
    # Each AVC I_PCM packet is 778 bytes, while the decoded 32x16 YUV420 frame is 768 bytes.
    y4m = build_y4m_32x16()
    mp4 = build_single_rotated_mp4(avcc, avc_packets)
    mkv = build_mkv_avc_32x16(avcc, avc_packets)

    files = {
        "video-32x16.y4m": y4m,
        "avc-32x16.mp4": mp4,
        "avc-32x16.mkv": mkv,
    }
    manifest = {
        "batch_id": "FVID-0253",
        "generator": "scripts/generate_y4m_export_budget_parity_regressions.py",
        "sha256": {
            name: hashlib.sha256(data).hexdigest()
            for name, data in sorted(files.items())
        },
    }
    files["manifest.json"] = (
        json.dumps(manifest, indent=2, sort_keys=True) + "\n"
    ).encode("utf-8")
    return files


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)


if __name__ == "__main__":
    main()
