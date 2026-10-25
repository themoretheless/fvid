#!/usr/bin/env python3
"""Generate synthetic bare FLAC and bare MPEG Layer III prefix packet-byte budget fixtures for FVID-0266."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/raw-flac-mp3-prefix-budget"


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
    # Subframe header: 1-bit pad=0, 6-bit type=1 (VERBATIM), 1-bit wasted=0 -> 0x02
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


def generate() -> dict[str, bytes]:
    # 1. Bare FLAC at 48 kHz mono 16-bit with 2 frames of 192 samples each (4 ms per frame):
    #    - Frame 0 is CONSTANT (11 bytes)
    #    - Frame 1 is VERBATIM (393 bytes)
    frame0_flac = make_flac_constant_frame(0, value=2048)
    assert len(frame0_flac) == 11
    frame1_flac = make_flac_verbatim_frame(1, [2048 if (i & 1) == 0 else -2048 for i in range(192)])
    assert len(frame1_flac) == 393
    si = make_streaminfo(
        min_block=192,
        max_block=192,
        min_frame=len(frame0_flac),
        max_frame=len(frame1_flac),
        sample_rate=48_000,
        channels=1,
        bits_per_sample=16,
        total_samples=384,
    )
    flac_prefix_budget = b"fLaC" + bytes([0x80, 0x00, 0x00, 34]) + si + frame0_flac + frame1_flac

    # 2. Bare MPEG-1 Layer III at 48 kHz mono from saved synthetic 4-frame chain:
    #    - Frame 0 is 960 bytes (pad=0)
    #    - Frame 1 is 961 bytes (pad=1)
    #    - Frame 2 is 960 bytes (pad=0)
    #    - Frame 3 is 961 bytes (pad=1)
    mp3_chain = (ROOT / "tests/fixtures/playback-errors/mp3-decoder/48000-1ch.mp3").read_bytes()
    assert len(mp3_chain) == 960 + 961 + 960 + 961

    files = {
        "prefix-budget.flac": flac_prefix_budget,
        "prefix-budget.mp3": mp3_chain,
    }
    manifest = {
        "generator": "scripts/generate_raw_flac_mp3_prefix_budget_regressions.py",
        "batch_id": "FVID-0266",
        "provenance": "100% synthetic bare FLAC (CONSTANT 11-byte frame + VERBATIM 393-byte frame) and bare MPEG-1 Layer III (960-byte frame + 961-byte padded frame); no FFmpeg, network, or private media.",
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
