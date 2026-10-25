#!/usr/bin/env python3
"""Generate synthetic Matroska audio fixtures for FVID-0280 interval-prefix budget admission."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_aac_ps_fixtures import ps as build_ps_bits, sbr as build_sbr_fill
from generate_ffv1_gray_fixtures import element, uint
from generate_he_aac_packet_fixtures import asc as build_he_aac_asc, packet as build_sbr_packet
from generate_mp4_audio_selection_regressions import build_aac_pulse_packet


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/matroska-interval-prefix-budget"


def build_ps_packet() -> bytes:
    ps_bits, _ = build_ps_bits(1, 1, 0)
    sbr_raw = build_sbr_fill(ps_bits, 0)
    return build_sbr_packet(sbr_raw)


def build_matroska_aac_mono_then_ps() -> bytes:
    # 5 packets at 48 kHz (2048 output samples / ~42.667 ms per packet):
    # Packets 0..=3: 8-byte mono AAC pulse packets at 0, 43, 85, 128 ms
    # Packet 4: 31-byte SBR + in-band PS packet at 171 ms (expands to stereo)
    pkt_small = build_aac_pulse_packet(140, 1, 7)
    pkt_ps = build_ps_packet()
    assert len(pkt_small) == 8 and len(pkt_ps) > 24

    asc_bytes = build_he_aac_asc(24000, 48000, 16, "explicit", ps=False)
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
        + element(0x5741, b"fvid-synthetic-mka-interval-aac")
        + element(0x4489, struct.pack(">d", 213.33333333333334)),
    )
    a_settings = element(
        0xE1,
        element(0xB5, struct.pack(">d", 48000.0)) + uint(0x9F, 1),
    )
    a_track = element(
        0xAE,
        uint(0xD7, 1)
        + uint(0x73C5, 1)
        + uint(0x83, 2)
        + element(0x86, b"A_AAC")
        + element(0x63A2, asc_bytes)
        + a_settings,
    )
    tracks = element(0x1654AE6B, a_track)
    timestamps_ms = [0, 43, 85, 128, 171]
    packets = [pkt_small, pkt_small, pkt_small, pkt_small, pkt_ps]
    blocks = uint(0xE7, 0) + b"".join(
        element(0xA3, b"\x81" + struct.pack(">h", ts) + b"\x80" + pkt)
        for ts, pkt in zip(timestamps_ms, packets)
    )
    cluster = element(0x1F43B675, blocks)
    return header + element(0x18538067, info + tracks + cluster)


def build_matroska_pcm_small_then_large() -> bytes:
    # 16-bit little-endian mono PCM at 1000 Hz (2 bytes per sample):
    # Packets 0..=2: 10 samples (20 bytes) each at 0, 10, 20 ms -> covers [0, 30 ms)
    # Packet 3: 40 samples (80 bytes) at 100 ms
    pkt_small = b"".join(struct.pack("<h", (i * 400) - 1800) for i in range(10))
    pkt_large = b"".join(struct.pack("<h", ((i % 10) * 400) - 1800) for i in range(40))
    assert len(pkt_small) == 20 and len(pkt_large) == 80

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
        + element(0x5741, b"fvid-synthetic-mka-interval-pcm")
        + element(0x4489, struct.pack(">d", 140.0)),
    )
    a_settings = element(
        0xE1,
        element(0xB5, struct.pack(">d", 1000.0)) + uint(0x9F, 1) + uint(0x6264, 16),
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
    timestamps_ms = [0, 10, 20, 100]
    packets = [pkt_small, pkt_small, pkt_small, pkt_large]
    blocks = uint(0xE7, 0) + b"".join(
        element(0xA3, b"\x81" + struct.pack(">h", ts) + b"\x80" + pkt)
        for ts, pkt in zip(timestamps_ms, packets)
    )
    cluster = element(0x1F43B675, blocks)
    return header + element(0x18538067, info + tracks + cluster)


def generate() -> dict[str, bytes]:
    files = {
        "matroska-aac-mono-then-ps.mka": build_matroska_aac_mono_then_ps(),
        "matroska-pcm-small-then-large.mka": build_matroska_pcm_small_then_large(),
    }
    manifest = {
        "generator": "scripts/generate_matroska_interval_prefix_budget.py",
        "batch_id": "FVID-0280",
        "provenance": "100% synthetic Matroska A_AAC and A_PCM/INT/LIT fixtures with small early packets and an oversized or in-band PS packet after the presentation interval end; no FFmpeg, network, or private media.",
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
