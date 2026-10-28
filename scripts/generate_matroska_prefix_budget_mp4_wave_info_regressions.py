#!/usr/bin/env python3
"""Generate synthetic fixtures for FVID-0269 Matroska prefix packet-byte budgets and MP4 WAVE INFO writing-application metadata."""

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
from generate_vp9_chroma import atom, word


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/matroska-prefix-budget-mp4-wave-info"


def crc8(data: bytes) -> int:
    crc = 0
    for byte in data:
        crc ^= byte
        for _ in range(8):
            if crc & 0x80:
                crc = ((crc << 1) ^ 0x07) & 0xFF
            else:
                crc = (crc << 1) & 0xFF
    return crc


def crc16_flac(data: bytes) -> int:
    crc = 0
    for byte in data:
        crc ^= byte << 8
        for _ in range(8):
            if crc & 0x8000:
                crc = ((crc << 1) ^ 0x8005) & 0xFFFF
            else:
                crc = (crc << 1) & 0xFFFF
    return crc


def make_flac_constant_frame(frame_number: int, value: int = 2048) -> bytes:
    header_prefix = bytes([0xFF, 0xF8, 0x1A, 0x08, frame_number])
    header = header_prefix + bytes([crc8(header_prefix)])
    subframe = b"\x00" + struct.pack(">h", value)
    body = header + subframe
    return body + struct.pack(">H", crc16_flac(body))


def make_flac_verbatim_frame(frame_number: int, samples: list[int]) -> bytes:
    assert len(samples) == 192
    header_prefix = bytes([0xFF, 0xF8, 0x1A, 0x08, frame_number])
    header = header_prefix + bytes([crc8(header_prefix)])
    subframe = b"\x02" + b"".join(struct.pack(">h", s) for s in samples)
    body = header + subframe
    return body + struct.pack(">H", crc16_flac(body))


def make_streaminfo(
    *,
    min_block: int,
    max_block: int,
    min_frame: int,
    max_frame: int,
    sample_rate: int,
    channels: int,
    bits_per_sample: int,
    total_samples: int,
) -> bytes:
    packed = (
        ((sample_rate & 0xFFFFF) << 44)
        + (((channels - 1) & 0x7) << 41)
        + (((bits_per_sample - 1) & 0x1F) << 36)
        + (total_samples & ((1 << 36) - 1))
    )
    return (
        struct.pack(">HH", min_block, max_block)
        + min_frame.to_bytes(3, "big")
        + max_frame.to_bytes(3, "big")
        + struct.pack(">Q", packed)
        + b"\x00" * 16
    )


def build_matroska_flac_prefix_mka() -> bytes:
    f0 = make_flac_constant_frame(0, value=2048)
    f1 = make_flac_verbatim_frame(1, [2048 if (i & 1) == 0 else -2048 for i in range(192)])
    assert len(f0) == 11 and len(f1) == 393
    si = make_streaminfo(
        min_block=192,
        max_block=192,
        min_frame=len(f0),
        max_frame=len(f1),
        sample_rate=48_000,
        channels=1,
        bits_per_sample=16,
        total_samples=384,
    )
    codec_private = b"fLaC" + bytes([0x80, 0x00, 0x00, 34]) + si

    header = element(
        0x1A45DFA3,
        uint(0x4286, 1)
        + uint(0x42F7, 1)
        + uint(0x42F2, 4)
        + uint(0x42F3, 8)
        + element(0x4282, b"matroska")
        + uint(0x4287, 4)
        + uint(0x4285, 2),
    )
    info = element(
        0x1549A966,
        uint(0x2AD7B1, 1_000_000)
        + element(0x4D80, b"fvid-synthetic")
        + element(0x5741, b"fvid-synthetic-mka")
        + element(0x7BA9, b"Matroska Prefix FLAC")
        + element(0x4489, struct.pack(">d", 8.0)),
    )
    a_settings = element(
        0xE1,
        element(0xB5, struct.pack(">d", 48000.0)) + uint(0x9F, 1) + uint(0x6264, 16),
    )
    a_track = element(
        0xAE,
        uint(0xD7, 1)
        + uint(0x73C5, 1)
        + uint(0x83, 2)
        + element(0x86, b"A_FLAC")
        + element(0x63A2, codec_private)
        + a_settings,
    )
    tracks = element(0x1654AE6B, a_track)
    blocks = (
        uint(0xE7, 0)
        + element(0xA3, b"\x81\x00\x00\x80" + f0)
        + element(0xA3, b"\x81\x00\x04\x80" + f1)
    )
    cluster = element(0x1F43B675, blocks)
    return header + element(0x18538067, info + tracks + cluster)


