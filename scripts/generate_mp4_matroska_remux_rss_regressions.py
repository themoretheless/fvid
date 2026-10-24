#!/usr/bin/env python3
"""Generate synthetic MP4, Matroska, and ADTS fixtures for RSS budget and MP4-Matroska remux routing (FVID-0219)."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_audio_reader_eof_regressions import generate as owned_eof
from generate_loudnorm_budget_mix_concat_regressions import build_adts_48k

ROOT = Path(__file__).resolve().parents[1]
SEED_DIR = ROOT / "tests/fixtures/playback-errors"
OUT = SEED_DIR / "mp4-matroska-remux-rss"


def boxes(data: bytes):
    offset = 0
    while offset < len(data):
        size, tag = struct.unpack_from(">I4s", data, offset)
        assert size >= 8 and offset + size <= len(data)
        yield tag, data[offset + 8 : offset + size]
        offset += size


def box(tag: bytes, body: bytes) -> bytes:
    return struct.pack(">I4s", len(body) + 8, tag) + body


def build_two_track_mp4() -> bytes:
    seed = (SEED_DIR / "avc-slice-lists-temporal.mp4").read_bytes()
    parts = list(boxes(seed))
    assert [tag for tag, _ in parts].index(b"mdat") < [tag for tag, _ in parts].index(b"moov")
    output = bytearray()
    for tag, body in parts:
        if tag == b"moov":
            children = list(boxes(body))
            tracks = [b for t, b in children if t == b"trak"]
            assert len(tracks) == 1
            duplicate = bytearray()
            for child, payload in boxes(tracks[0]):
                if child == b"tkhd":
                    payload = bytearray(payload)
                    offset = 20 if payload[0] == 1 else 12
                    struct.pack_into(">I", payload, offset, 2)
                duplicate += box(child, payload)
            rebuilt = bytearray()
            for child, payload in children:
                if child == b"mvhd":
                    payload = payload[:-4] + struct.pack(">I", 3)
                rebuilt += box(child, payload)
            entries = b"".join(
                box(t, box(b"data", b"\x00\x00\x00\x01" + bytes(4) + v.encode()))
                for t, v in [
                    (b"\xa9nam", "Original MP4 Title"),
                    (b"\xa9ART", "Original MP4 Artist"),
                ]
            )
            udta = box(b"udta", box(b"meta", bytes(4) + box(b"ilst", entries)))
            body = bytes(rebuilt) + box(b"trak", bytes(duplicate)) + udta
        output += box(tag, body)

    track_tagged = bytearray()
    for tag, body in boxes(bytes(output)):
        if tag == b"moov":
            children = bytearray()
            index = 0
            for child, payload in boxes(body):
                if child == b"trak":
                    rebuilt = bytearray()
                    for field, value in boxes(payload):
                        if field == b"udta":
                            continue
                        if field == b"mdia":
                            media = bytearray()
                            for kind, contents in boxes(value):
                                if kind == b"mdhd":
                                    contents = bytearray(contents)
                                    code = 0
                                    for letter in ["eng", "fra"][index]:
                                        code = (code << 5) | (ord(letter) - 96)
                                    struct.pack_into(
                                        ">H", contents, 20 if contents[0] == 0 else 32, code
                                    )
                                media += box(kind, bytes(contents))
                            value = bytes(media)
                        rebuilt += box(field, value)
                    rebuilt += box(b"udta", box(b"name", f"Track {index}".encode()))
                    payload = bytes(rebuilt)
                    index += 1
                children += box(child, payload)
            assert index == 2
            body = bytes(children)
        track_tagged += box(tag, body)
    return bytes(track_tagged)


def generate() -> dict[str, bytes]:
    files = {
        "two-track-avc.mp4": build_two_track_mp4(),
        "video.mkv": owned_eof()["video.mkv"],
        "adts-48k.aac": build_adts_48k(),
    }
    manifest = {
        "batch_id": "FVID-0219",
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
