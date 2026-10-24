#!/usr/bin/env python3
"""Generate synthetic audio decoder regression fixtures for FVID-0197."""

from __future__ import annotations

import json
from pathlib import Path
import struct


ROOT = Path(__file__).resolve().parents[1]
OUT_DIR = ROOT / "tests" / "fixtures" / "playback-errors" / "audio-decoders"


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


def make_flac_constant_frame_192(frame_number: int = 0, value: int = 1024) -> bytes:
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


def xm_field(text: str, width: int) -> bytes:
    raw = text.encode("ascii")[:width]
    return raw + b"\x00" * (width - len(raw))


def build_two_channel_xm() -> bytes:
    out = bytearray()
    out += b"Extended Module: "
    out += xm_field("fvid decoder test", 20)
    out.append(0x1A)
    out += xm_field("FastTracker II v2.00", 20)
    out += struct.pack("<H", 0x0104)
    out += struct.pack("<I", 276)
    out += struct.pack("<H", 1)  # song length
    out += struct.pack("<H", 0)  # restart position
    out += struct.pack("<H", 2)  # 2 channels
    out += struct.pack("<H", 1)  # 1 pattern
    out += struct.pack("<H", 1)  # 1 instrument
    out += struct.pack("<H", 1)  # linear freq table
    out += struct.pack("<H", 6)  # speed
    out += struct.pack("<H", 125)  # bpm
    order_table = bytearray(256)
    order_table[0] = 0
    out += order_table

    # Pattern 0: 1 row, 2 channels, unpacked
    # Channel 0: note 49, inst 1, vol 0x51 (unsupported volume column effect), effect 0x24 (unsupported effect), param 1
    # Channel 1: empty cell
    pattern_data = bytes([49, 1, 0x51, 0x24, 1, 0, 0, 0, 0, 0])
    out += struct.pack("<IBHH", 9, 0, 1, len(pattern_data))
    out += pattern_data

    # Instrument 0: 1 sample
    inst = bytearray()
    inst += struct.pack("<I", 243)
    inst += xm_field("lead", 22)
    inst.append(0)
    inst += struct.pack("<H", 1)
    inst += struct.pack("<I", 40)
    inst += bytes(96)
    inst += bytes(96)
    inst += bytes(233 - 225)
    inst += struct.pack("<H", 0)
    inst += bytes(4)
    inst += struct.pack("<H", 0)
    inst += struct.pack("<H", 0)
    assert len(inst) == 243

    pcm = bytes([40, 216, 0, 0])
    sample_hdr = bytearray()
    sample_hdr += struct.pack("<III", len(pcm), 0, len(pcm))
    sample_hdr += bytes([64, 0, 0x00, 128, 0, 0])
    sample_hdr += xm_field("s0", 22)
    assert len(sample_hdr) == 40
    out += inst + sample_hdr + pcm
    return bytes(out)


def build_xm_row_packet(bpm: int, speed: int, cells: list[tuple[int, int, int, int, int]]) -> bytes:
    header = struct.pack("<HB", bpm, speed) + bytes(5)
    body = b"".join(bytes(c) for c in cells)
    return header + body


def generate(out_dir: Path = OUT_DIR) -> dict[str, bytes]:
    out_dir.mkdir(parents=True, exist_ok=True)
    files = {
        "flac_streaminfo_192.bin": make_streaminfo(min_block=192, max_block=192),
        "flac_streaminfo_max_64.bin": make_streaminfo(min_block=16, max_block=64),
        "flac_streaminfo_min_frame_gt_max_frame.bin": make_streaminfo(
            min_block=192,
            max_block=192,
            min_frame=20,
            max_frame=11,
        ),
        "flac_frame_192.bin": make_flac_constant_frame_192(),
        "xm_two_channel.xm": build_two_channel_xm(),
        "xm_row_2ch_valid.bin": build_xm_row_packet(
            125,
            6,
            [(49, 1, 0x51, 0x24, 1), (0, 0, 0, 0, 0)],
        ),
        "xm_row_1ch_mismatch.bin": build_xm_row_packet(
            125,
            6,
            [(49, 1, 0x40, 0, 0)],
        ),
    }
    for name, content in files.items():
        (out_dir / name).write_bytes(content)

    manifest = {
        "generator": "scripts/generate_audio_decoder_regressions.py",
        "cases": [
            {
                "id": "flac-valid-192",
                "file": "flac_streaminfo_192.bin",
                "expect": "accept",
            },
            {
                "id": "flac-frame-exceeds-streaminfo-max-block",
                "file": "flac_streaminfo_max_64.bin",
                "expect": "refuse",
                "reason": "FLAC frame block size 192 exceeds STREAMINFO max_block 64",
            },
            {
                "id": "flac-min-frame-gt-max-frame",
                "file": "flac_streaminfo_min_frame_gt_max_frame.bin",
                "expect": "refuse",
                "reason": "FLAC STREAMINFO min_frame 20 exceeds non-zero max_frame 11",
            },
            {
                "id": "xm-two-channel-valid-row",
                "file": "xm_row_2ch_valid.bin",
                "expect": "accept",
            },
            {
                "id": "xm-row-channel-count-mismatch",
                "file": "xm_row_1ch_mismatch.bin",
                "expect": "refuse",
                "reason": "XM row packet has 1 cell for a 2-channel module",
            },
        ],
    }
    manifest_bytes = (json.dumps(manifest, indent=2) + "\n").encode("utf-8")
    (out_dir / "manifest.json").write_bytes(manifest_bytes)
    files["manifest.json"] = manifest_bytes
    return files


if __name__ == "__main__":
    generate()
