#!/usr/bin/env python3
"""Generate synthetic Y4M, Matroska V_FFV1+A_AAC, and MP4 avc1+mp4a fixtures for lossless export, crop-lossless, and overlay packet-budget, memory-budget, and stream-selection parity regressions (FVID-0252)."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_ffv1_gray_fixtures import element, uint
from generate_mp4_audio_selection_regressions import (
    build_aac_packet,
    build_esds,
    make_audio_track,
    mp4a_v0_header,
)
from generate_rotated_mp4_decode_export_probe_regressions import (
    build_avc_32x16_ipcm,
    build_rotated_avc_video_track,
)
from generate_vp9_chroma import atom, word


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/lossless-overlay-multitrack-budget-parity"
FIXTURES = ROOT / "tests/fixtures/playback-errors"


def build_y4m_pair() -> tuple[bytes, bytes]:
    # Primary: 96x64 YUV420 (frame_len = 96*64 + 2*48*32 = 9216 bytes)
    primary_header = b"YUV4MPEG2 W96 H64 F2:1 Ip A1:1 C420\n"
    y0 = bytes([40] * (96 * 64))
    uv0 = bytes([128] * (2 * 48 * 32))
    y1 = bytes([80] * (96 * 64))
    uv1 = bytes([128] * (2 * 48 * 32))
    primary = primary_header + b"FRAME\n" + y0 + uv0 + b"FRAME\n" + y1 + uv1

    # Secondary (foreground): 32x32 YUV420 (frame_len = 32*32 + 2*16*16 = 1536 bytes)
    fg_header = b"YUV4MPEG2 W32 H32 F2:1 Ip A1:1 C420\n"
    fy0 = bytes([180] * (32 * 32))
    fuv0 = bytes([96] * (16 * 16)) + bytes([160] * (16 * 16))
    foreground = fg_header + b"FRAME\n" + fy0 + fuv0 + b"FRAME\n" + fy0 + fuv0
    return primary, foreground


def build_multitrack_matroska_ffv1_aac() -> bytes:
    # Track 1: 4x3 V_FFV1 gray (~20 bytes per packet)
    v_packets = [
        (FIXTURES / "ffv1-gray-8-0.packet").read_bytes(),
        (FIXTURES / "ffv1-gray-8-1.packet").read_bytes(),
    ]
    # Track 2: A_AAC mono 48kHz with 512-byte padded packets
    raw_aac = build_aac_packet(100)
    aac_pkt = raw_aac + bytes(512 - len(raw_aac))
    asc = b"\x11\x88"

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
    a_settings = element(
        0xE1,
        element(0xB5, struct.pack(">d", 48000.0)) + uint(0x9F, 1),
    )
    a_track = element(
        0xAE,
        uint(0xD7, 2)
        + uint(0x73C5, 2)
        + uint(0x83, 2)
        + element(0x86, b"A_AAC")
        + element(0x63A2, asc)
        + uint(0x23E383, 21_333_334)
        + a_settings,
    )
    tracks = element(0x1654AE6B, v_track + a_track)
    blocks = (
        uint(0xE7, 0)
        + element(0xA3, b"\x81\x00\x00\x80" + v_packets[0])
        + element(0xA3, b"\x82\x00\x00\x80" + aac_pkt)
        + element(0xA3, b"\x81\x00\x15\x80" + v_packets[1])
        + element(0xA3, b"\x82\x00\x15\x80" + aac_pkt)
    )
    cluster = element(0x1F43B675, blocks)
    return header + element(0x18538067, info + tracks + cluster)


def build_multitrack_mp4_avc_aac() -> bytes:
    # Track 1: 32x16 avc1 I_PCM (~778 bytes per packet)
    # Track 2: mp4a mono 48kHz with 2048-byte padded packets
    avcc, avc_packets = build_avc_32x16_ipcm()
    raw_aac = build_aac_packet(100)
    aac_pkt = raw_aac + bytes(2048 - len(raw_aac))
    aac_packets = [aac_pkt, aac_pkt]

    ftyp = atom(b"ftyp", b"isom\x00\x00\x00\x00isomiso6avc1mp41")
    v_payload = b"".join(avc_packets)
    a_payload = b"".join(aac_packets)
    mdat = atom(b"mdat", v_payload + a_payload)

    v_offset = len(ftyp) + 8
    a_offset = v_offset + len(v_payload)

    asc = b"\x11\x88"
    mp4a_payload = mp4a_v0_header(1, 48000) + atom(b"esds", build_esds(asc))

    v_trak = build_rotated_avc_video_track(
        1, avcc, avc_packets, v_offset, rotation=0, pasp=(1, 1), rate=48000, delta=1024
    )
    a_trak = make_audio_track(2, 3, b"mp4a", mp4a_payload, aac_packets, a_offset, 48000, 1024)

    mvhd = bytearray(100)
    mvhd[12:16] = word(48000)
    mvhd[16:20] = word(2048)
    mvhd[20:24] = word(65536)
    mvhd[24:28] = word(0x01000000)
    for at, val in [(36, 65536), (52, 65536), (68, 1 << 30), (96, 3)]:
        mvhd[at : at + 4] = word(val)
    return ftyp + mdat + atom(b"moov", atom(b"mvhd", bytes(mvhd)) + v_trak + a_trak)


def generate() -> dict[str, bytes]:
    primary_y4m, foreground_y4m = build_y4m_pair()
    files = {
        "lossless-primary.y4m": primary_y4m,
        "lossless-foreground.y4m": foreground_y4m,
        "multitrack-ffv1-aac.mkv": build_multitrack_matroska_ffv1_aac(),
        "multitrack-avc-aac.mp4": build_multitrack_mp4_avc_aac(),
    }
    manifest = {
        "batch_id": "FVID-0252",
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
