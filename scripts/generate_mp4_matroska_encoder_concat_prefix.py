#!/usr/bin/env python3
"""Generate synthetic MP4 fixtures for FVID-0272 MP4-to-Matroska remux/concat ©too encoder tag preservation and concat prefix packet-byte budgets."""

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
OUT = ROOT / "tests/fixtures/playback-errors/mp4-matroska-encoder-concat-prefix"


def itunes_text_tag(fourcc: bytes, text: str) -> bytes:
    raw = text.encode("utf-8")
    data_box = atom(b"data", word(1) + word(0) + raw)
    return struct.pack(">I", 8 + len(data_box)) + fourcc + data_box


def build_mp4_aac_segment(packets: list[bytes], title: str, encoder: str) -> bytes:
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
    ilst = atom(
        b"ilst",
        itunes_text_tag(b"\xa9nam", title)
        + itunes_text_tag(b"\xa9too", encoder),
    )
    hdlr = atom(b"hdlr", word(0) + word(0) + b"mdir" + b"appl" + word(0) + word(0) + b"\x00")
    meta = atom(b"meta", word(0) + hdlr + ilst)
    udta = atom(b"udta", meta)
    moov = atom(b"moov", mvhd + trak + udta)
    return ftyp + mdat + moov


def generate() -> dict[str, bytes]:
    pkt_small_a = build_aac_pulse_packet(140, 1, 7)
    pkt_small_b = build_aac_pulse_packet(140, 2, 7)
    pkt_large_b = build_aac_pulse_packet(140, 3, 7) + bytes(40)
    assert len(pkt_small_a) == 8 and len(pkt_small_b) == 8 and len(pkt_large_b) == 48

    files = {
        "segment-a.m4a": build_mp4_aac_segment(
            [pkt_small_a, pkt_small_b],
            "MP4 Remux And Concat Title",
            "fvid-mp4-remux-encoder 2.0",
        ),
        "segment-b.m4a": build_mp4_aac_segment(
            [pkt_small_a, pkt_large_b],
            "MP4 Second Segment",
            "fvid-mp4-second-encoder 2.0",
        ),
    }
    manifest = {
        "generator": "scripts/generate_mp4_matroska_encoder_concat_prefix.py",
        "batch_id": "FVID-0272",
        "provenance": "100% synthetic mono AAC-LC 48 kHz MP4 segments with iTunes ©nam/©too tags, where segment-a has two 8-byte packets and segment-b has an 8-byte packet 0 followed by a 48-byte packet 1; no FFmpeg, network, or private media.",
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
