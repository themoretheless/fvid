#!/usr/bin/env python3
"""Generate synthetic bare FLAC and Ogg Vorbis/Opus metadata regression fixtures for FVID-0229."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/flac-ogg-metadata"


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


def crc16(data: bytes) -> int:
    crc = 0
    for byte in data:
        crc ^= byte << 8
        for _ in range(8):
            if crc & 0x8000:
                crc = ((crc << 1) ^ 0x8005) & 0xFFFF
            else:
                crc = (crc << 1) & 0xFFFF
    return crc


def make_constant_frame(frame_number: int = 0, block_size: int = 192, value: int = 2048) -> bytes:
    assert block_size == 192
    assert 0 <= frame_number < 128
    header_prefix = bytes([0xFF, 0xF8, 0x1A, 0x08, frame_number])
    header = header_prefix + bytes([crc8(header_prefix)])
    subframe = b"\x00" + struct.pack(">h", value)
    body = header + subframe
    return body + struct.pack(">H", crc16(body))


def make_streaminfo(
    *,
    min_block: int = 192,
    max_block: int = 192,
    min_frame: int = 11,
    max_frame: int = 11,
    sample_rate: int = 48_000,
    channels: int = 1,
    bits_per_sample: int = 16,
    total_samples: int = 192,
) -> bytes:
    packed = (
        ((sample_rate & 0xFFFFF) << 44)
        | (((channels - 1) & 0x7) << 41)
        | (((bits_per_sample - 1) & 0x1F) << 36)
        | (total_samples & ((1 << 36) - 1))
    )
    return (
        struct.pack(">HH", min_block, max_block)
        + min_frame.to_bytes(3, "big")
        + max_frame.to_bytes(3, "big")
        + struct.pack(">Q", packed)
        + b"\x00" * 16
    )


def make_metadata_block(block_type: int, is_last: bool, payload: bytes) -> bytes:
    flag = (0x80 if is_last else 0x00) | (block_type & 0x7F)
    return bytes([flag]) + len(payload).to_bytes(3, "big") + payload


def make_vorbis_comment_body(vendor: str, fields: list[str]) -> bytes:
    vendor_bytes = vendor.encode("utf-8")
    out = bytearray(struct.pack("<I", len(vendor_bytes)) + vendor_bytes)
    out += struct.pack("<I", len(fields))
    for field in fields:
        encoded = field.encode("utf-8")
        out += struct.pack("<I", len(encoded)) + encoded
    return bytes(out)


def ogg_sealed(payload: bytes, laces: list[int], granule: int, flags: int, sequence: int, serial: int) -> bytes:
    assert sum(laces) == len(payload)
    buf = bytearray(
        b"OggS"
        + bytes([0, flags])
        + struct.pack("<qII", granule, serial, sequence)
        + bytes(4)
        + bytes([len(laces)])
        + bytes(laces)
        + payload
    )
    crc = 0
    for b in buf:
        crc ^= b << 24
        for _ in range(8):
            crc = ((crc << 1) ^ 0x04C11DB7 if crc & 0x80000000 else crc << 1) & 0xFFFFFFFF
    struct.pack_into("<I", buf, 22, crc)
    return bytes(buf)


def ogg_page(payload: bytes, granule: int, flags: int, sequence: int, serial: int) -> bytes:
    laces = [255] * (len(payload) // 255) + [len(payload) % 255]
    return ogg_sealed(payload, laces, granule, flags, sequence, serial)


def pack_vorbis_bits(fields: list[tuple[int, int]]) -> bytes:
    out = bytearray()
    at = 0
    for value, width in fields:
        for i in range(width):
            if at % 8 == 0:
                out.append(0)
            out[at // 8] |= ((value >> i) & 1) << (at % 8)
            at += 1
    return bytes(out)


def make_tagged_vorbis_ogg(*, serial: int, channels: int, sample_rate: int, fields: list[str]) -> bytes:
    ident = b"\x01vorbis" + struct.pack("<IBIiiiBB", 0, channels, sample_rate, 0, 0, 0, 0x86, 1)
    comment = b"\x03vorbis" + make_vorbis_comment_body("fvid-vorbis-synth", fields) + b"\x01"
    setup_fields = (
        [(5, 8)]
        + [(b, 8) for b in b"vorbis"]
        + [
            (0, 8),
            (0x564342, 24),
            (1, 16),
            (1, 24),
            (0, 1),
            (0, 1),
            (0, 5),
            (0, 4),
            (0, 6),
            (0, 16),
            (0, 6),
            (1, 16),
            (0, 5),
            (0, 2),
            (6, 4),
            (0, 6),
            (0, 16),
            (0, 24),
            (128, 24),
            (31, 24),
            (0, 6),
            (0, 8),
            (0, 3),
            (0, 1),
            (0, 6),
            (0, 16),
            (0, 1),
            (0, 1),
            (0, 2),
            (0, 8),
            (0, 8),
            (0, 8),
            (2, 6),
        ]
    )
    for long_flag in [0, 0, 1]:
        setup_fields.extend([(long_flag, 1), (0, 16), (0, 16), (0, 8)])
    setup = pack_vorbis_bits(setup_fields + [(1, 1)])
    modes = [0, 2, 1, 2, 2, 0]
    packets = []
    for i, mode in enumerate(modes):
        header = [(0, 1), (mode, 2)]
        if mode == 2:
            header.extend(
                [
                    (int(i > 0 and modes[i - 1] == 2), 1),
                    (int(i + 1 < len(modes) and modes[i + 1] == 2), 1),
                ]
            )
        packets.append(pack_vorbis_bits(header + [(0, 1)] * channels))
    data = (
        ogg_page(ident, 0, 2, 0, serial)
        + ogg_page(comment, 0, 0, 1, serial)
        + ogg_page(setup, 0, 0, 2, serial)
    )
    ends = [0, 80, 160, 240, 368, 448]
    for i, pkt in enumerate(packets):
        granule = 411 if i == 5 else ends[i] if i == 1 else -1
        flags = 4 if i == 5 else 0
        data += ogg_page(pkt, granule, flags, i + 3, serial)
    return data


def make_tagged_opus_ogg(*, serial: int, channels: int, fields: list[str]) -> bytes:
    head = b"OpusHead" + bytes([1, channels]) + struct.pack("<HIhB", 0, 48_000, 0, 0)
    tags = b"OpusTags" + make_vorbis_comment_body("fvid-opus-synth", fields)
    raw20 = bytes.fromhex("f8fffe")
    return b"".join(
        [
            ogg_page(head, 0, 2, 0, serial),
            ogg_page(tags, 0, 0, 1, serial),
            ogg_page(raw20, 960, 0, 2, serial),
            ogg_page(raw20, 1920, 4, 3, serial),
        ]
    )


def generate() -> dict[str, bytes]:
    common_fields = [
        "TITLE=Synthetic Tone",
        "ARTIST=FVid Lab",
        "ALBUM=Regression Suite",
        "DATE=2026-10-23",
        "COMMENT=Synthetic Vorbis Comment Fixture",
        "GENRE=Test",
        "COPYRIGHT=Public Domain",
        "ENCODER=fvid-synthetic-generator",
        "TRACKNUMBER=3/12",
        "ALBUMARTIST=FVid Ensemble",
        "DISCNUMBER=1/2",
        "PUBLISHER=FVid Press",
        "DESCRIPTION=Lossless metadata verification",
        "RATING=5",
        "LANGUAGE=eng",
        "CUSTOM_TAG=custom-value",
    ]
    flac_fields = common_fields + ["WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x0004"]
    frames = b"".join(make_constant_frame(frame_number=i, block_size=192, value=2048 if i % 2 == 0 else -2048) for i in range(25))
    tagged_flac = (
        b"fLaC"
        + make_metadata_block(0, False, make_streaminfo(total_samples=192 * 25))
        + make_metadata_block(4, True, make_vorbis_comment_body("fvid-flac-synth", flac_fields))
        + frames
    )
    tagged_opus = make_tagged_opus_ogg(serial=0x46560229, channels=2, fields=common_fields)
    tagged_vorbis = make_tagged_vorbis_ogg(
        serial=0x4656022A,
        channels=2,
        sample_rate=48_000,
        fields=common_fields,
    )
    files = {
        "tagged.flac": tagged_flac,
        "tagged-opus.ogg": tagged_opus,
        "tagged-vorbis.ogg": tagged_vorbis,
    }
    manifest = {
        "generator": "scripts/generate_flac_ogg_metadata_regressions.py",
        "batch_id": "FVID-0229",
        "provenance": "100% synthetic bare FLAC and Ogg Opus/Vorbis fixtures with Vorbis comment metadata; no FFmpeg, network, or private media.",
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
