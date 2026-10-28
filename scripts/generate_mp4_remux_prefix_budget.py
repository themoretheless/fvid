#!/usr/bin/env python3
"""Generate synthetic MP4 fixtures for FVID-0274 MP4 Matroska remux/concat prefix & stream packet-byte budgets and dynamic-description decode-audio option forwarding."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_mp4_audio_selection_regressions import (
    build_aac_pulse_packet,
    build_esds,
    make_audio_track,
    mp4a_v0_header,
)
from generate_vp9_chroma import atom, word


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/mp4-remux-prefix-budget"


def build_single_track_mp4(packets: list[bytes]) -> bytes:
    mdat_payload = b"".join(packets)
    ftyp = atom(b"ftyp", b"isom" + word(0x200) + b"isommp42")
    mdat_offset = len(ftyp) + 8
    mdat = atom(b"mdat", mdat_payload)

    esds = atom(b"esds", build_esds(b"\x11\x88"))
    mp4a_payload = mp4a_v0_header(1, 48000) + esds
    trak = make_audio_track(
        1, 3, b"mp4a", mp4a_payload, packets, mdat_offset, 48000, 1024
    )
    mvhd = atom(
        b"mvhd",
        bytes([0, 0, 0, 0])
        + word(0)
        + word(0)
        + word(48000)
        + word(1024 * len(packets))
        + word(0x00010000)
        + struct.pack(">H", 0x0100)
        + bytes(10)
        + struct.pack(">9I", 0x00010000, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000)
        + bytes(24)
        + word(2),
    )
    moov = atom(b"moov", mvhd + trak)
    return ftyp + mdat + moov


def build_dual_track_mp4(packets_0: list[bytes], packets_1: list[bytes]) -> bytes:
    payload_0 = b"".join(packets_0)
    payload_1 = b"".join(packets_1)
    ftyp = atom(b"ftyp", b"isom" + word(0x200) + b"isommp42")
    offset_0 = len(ftyp) + 8
    offset_1 = offset_0 + len(payload_0)
    mdat = atom(b"mdat", payload_0 + payload_1)

    esds = atom(b"esds", build_esds(b"\x11\x88"))
    mp4a_payload = mp4a_v0_header(1, 48000) + esds
    trak_0 = make_audio_track(
        1, 3, b"mp4a", mp4a_payload, packets_0, offset_0, 48000, 1024
    )
    trak_1 = make_audio_track(
        2, 3, b"mp4a", mp4a_payload, packets_1, offset_1, 48000, 1024
    )
    mvhd = atom(
        b"mvhd",
        bytes([0, 0, 0, 0])
        + word(0)
        + word(0)
        + word(48000)
        + word(1024 * max(len(packets_0), len(packets_1)))
        + word(0x00010000)
        + struct.pack(">H", 0x0100)
        + bytes(10)
        + struct.pack(">9I", 0x00010000, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000)
        + bytes(24)
        + word(3),
    )
    moov = atom(b"moov", mvhd + trak_0 + trak_1)
    return ftyp + mdat + moov


def build_dynamic_stsd_mp4(pkt_0: bytes, pkt_1: bytes) -> bytes:
    mdat_payload = pkt_0 + pkt_1
    ftyp = atom(b"ftyp", b"isom" + word(0x200) + b"isommp42")
    offset_0 = len(ftyp) + 8
    offset_1 = offset_0 + len(pkt_0)
    mdat = atom(b"mdat", mdat_payload)

    esds_1 = atom(b"esds", build_esds(b"\x11\x88"))
    # Second entry uses 44.1 kHz ASC (0x1208) so ensure_constant_description() returns Err
    esds_2 = atom(b"esds", build_esds(b"\x12\x08"))
    entry_1 = atom(b"mp4a", mp4a_v0_header(1, 48000) + esds_1)
    entry_2 = atom(b"mp4a", mp4a_v0_header(1, 44100) + esds_2)
    stsd = atom(b"stsd", word(0, 2) + entry_1 + entry_2)
    stts = atom(b"stts", word(0, 1, 2, 1024))
    # Chunk 1 has 1 sample with description 1; chunk 2 has 1 sample with description 2
    stsc = atom(b"stsc", word(0, 2, 1, 1, 1, 2, 1, 2))
    stsz = atom(b"stsz", word(0, 0, 2, len(pkt_0), len(pkt_1)))
    stco = atom(b"stco", word(0, 2, offset_0, offset_1))
    stbl = atom(b"stbl", stsd + stts + stsc + stsz + stco)
    dinf = atom(b"dinf", atom(b"dref", word(0, 1) + atom(b"url ", word(1))))
    smhd = atom(b"smhd", word(0, 0))
    mdhd = bytearray(24)
    mdhd[12:20] = word(48000, 2048)
    mdia = atom(
        b"mdia",
        atom(b"mdhd", bytes(mdhd))
        + atom(b"hdlr", word(0, 0) + b"soun" + bytes(12))
        + atom(b"minf", smhd + dinf + stbl),
    )
    tkhd = bytearray(84)
    tkhd[:4] = word(3)
    tkhd[12:16] = word(1)
    tkhd[20:24] = word(2048)
    tkhd[36:38] = struct.pack(">H", 0x0100)
    tkhd[40:44] = tkhd[56:60] = word(65536)
    tkhd[72:76] = word(0x40000000)
    trak = atom(b"trak", atom(b"tkhd", bytes(tkhd)) + mdia)
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


def generate() -> dict[str, bytes]:
    pkt_small_0 = build_aac_pulse_packet(140, 1, 7)
    pkt_small_1 = build_aac_pulse_packet(140, 2, 7)
    pkt_large = build_aac_pulse_packet(140, 3, 7) + bytes(40)
    assert len(pkt_small_0) == 8 and len(pkt_small_1) == 8 and len(pkt_large) == 48

    files = {
        "single-mixed.m4a": build_single_track_mp4([pkt_small_0, pkt_large]),
        "dual-track-mixed.m4a": build_dual_track_mp4(
            [pkt_small_0, pkt_small_1], [pkt_large, pkt_large]
        ),
        "dynamic-stsd-mixed.m4a": build_dynamic_stsd_mp4(pkt_small_0, pkt_large),
    }
    manifest = {
        "generator": "scripts/generate_mp4_remux_prefix_budget.py",
        "batch_id": "FVID-0274",
        "provenance": "100% synthetic mono AAC-LC 48 kHz MP4 fixtures (single-mixed.m4a has an 8-byte sample 0 and 48-byte sample 1; dual-track-mixed.m4a has two 8-byte samples on track 0 and two 48-byte samples on track 1; dynamic-stsd-mixed.m4a has two stsd entries with an 8-byte sample 0 and 48-byte sample 1); no FFmpeg, network, or private media.",
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
