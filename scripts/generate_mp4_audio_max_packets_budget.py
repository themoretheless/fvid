#!/usr/bin/env python3
"""Generate synthetic fixtures for FVID-0270 unedited MP4 audio max_packets packet-byte and controlled-memory budget admission."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_mp4_audio_selection_regressions import (
    make_audio_track,
    sowt_v0_header,
)
from generate_vp9_chroma import atom, word


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/mp4-audio-max-packets-budget"


def build_mp4_sowt_prefix_m4a() -> bytes:
    # Packet 0: 16 bytes (8 mono s16le samples at 8000 Hz = 1 ms)
    # Packet 1: 256 bytes (128 mono s16le samples at 8000 Hz = 16 ms)
    p0 = b"".join(struct.pack("<h", 2048 if (i & 1) == 0 else -2048) for i in range(8))
    p1 = b"".join(struct.pack("<h", 1536 if (i & 1) == 0 else -1536) for i in range(128))
    assert len(p0) == 16 and len(p1) == 256

    mdat_payload = p0 + p1
    ftyp = atom(b"ftyp", b"isom" + word(0x200) + b"isommp42")
    mdat_offset = len(ftyp) + 8
    mdat = atom(b"mdat", mdat_payload)

    sowt_payload = sowt_v0_header(1, 8000)
    stbl = (
        atom(b"stsd", word(0, 1) + atom(b"sowt", sowt_payload))
        + atom(b"stts", word(0, 2, 1, 8, 1, 128))
        + atom(b"stsc", word(0, 1, 1, 2, 1))
        + atom(b"stsz", word(0, 0, 2, len(p0), len(p1)))
        + atom(b"stco", word(0, 1, mdat_offset))
    )
    dinf = atom(b"dinf", atom(b"dref", word(0, 1) + atom(b"url ", word(1))))
    smhd = atom(b"smhd", word(0, 0))
    mdhd = bytearray(24)
    mdhd[12:20] = word(8000, 136)
    mdia = atom(
        b"mdia",
        atom(b"mdhd", bytes(mdhd))
        + atom(b"hdlr", word(0, 0) + b"soun" + bytes(12))
        + atom(b"minf", smhd + dinf + atom(b"stbl", stbl)),
    )
    tkhd = bytearray(84)
    tkhd[:4] = word(3)
    tkhd[12:16] = word(1)
    tkhd[20:24] = word(136)
    tkhd[36:38] = struct.pack(">H", 0x0100)
    tkhd[40:44] = tkhd[56:60] = word(65536)
    tkhd[72:76] = word(0x40000000)
    trak = atom(b"trak", atom(b"tkhd", bytes(tkhd)) + mdia)

    mvhd = atom(
        b"mvhd",
        bytes([0, 0, 0, 0])
        + word(0)
        + word(0)
        + word(8000)
        + word(136)
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
    files = {
        "prefix-sowt.m4a": build_mp4_sowt_prefix_m4a(),
    }
    manifest = {
        "generator": "scripts/generate_mp4_audio_max_packets_budget.py",
        "batch_id": "FVID-0270",
        "provenance": "100% synthetic unedited MP4 sowt PCM (16B packet 0 + 256B packet 1); no FFmpeg, network, or private media.",
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
