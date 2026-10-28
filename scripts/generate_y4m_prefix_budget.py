#!/usr/bin/env python3
"""Generate synthetic Matroska V_FFV1 and MP4 avc1 fixtures for Y4M export, trim, concat, and planning prefix packet-budget scoping regressions (FVID-0275)."""

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
OUT = ROOT / "tests/fixtures/playback-errors/y4m-prefix-budget"
FIXTURES = ROOT / "tests/fixtures/playback-errors"


def build_mkv_ffv1(packets: list[bytes]) -> bytes:
    header = element(
        0x1A45DFA3,
        element(0x4282, b"matroska") + uint(0x4287, 4) + uint(0x4285, 2),
    )
    info = element(
        0x1549A966,
        uint(0x2AD7B1, 1_000_000)
        + element(0x4489, struct.pack(">d", float(len(packets) * 21))),
    )
    v_track = element(
        0xAE,
        uint(0xD7, 1)
        + uint(0x73C5, 1)
        + uint(0x83, 1)
        + element(0x86, b"V_FFV1")
        + uint(0x23E383, 21_000_000)
        + element(0xE0, uint(0xB0, 4) + uint(0xBA, 3)),
    )
    tracks = element(0x1654AE6B, v_track)
    blocks = uint(0xE7, 0)
    for idx, pkt in enumerate(packets):
        ts = (idx * 21).to_bytes(2, "big", signed=True)
        blocks += element(0xA3, b"\x81" + ts + b"\x80" + pkt)
    cluster = element(0x1F43B675, blocks)
    return header + element(0x18538067, info + tracks + cluster)


def build_mp4_avc_tail_oversized() -> bytes:
    avcc, avc_packets = build_avc_32x16_ipcm()
    # Packet 0 is 778 bytes; append a 500-byte filler NAL unit (type 12 = 0x0C) to packet 1
    # so packet 1 is 1283 bytes while decoded 32x16 YUV420p frame is 768 bytes.
    filler_nal = b"\x0c" + b"\xff" * 499 + b"\x80"
    pkt1_oversized = avc_packets[1] + len(filler_nal).to_bytes(4, "big") + filler_nal
    return build_single_rotated_mp4(avcc, [avc_packets[0], pkt1_oversized])


def build_mp4_avc_three_frames_tail_oversized() -> bytes:
    avcc, avc_packets = build_avc_32x16_ipcm()
    # Samples 0 and 1 are 778 bytes; sample 2 (IDR copy of packet 0) has a 500-byte filler NAL unit (1283 bytes).
    filler_nal = b"\x0c" + b"\xff" * 499 + b"\x80"
    pkt2_oversized = avc_packets[0] + len(filler_nal).to_bytes(4, "big") + filler_nal
    return build_single_rotated_mp4(
        avcc, [avc_packets[0], avc_packets[1], pkt2_oversized]
    )


def generate() -> dict[str, bytes]:
    ffv1_pkt0 = (FIXTURES / "ffv1-gray-8-0.packet").read_bytes()
    ffv1_pkt1 = (FIXTURES / "ffv1-gray-8-1.packet").read_bytes()
    # Pad tail packet with 200 trailing zero bytes so small packets are ~29 bytes,
    # decoded 4x3 frame is 18 bytes, and oversized tail packet is ~229 bytes.
    ffv1_one_frame = build_mkv_ffv1([ffv1_pkt0])
    ffv1_tail_oversized = build_mkv_ffv1([ffv1_pkt0, ffv1_pkt1 + bytes(200)])
    ffv1_three_frames_tail_oversized = build_mkv_ffv1(
        [ffv1_pkt0, ffv1_pkt0, ffv1_pkt1 + bytes(200)]
    )
    avc_tail_oversized = build_mp4_avc_tail_oversized()
    avc_three_frames_tail_oversized = build_mp4_avc_three_frames_tail_oversized()
    y4m_zero_frames = b"YUV4MPEG2 W4 H4 F25:1 Ip A1:1 C420\n"
    y4m_two_frames = (
        b"YUV4MPEG2 W4 H4 F25:1 Ip A1:1 C420\n"
        + b"FRAME\n"
        + bytes([64] * 16 + [128] * 8)
        + b"FRAME\n"
        + bytes([96] * 16 + [128] * 8)
    )

    files = {
        "ffv1-one-frame.mkv": ffv1_one_frame,
        "ffv1-two-frames-tail-oversized.mkv": ffv1_tail_oversized,
        "ffv1-three-frames-tail-oversized.mkv": ffv1_three_frames_tail_oversized,
        "avc-two-frames-tail-oversized.mp4": avc_tail_oversized,
        "avc-three-frames-tail-oversized.mp4": avc_three_frames_tail_oversized,
        "zero-frames-4x4.y4m": y4m_zero_frames,
        "two-frames-4x4-25fps.y4m": y4m_two_frames,
    }
    manifest = {
        "batch_id": "FVID-0275",
        "generator": "scripts/generate_y4m_prefix_budget.py",
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
