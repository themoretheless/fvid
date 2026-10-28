#!/usr/bin/env python3
"""Generate synthetic fixtures for FVID-0268 mix-audio/merge-audio prefix packet budgets and WAVE 4-CC INFO tag preservation."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/mix-merge-prefix-budget-wave-info"


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


def crc16_flac(data: bytes) -> int:
    crc = 0
    for byte in data:
        crc ^= byte << 8
        for _ in range(8):
            if crc & 0x8000:
                crc = ((crc << 1) ^ 0x8005) & 0xFFFF
            else:
                crc = (crc << 1) & 0xFFFF
    return crc


def make_flac_constant_frame(frame_number: int, value: int = 2048) -> bytes:
    header_prefix = bytes([0xFF, 0xF8, 0x1A, 0x08, frame_number])
    header = header_prefix + bytes([crc8(header_prefix)])
    subframe = b"\x00" + struct.pack(">h", value)
    body = header + subframe
    return body + struct.pack(">H", crc16_flac(body))


def make_flac_verbatim_frame(frame_number: int, samples: list[int]) -> bytes:
    assert len(samples) == 192
    header_prefix = bytes([0xFF, 0xF8, 0x1A, 0x08, frame_number])
    header = header_prefix + bytes([crc8(header_prefix)])
    subframe = b"\x02" + b"".join(struct.pack(">h", s) for s in samples)
    body = header + subframe
    return body + struct.pack(">H", crc16_flac(body))


def make_streaminfo(
    *,
    min_block: int,
    max_block: int,
    min_frame: int,
    max_frame: int,
    sample_rate: int,
    channels: int,
    bits_per_sample: int,
    total_samples: int,
) -> bytes:
    packed = (
        ((sample_rate & 0xFFFFF) << 44)
        + (((channels - 1) & 0x7) << 41)
        + (((bits_per_sample - 1) & 0x1F) << 36)
        + (total_samples & ((1 << 36) - 1))
    )
    return (
        struct.pack(">HH", min_block, max_block)
        + min_frame.to_bytes(3, "big")
        + max_frame.to_bytes(3, "big")
        + struct.pack(">Q", packed)
        + b"\x00" * 16
    )


def make_vorbis_comment_block(is_last: bool, vendor: str, fields: list[str]) -> bytes:
    v_bytes = vendor.encode("utf-8")
    body = bytearray(struct.pack("<I", len(v_bytes)) + v_bytes)
    body += struct.pack("<I", len(fields))
    for f in fields:
        enc = f.encode("utf-8")
        body += struct.pack("<I", len(enc)) + enc
    flag = (0x80 if is_last else 0x00) | 4
    return bytes([flag]) + len(body).to_bytes(3, "big") + bytes(body)


def ogg_page(payload: bytes, granule: int, flags: int, sequence: int, serial: int) -> bytes:
    laces = [255] * (len(payload) // 255) + [len(payload) % 255]
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


def synchsafe(value: int) -> bytes:
    return bytes(
        [
            (value >> 21) & 0x7F,
            (value >> 14) & 0x7F,
            (value >> 7) & 0x7F,
            value & 0x7F,
        ]
    )


def id3v24_txxx_frame(description: str, value: str) -> bytes:
    payload = bytes([3]) + description.encode("utf-8") + b"\x00" + value.encode("utf-8")
    return b"TXXX" + synchsafe(len(payload)) + b"\x00\x00" + payload


def id3v24_text_frame(frame_id: str, text: str) -> bytes:
    payload = bytes([3]) + text.encode("utf-8") + b"\x00"
    return frame_id.encode("ascii") + synchsafe(len(payload)) + b"\x00\x00" + payload


def generate() -> dict[str, bytes]:
    # 1. Two bare FLAC files at 48 kHz mono 16-bit with 4-CC Vorbis comments (`ABCD=flac-custom-4cc`, `TITLE=FLAC Mix Source`):
    #    - Frame 0 is CONSTANT (11 bytes)
    #    - Frame 1 is VERBATIM (393 bytes)
    f0_a = make_flac_constant_frame(0, value=2048)
    f1_a = make_flac_verbatim_frame(1, [2048 if (i & 1) == 0 else -2048 for i in range(192)])
    f0_b = make_flac_constant_frame(0, value=1024)
    f1_b = make_flac_verbatim_frame(1, [1024 if (i & 1) == 0 else -1024 for i in range(192)])
    si = make_streaminfo(
        min_block=192,
        max_block=192,
        min_frame=len(f0_a),
        max_frame=len(f1_a),
        sample_rate=48_000,
        channels=1,
        bits_per_sample=16,
        total_samples=384,
    )
    vc = make_vorbis_comment_block(
        True,
        "fvid-flac",
        ["TITLE=FLAC Mix Source", "ABCD=flac-custom-4cc", "ISRC=US-FVD-26-00268"],
    )
    flac_a = b"fLaC" + bytes([0x00, 0x00, 0x00, 34]) + si + vc + f0_a + f1_a
    flac_b = b"fLaC" + bytes([0x00, 0x00, 0x00, 34]) + si + vc + f0_b + f1_b

    # 2. Ogg Opus at 48 kHz mono with 4-CC OpusTags (`ABCD=opus-custom-4cc`, `ISRC=US-OPU-26-00268`):
    head = b"OpusHead" + bytes([1, 1]) + struct.pack("<HIhB", 0, 48_000, 0, 0)
    v_bytes = b"fvid-opus"
    opus_fields = [b"TITLE=Opus WAVE Source", b"ABCD=opus-custom-4cc", b"ISRC=US-OPU-26-00268"]
    tags_body = bytearray(b"OpusTags" + struct.pack("<I", len(v_bytes)) + v_bytes + struct.pack("<I", len(opus_fields)))
    for f in opus_fields:
        tags_body += struct.pack("<I", len(f)) + f
    raw20 = bytes.fromhex("f8fffe")
    ogg_opus = b"".join(
        [
            ogg_page(head, 0, 2, 0, 0x46560268),
            ogg_page(bytes(tags_body), 0, 0, 1, 0x46560268),
            ogg_page(raw20, 960, 0, 2, 0x46560268),
            ogg_page(raw20, 1920, 4, 3, 0x46560268),
        ]
    )

    # 3. Bare MP3 at 48 kHz mono with 4-CC ID3v2.4 TXXX tags (`ABCD=mp3-custom-4cc`, `ISRC=US-MP3-26-00268`):
    raw_mp3 = (ROOT / "tests/fixtures/playback-errors/mpeg-clock-metadata/mp3-48000.mp3").read_bytes()
    id3_body = (
        id3v24_text_frame("TIT2", "MP3 WAVE Source")
        + id3v24_txxx_frame("ABCD", "mp3-custom-4cc")
        + id3v24_txxx_frame("ISRC", "US-MP3-26-00268")
    )
    mp3_tagged = b"ID3\x04\x00\x00" + synchsafe(len(id3_body)) + id3_body + raw_mp3

    files = {
        "prefix-a.flac": flac_a,
        "prefix-b.flac": flac_b,
        "fourcc-tags.opus": ogg_opus,
        "fourcc-tags.mp3": mp3_tagged,
    }
    manifest = {
        "generator": "scripts/generate_mix_merge_prefix_budget_wave_info_regressions.py",
        "batch_id": "FVID-0268",
        "provenance": "100% synthetic bare FLAC (11-byte CONSTANT frame + 393-byte VERBATIM frame), Ogg Opus, and bare MP3 with 4-CC Vorbis/ID3 tags; no FFmpeg, network, or private media.",
        "sha256": {name: hashlib.sha256(data).hexdigest() for name, data in files.items()},
    }
    files["manifest.json"] = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode("utf-8")
    return files


def write_fixtures(out_dir: Path = OUT) -> dict[str, bytes]:
    out_dir.mkdir(parents=True, exist_ok=True)
    files = generate()
    for name, content in files.items():
        (out_dir / name).write_bytes(content)
    return files


if __name__ == "__main__":
    write_fixtures()
