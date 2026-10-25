#!/usr/bin/env python3
"""Generate synthetic AAC fixtures for FVID-0276 prefix-scoped source-info negotiation."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_aac_ps_fixtures import ps as build_ps_bits, sbr as build_sbr_fill
from generate_ffv1_gray_fixtures import element, uint
from generate_he_aac_packet_fixtures import asc as build_he_aac_asc, packet as build_sbr_packet
from generate_mp4_audio_selection_regressions import (
    build_aac_pulse_packet,
    build_esds,
    make_audio_track,
    mp4a_v0_header,
)
from generate_vp9_chroma import atom, word


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/aac-source-info-prefix-budget"


def adts_frame(payload: bytes, freq_index: int = 6, channels: int = 1) -> bytes:
    """Wrap a raw AAC data block in a 7-byte MPEG-4 ADTS header (profile=1/AAC-LC)."""
    frame_bytes = 7 + len(payload)
    assert frame_bytes < 8192
    b0 = 0xFF
    b1 = 0xF1  # MPEG-4, layer 0, protection_absent=1
    profile = 1  # AAC-LC (object_type = 2 -> profile = 1)
    b2 = ((profile & 0x3) << 6) | ((freq_index & 0xF) << 2) | ((channels >> 2) & 0x1)
    b3 = ((channels & 0x3) << 6) | ((frame_bytes >> 11) & 0x3)
    b4 = (frame_bytes >> 3) & 0xFF
    b5 = ((frame_bytes & 0x7) << 5) | 0x1F
    b6 = 0xFC  # fullness all 1s, raw_blocks = 0 (1 block)
    return bytes([b0, b1, b2, b3, b4, b5, b6]) + payload


def build_ps_packet() -> bytes:
    ps_bits, _ = build_ps_bits(1, 1, 0)
    sbr_raw = build_sbr_fill(ps_bits, 0)
    return build_sbr_packet(sbr_raw)


def build_mp4_mono_then_ps() -> bytes:
    # Packet 0: mono AAC pulse packet (8 bytes, no PS)
    # Packet 1: mono AAC + SBR + in-band PS (triggers PS stereo expansion)
    pkt_0 = build_aac_pulse_packet(140, 1, 7)
    pkt_1 = build_ps_packet()
    assert len(pkt_0) == 8 and len(pkt_1) > 16

    mdat_payload = pkt_0 + pkt_1
    ftyp = atom(b"ftyp", b"isom" + word(0x200) + b"isommp42")
    mdat_offset = len(ftyp) + 8
    mdat = atom(b"mdat", mdat_payload)

    # Explicit SBR (AOT 5), core 24 kHz, output 48 kHz, mono (channels=1), implicit PS
    asc_bytes = build_he_aac_asc(24000, 48000, 16, "explicit", ps=False)
    esds = atom(b"esds", build_esds(asc_bytes))
    mp4a_payload = mp4a_v0_header(1, 48000) + esds
    trak = make_audio_track(
        1, 3, b"mp4a", mp4a_payload, [pkt_0, pkt_1], mdat_offset, 48000, 2048
    )
    mvhd = atom(
        b"mvhd",
        bytes([0, 0, 0, 0])
        + word(0)
        + word(0)
        + word(48000)
        + word(4096)
        + word(0x00010000)
        + struct.pack(">H", 0x0100)
        + bytes(10)
        + struct.pack(">9I", 0x00010000, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000)
        + bytes(24)
        + word(2),
    )
    moov = atom(b"moov", mvhd + trak)
    return ftyp + mdat + moov


def build_matroska_mono_then_ps() -> bytes:
    pkt_0 = build_aac_pulse_packet(140, 1, 7)
    pkt_1 = build_ps_packet()
    assert len(pkt_0) == 8 and len(pkt_1) > 16

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
        + element(0x5741, b"fvid-synthetic-aac-ps")
        + element(0x4489, struct.pack(">d", 85.33333333333333)),
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
    blocks = (
        uint(0xE7, 0)
        + element(0xA3, b"\x81\x00\x00\x80" + pkt_0)
        + element(0xA3, b"\x81\x00\x2B\x80" + pkt_1)
    )
    cluster = element(0x1F43B675, blocks)
    return header + element(0x18538067, info + tracks + cluster)


def build_adts_mono_then_ps() -> bytes:
    # 24 kHz mono ADTS (freq_index = 6, channels = 1)
    # Frame 0: 8-byte payload (15-byte ADTS frame), mono LC only
    # Frame 1: 35-byte payload (42-byte ADTS frame), SBR + in-band PS (expands to 48 kHz stereo)
    pkt_0 = build_aac_pulse_packet(140, 1, 7)
    pkt_1 = build_ps_packet()
    return adts_frame(pkt_0, freq_index=6, channels=1) + adts_frame(
        pkt_1, freq_index=6, channels=1
    )


def generate() -> dict[str, bytes]:
    files = {
        "mono-then-ps.m4a": build_mp4_mono_then_ps(),
        "mono-then-ps.mka": build_matroska_mono_then_ps(),
        "mono-then-ps.aac": build_adts_mono_then_ps(),
    }
    manifest = {
        "generator": "scripts/generate_aac_source_info_prefix_budget.py",
        "batch_id": "FVID-0276",
        "provenance": "100% synthetic mono AAC-LC / HE-AACv2 fixtures where packet 0 is an 8-byte mono AAC pulse block and packet 1 is a 35-byte SBR+PS block; no FFmpeg, network, or private media.",
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