def build_matroska_pcm_prefix_mka() -> bytes:
    # Packet 0: 16 bytes (8 mono s16le samples at 8000 Hz = 1 ms)
    # Packet 1: 128 bytes (64 mono s16le samples at 8000 Hz = 8 ms)
    p0 = b"".join(struct.pack("<h", 2048 if (i & 1) == 0 else -2048) for i in range(8))
    p1 = b"".join(struct.pack("<h", 1536 if (i & 1) == 0 else -1536) for i in range(64))
    assert len(p0) == 16 and len(p1) == 128

    header = element(
        0x1A45DFA3,
        uint(0x4286, 1)
        + uint(0x42F7, 1)
        + uint(0x42F2, 4)
        + uint(0x42F3, 8)
        + element(0x4282, b"matroska")
        + uint(0x4287, 4)
        + uint(0x4285, 2),
    )
    info = element(
        0x1549A966,
        uint(0x2AD7B1, 1_000_000)
        + element(0x4D80, b"fvid-synthetic")
        + element(0x5741, b"fvid-synthetic-pcm")
        + element(0x4489, struct.pack(">d", 9.0)),
    )
    a_settings = element(
        0xE1,
        element(0xB5, struct.pack(">d", 8000.0)) + uint(0x9F, 1) + uint(0x6264, 16),
    )
    a_track = element(
        0xAE,
        uint(0xD7, 1)
        + uint(0x73C5, 1)
        + uint(0x83, 2)
        + element(0x86, b"A_PCM/INT/LIT")
        + a_settings,
    )
    tracks = element(0x1654AE6B, a_track)
    blocks = (
        uint(0xE7, 0)
        + element(0xA3, b"\x81\x00\x00\x80" + p0)
        + element(0xA3, b"\x81\x00\x01\x80" + p1)
    )
    cluster = element(0x1F43B675, blocks)
    return header + element(0x18538067, info + tracks + cluster)


def build_mp4_aac_with_too_tag() -> bytes:
    pkt_0 = build_aac_packet(100)
    pkt_1 = build_aac_packet(115)
    mdat_payload = pkt_0 + pkt_1
    ftyp = atom(b"ftyp", b"isom" + word(0x200) + b"isommp42")
    mdat_offset = len(ftyp) + 8
    mdat = atom(b"mdat", mdat_payload)

    esds = atom(b"esds", build_esds(b"\x11\x88"))
    mp4a_payload = mp4a_v0_header(1, 48000) + esds
    trak = make_audio_track(
        1, 3, b"mp4a", mp4a_payload, [pkt_0, pkt_1], mdat_offset, 48000, 1024
    )
    mvhd = atom(
        b"mvhd",
        bytes([0, 0, 0, 0])
        + word(0)
        + word(0)
        + word(48000)
        + word(2048)
        + word(0x00010000)
        + struct.pack(">H", 0x0100)
        + bytes(10)
        + struct.pack(">9I", 0x00010000, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000)
        + bytes(24)
        + word(2),
    )

    def itunes_text_tag(fourcc: bytes, text: str) -> bytes:
        raw = text.encode("utf-8")
        data_box = atom(b"data", word(1) + word(0) + raw)
        return struct.pack(">I", 8 + len(data_box)) + fourcc + data_box

    ilst = atom(
        b"ilst",
        itunes_text_tag(b"\xa9nam", "MP4 Tagged Track")
        + itunes_text_tag(b"\xa9too", "fvid-mp4-muxer 1.0"),
    )
    hdlr = atom(b"hdlr", word(0) + word(0) + b"mdir" + b"appl" + word(0) + word(0) + b"\x00")
    meta = atom(b"meta", word(0) + hdlr + ilst)
    udta = atom(b"udta", meta)
    moov = atom(b"moov", mvhd + trak + udta)
    return ftyp + mdat + moov


def generate() -> dict[str, bytes]:
    files = {
        "prefix-flac.mka": build_matroska_flac_prefix_mka(),
        "prefix-pcm.mka": build_matroska_pcm_prefix_mka(),
        "tagged-encoder.m4a": build_mp4_aac_with_too_tag(),
    }
    manifest = {
        "generator": "scripts/generate_matroska_prefix_budget_mp4_wave_info_regressions.py",
        "batch_id": "FVID-0269",
        "provenance": "100% synthetic Matroska A_FLAC (11B + 393B packets), Matroska A_PCM/INT/LIT (16B + 128B packets), and MP4 AAC with iTunes ©nam/©too tags; no FFmpeg, network, or private media.",
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
