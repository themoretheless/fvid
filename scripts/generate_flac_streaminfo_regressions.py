#!/usr/bin/env python3
"""Generate synthetic FLAC STREAMINFO and metadata regression fixtures for FVID-0194."""

from __future__ import annotations

import json
from pathlib import Path
import struct


ROOT = Path(__file__).resolve().parents[1]
FIXTURES_DIR = ROOT / "tests" / "fixtures" / "playback-errors" / "flac-streaminfo"


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
    # Sync 0xFFF8 (fixed-blocksize), block=1 (192 samples), rate=10 (48000 Hz),
    # channel=0 (1 ch mono), bits=4 (16-bit), reserved=0 -> 0x1A, 0x08.
    header_prefix = bytes([0xFF, 0xF8, 0x1A, 0x08, frame_number])
    header = header_prefix + bytes([crc8(header_prefix)])
    # Subframe header: 1-bit pad=0, 6-bit type=0 (CONSTANT), 1-bit wasted=0 -> 0x00
    # Followed by 16-bit signed big-endian constant sample.
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


def make_vorbis_comment(fields: list[str]) -> bytes:
    vendor = b"fvid-synth"
    out = bytearray(struct.pack("<I", len(vendor)) + vendor)
    out += struct.pack("<I", len(fields))
    for field in fields:
        encoded = field.encode("utf-8")
        out += struct.pack("<I", len(encoded)) + encoded
    return bytes(out)


def ebml_vint(value: int) -> bytes:
    return bytes([0x80 | value])


def ebml_element(elem_id: bytes, payload: bytes) -> bytes:
    length = len(payload)
    if length < 0x7F:
        size_bytes = bytes([0x80 | length])
    elif length < 0x3FFF:
        size_bytes = struct.pack(">H", 0x4000 | length)
    else:
        raise ValueError("payload too large")
    return elem_id + size_bytes + payload


def make_matroska_flac(codec_private: bytes, frame: bytes, channels: int = 1) -> bytes:
    ebml_head = ebml_element(
        b"\x1a\x45\xdf\xa3",
        ebml_element(b"\x42\x82", b"matroska")
        + ebml_element(b"\x42\x87", b"\x04")
        + ebml_element(b"\x42\x85", b"\x02"),
    )
    info = ebml_element(
        b"\x15\x49\xa9\x66",
        ebml_element(b"\x2a\xd7\xb1", (1_000_000).to_bytes(3, "big")),
    )
    audio = ebml_element(
        b"\xe1",
        ebml_element(b"\xb5", struct.pack(">d", 48_000.0))
        + ebml_element(b"\x9f", bytes([channels]))
        + ebml_element(b"\x62\x64", b"\x10"),
    )
    track_entry = ebml_element(
        b"\xae",
        ebml_element(b"\xd7", b"\x01")
        + ebml_element(b"\x73\xc5", b"\x01")
        + ebml_element(b"\x83", b"\x02")
        + ebml_element(b"\x86", b"A_FLAC")
        + ebml_element(b"\x63\xa2", codec_private)
        + audio,
    )
    tracks = ebml_element(b"\x16\x54\xae\x6b", track_entry)
    simple_block = ebml_element(b"\xa3", b"\x81\x00\x00\x80" + frame)
    cluster = ebml_element(
        b"\x1f\x43\xb6\x75",
        ebml_element(b"\xe7", b"\x00") + simple_block,
    )
    segment = ebml_element(b"\x18\x53\x80\x67", info + tracks + cluster)
    return ebml_head + segment


def build_fixtures() -> dict[str, bytes]:
    frame = make_constant_frame()
    assert len(frame) == 11
    valid_si = make_streaminfo()
    assert len(valid_si) == 34

    return {
        "valid_baseline.flac": (
            b"fLaC"
            + make_metadata_block(0, False, valid_si)
            + make_metadata_block(4, True, make_vorbis_comment(["WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x4"]))
            + frame
        ),
        "padding_before_streaminfo.flac": (
            b"fLaC"
            + make_metadata_block(1, False, b"\x00" * 4)
            + make_metadata_block(0, True, valid_si)
            + frame
        ),
        "duplicate_streaminfo.flac": (
            b"fLaC"
            + make_metadata_block(0, False, valid_si)
            + make_metadata_block(0, True, valid_si)
            + frame
        ),
        "overlong_streaminfo.flac": (
            b"fLaC"
            + make_metadata_block(0, True, valid_si + b"\x00\x00")
            + frame
        ),
        "reserved_metadata_type_7.flac": (
            b"fLaC"
            + make_metadata_block(0, False, valid_si)
            + make_metadata_block(7, True, b"\x00" * 4)
            + frame
        ),
        "reserved_metadata_type_127.flac": (
            b"fLaC"
            + make_metadata_block(0, False, valid_si)
            + make_metadata_block(127, True, b"\x00" * 4)
            + frame
        ),
        "min_block_below_16.flac": (
            b"fLaC"
            + make_metadata_block(0, True, make_streaminfo(min_block=15, max_block=192))
            + frame
        ),
        "min_block_exceeds_max_block.flac": (
            b"fLaC"
            + make_metadata_block(0, True, make_streaminfo(min_block=256, max_block=192))
            + frame
        ),
        "max_frame_mismatch_when_min_zero.flac": (
            b"fLaC"
            + make_metadata_block(0, True, make_streaminfo(min_frame=0, max_frame=256))
            + frame
        ),
        "channel_mask_popcount_overflow.flac": (
            b"fLaC"
            + make_metadata_block(0, False, valid_si)
            + make_metadata_block(4, True, make_vorbis_comment(["WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x3"]))
            + frame
        ),
        "matroska_flac_valid.mkv": make_matroska_flac(
            b"fLaC"
            + make_metadata_block(0, False, valid_si)
            + make_metadata_block(4, True, make_vorbis_comment(["WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x4"])),
            frame,
        ),
        "matroska_flac_mask_popcount_overflow.mkv": make_matroska_flac(
            b"fLaC"
            + make_metadata_block(0, False, valid_si)
            + make_metadata_block(4, True, make_vorbis_comment(["WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x3"])),
            frame,
        ),
    }


def write_fixtures(out_dir: Path = FIXTURES_DIR) -> dict[str, Path]:
    out_dir.mkdir(parents=True, exist_ok=True)
    fixtures = build_fixtures()
    written: dict[str, Path] = {}
    manifest_entries = []
    for name, content in fixtures.items():
        path = out_dir / name
        path.write_bytes(content)
        written[name] = path
        manifest_entries.append({"name": name, "bytes": len(content)})
    (out_dir / "manifest.json").write_text(
        json.dumps({"batch": 187, "id": "FVID-0194", "fixtures": manifest_entries}, indent=2)
        + "\n",
        encoding="utf-8",
    )
    return written


if __name__ == "__main__":
    write_fixtures()
