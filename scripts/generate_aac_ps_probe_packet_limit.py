#!/usr/bin/env python3
"""Generate synthetic mono AAC MP4 and Matroska fixtures for FVID-0271 in-band PS probe packet-limit budgets."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_ffv1_gray_fixtures import element, uint
from generate_mp4_audio_selection_regressions import (
    build_aac_pulse_packet,
    build_esds,
    make_audio_track,
    mp4a_v0_header,
)
from generate_vp9_chroma import atom, word


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/aac-ps-probe-packet-limit"


def build_mp4_mono_aac_prefix() -> bytes:
    # Packet 0: 8 bytes (mono AAC pulse frame at 48 kHz)
    # Packet 1: 48 bytes (mono AAC pulse frame + 40 zero trailing bytes after ID_END)
    pkt_0 = build_aac_pulse_packet(140, 1, 7)
    pkt_1 = build_aac_pulse_packet(140, 2, 7) + bytes(40)
    assert len(pkt_0) == 8 and len(pkt_1) == 48

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
    moov = atom(b"moov", mvhd + trak)
    return ftyp + mdat + moov


def build_matroska_mono_aac_prefix() -> bytes:
    pkt_0 = build_aac_pulse_packet(140, 1, 7)
    pkt_1 = build_aac_pulse_packet(140, 2, 7) + bytes(40)
    assert len(pkt_0) == 8 and len(pkt_1) == 48

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
        + element(0x5741, b"fvid-synthetic-aac")
        + element(0x4489, struct.pack(">d", 42.666666666666664)),
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
        + element(0x63A2, b"\x11\x88")
        + a_settings,
    )
    tracks = element(0x1654AE6B, a_track)
    blocks = (
        uint(0xE7, 0)
        + element(0xA3, b"\x81\x00\x00\x80" + pkt_0)
        + element(0xA3, b"\x81\x00\x15\x80" + pkt_1)
    )
    cluster = element(0x1F43B675, blocks)
    return header + element(0x18538067, info + tracks + cluster)


def generate() -> dict[str, bytes]:
    files = {
        "prefix-mono-aac.m4a": build_mp4_mono_aac_prefix(),
        "prefix-mono-aac.mka": build_matroska_mono_aac_prefix(),
    }
    manifest = {
        "generator": "scripts/generate_aac_ps_probe_packet_limit.py",
        "batch_id": "FVID-0271",
        "provenance": "100% synthetic mono AAC-LC 48 kHz MP4 and Matroska fixtures with an 8-byte packet 0 and a 48-byte packet 1; no FFmpeg, network, or private media.",
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
