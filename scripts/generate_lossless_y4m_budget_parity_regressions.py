#!/usr/bin/env python3
"""Generate synthetic Y4M, Matroska V_FFV1, and MP4 avc1 single-track video fixtures for lossless FFV1 export and Y4M trim/concat policy and packet-budget parity regressions (FVID-0259)."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_ffv1_gray_fixtures import element, uint
from generate_rotated_mp4_decode_export_probe_regressions import (
    build_avc_32x16_ipcm,
    build_rotated_avc_video_track,
)
from generate_vp9_chroma import atom, word


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/lossless-y4m-budget-parity"
FIXTURES = ROOT / "tests/fixtures/playback-errors"


def build_y4m_video() -> bytes:
    # 16x16 YUV420 (frame_len = 16*16 + 2*8*8 = 384 bytes)
    header = b"YUV4MPEG2 W16 H16 F2:1 Ip A1:1 C420\n"
    y0 = bytes([(x * 9 + y * 13) % 220 + 16 for y in range(16) for x in range(16)])
    u0 = bytes([(x * 5 + y * 7) % 200 + 16 for y in range(8) for x in range(8)])
    v0 = bytes([(x * 11 + y * 3) % 200 + 16 for y in range(8) for x in range(8)])
    y1 = bytes([(x * 15 + y * 5) % 220 + 16 for y in range(16) for x in range(16)])
    u1 = bytes([(x * 7 + y * 11) % 200 + 16 for y in range(8) for x in range(8)])
    v1 = bytes([(x * 3 + y * 13) % 200 + 16 for y in range(8) for x in range(8)])
    return header + b"FRAME\n" + y0 + u0 + v0 + b"FRAME\n" + y1 + u1 + v1


def build_single_track_matroska_ffv1() -> bytes:
    # Single-track 4x3 V_FFV1 gray (~20 bytes per packet)
    v_packets = [
        (FIXTURES / "ffv1-gray-8-0.packet").read_bytes(),
        (FIXTURES / "ffv1-gray-8-1.packet").read_bytes(),
    ]
    header = element(
        0x1A45DFA3,
        element(0x4282, b"matroska") + uint(0x4287, 4) + uint(0x4285, 2),
    )
    info = element(
        0x1549A966,
        uint(0x2AD7B1, 1_000_000) + element(0x4489, struct.pack(">d", 42.0)),
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
    blocks = (
        uint(0xE7, 0)
        + element(0xA3, b"\x81\x00\x00\x80" + v_packets[0])
        + element(0xA3, b"\x81\x00\x15\x80" + v_packets[1])
    )
    cluster = element(0x1F43B675, blocks)
    return header + element(0x18538067, info + tracks + cluster)


def build_single_track_mp4_avc() -> bytes:
    # Single-track 32x16 avc1 I_PCM (~778 bytes per packet)
    avcc, avc_packets = build_avc_32x16_ipcm()
    ftyp = atom(b"ftyp", b"isom\x00\x00\x00\x00isomiso6avc1mp41")
    v_payload = b"".join(avc_packets)
    mdat = atom(b"mdat", v_payload)
    v_offset = len(ftyp) + 8
    v_trak = build_rotated_avc_video_track(
        1, avcc, avc_packets, v_offset, rotation=0, pasp=(1, 1), rate=48000, delta=1024
    )
    mvhd = bytearray(100)
    mvhd[12:16] = word(48000)
    mvhd[16:20] = word(2048)
    mvhd[20:24] = word(65536)
    mvhd[24:28] = word(0x01000000)
    for at, val in [(36, 65536), (52, 65536), (68, 1 << 30), (96, 2)]:
        mvhd[at : at + 4] = word(val)
    return ftyp + mdat + atom(b"moov", atom(b"mvhd", bytes(mvhd)) + v_trak)


def generate() -> dict[str, bytes]:
    files = {
        "lossless-textured.y4m": build_y4m_video(),
        "single-video-ffv1.mkv": build_single_track_matroska_ffv1(),
        "single-video-avc.mp4": build_single_track_mp4_avc(),
    }
    manifest = {
        "batch_id": "FVID-0259",
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
