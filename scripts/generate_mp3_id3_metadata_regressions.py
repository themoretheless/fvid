#!/usr/bin/env python3
"""Generate synthetic bare MP3 fixtures with ID3v2.3/ID3v2.4 and ID3v1 tags for FVID-0230."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/mp3-id3-metadata"


def synchsafe(value: int) -> bytes:
    return bytes(
        [
            (value >> 21) & 0x7F,
            (value >> 14) & 0x7F,
            (value >> 7) & 0x7F,
            value & 0x7F,
        ]
    )


def id3v24_text_frame(frame_id: str, text: str, encoding: int = 3) -> bytes:
    assert len(frame_id) == 4
    if encoding == 3:
        payload = bytes([3]) + text.encode("utf-8") + b"\x00"
    elif encoding == 1:
        payload = bytes([1]) + b"\xff\xfe" + text.encode("utf-16-le")
    elif encoding == 2:
        payload = bytes([2]) + text.encode("utf-16-be")
    else:
        payload = bytes([0]) + text.encode("latin-1")
    return frame_id.encode("ascii") + synchsafe(len(payload)) + b"\x00\x00" + payload


def id3v24_comm_frame(language: str, description: str, text: str) -> bytes:
    lang = language.encode("ascii")[:3].ljust(3, b" ")
    payload = (
        bytes([3])
        + lang
        + description.encode("utf-8")
        + b"\x00"
        + text.encode("utf-8")
    )
    return b"COMM" + synchsafe(len(payload)) + b"\x00\x00" + payload


def id3v24_txxx_frame(description: str, value: str) -> bytes:
    payload = (
        bytes([3])
        + description.encode("utf-8")
        + b"\x00"
        + value.encode("utf-8")
    )
    return b"TXXX" + synchsafe(len(payload)) + b"\x00\x00" + payload


def build_id3v24_tag(frames: list[bytes], padding: int = 8) -> bytes:
    body = b"".join(frames) + (b"\x00" * padding)
    return b"ID3\x04\x00\x00" + synchsafe(len(body)) + body


def build_id3v1_tag(
    *,
    title: str,
    artist: str,
    album: str,
    year: str,
    comment: str,
    track: int,
    genre: int,
) -> bytes:
    buf = bytearray(128)
    buf[0:3] = b"TAG"
    buf[3:33] = title.encode("ascii")[:30].ljust(30, b"\x00")
    buf[33:63] = artist.encode("ascii")[:30].ljust(30, b"\x00")
    buf[63:93] = album.encode("ascii")[:30].ljust(30, b"\x00")
    buf[93:97] = year.encode("ascii")[:4].ljust(4, b"\x00")
    buf[97:125] = comment.encode("ascii")[:28].ljust(28, b"\x00")
    buf[125] = 0
    buf[126] = track & 0xFF
    buf[127] = genre & 0xFF
    return bytes(buf)


def generate() -> dict[str, bytes]:
    raw_mp3 = (ROOT / "tests/fixtures/playback-errors/mpeg-clock-metadata/mp3-48000.mp3").read_bytes()

    id3v24_tag = build_id3v24_tag(
        [
            id3v24_text_frame("TIT2", "Synthetic MP3 Tone"),
            id3v24_text_frame("TPE1", "FVid Audio Lab", encoding=1),
            id3v24_text_frame("TALB", "MPEG Metadata Suite"),
            id3v24_text_frame("TDRC", "2026-10-23"),
            id3v24_comm_frame("eng", "", "Synthetic ID3v2.4 Comment"),
            id3v24_text_frame("TCON", "Electronic"),
            id3v24_text_frame("TCOP", "Public Domain"),
            id3v24_text_frame("TSSE", "fvid-synthetic-mp3"),
            id3v24_text_frame("TRCK", "4/10"),
            id3v24_text_frame("TPE2", "FVid Orchestra"),
            id3v24_text_frame("TPOS", "1/2"),
            id3v24_text_frame("TPUB", "FVid Records"),
            id3v24_text_frame("TIT3", "Lossless ID3 verification"),
            id3v24_text_frame("TLAN", "eng"),
            id3v24_txxx_frame("CUSTOM_TAG", "custom-mp3-value"),
        ]
    )

    id3v1_tag = build_id3v1_tag(
        title="ID3v1 Fallback Title",
        artist="ID3v1 Artist",
        album="ID3v1 Album",
        year="2025",
        comment="ID3v1 Comment",
        track=7,
        genre=12,
    )

    files = {
        "tagged-id3v24.mp3": id3v24_tag + raw_mp3 + id3v1_tag,
        "tagged-id3v1.mp3": raw_mp3 + id3v1_tag,
    }
    manifest = {
        "generator": "scripts/generate_mp3_id3_metadata_regressions.py",
        "batch_id": "FVID-0230",
        "provenance": "100% synthetic bare MP3 fixtures with ID3v2.4 (UTF-8 and UTF-16LE text/COMM/TXXX frames) and ID3v1.1 tags; no FFmpeg, network, or private media.",
        "sha256": {name: hashlib.sha256(data).hexdigest() for name, data in files.items()},
    }
    files["manifest.json"] = (json.dumps(manifest, indent=2) + "\n").encode("utf-8")
    return files


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)


if __name__ == "__main__":
    main()
