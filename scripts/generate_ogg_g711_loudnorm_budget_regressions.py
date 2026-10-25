#!/usr/bin/env python3
"""Generate synthetic Ogg Opus, G.711 WAVE, and ADTS AAC fixtures for packet-byte budget and stream-selection parity regressions (FVID-0256)."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/ogg-g711-loudnorm-budget"


def _ogg_crc_table() -> list[int]:
    table = [0] * 256
    for index in range(256):
        word = index << 24
        for _ in range(8):
            if word & 0x8000_0000:
                word = ((word << 1) ^ 0x04C1_1DB7) & 0xFFFF_FFFF
            else:
                word = (word << 1) & 0xFFFF_FFFF
        table[index] = word
    return table


_OGG_TABLE = _ogg_crc_table()


def _ogg_crc(data: bytes) -> int:
    crc = 0
    for byte in data:
        crc = ((crc << 8) & 0xFFFF_FFFF) ^ _OGG_TABLE[((crc >> 24) ^ byte) & 0xFF]
    return crc


def _ogg_page(serial: int, number: int, granule: int, flags: int, packets: list[bytes]) -> bytes:
    table = bytearray()
    body = bytearray()
    for packet in packets:
        body.extend(packet)
        rest = len(packet)
        while rest >= 255:
            table.append(255)
            rest -= 255
        table.append(rest)
    header_prefix = struct.pack("<4sBBqII", b"OggS", 0, flags, granule, serial, number)
    unsealed = header_prefix + struct.pack("<IB", 0, len(table)) + bytes(table) + bytes(body)
    crc = _ogg_crc(unsealed)
    return header_prefix + struct.pack("<IB", crc, len(table)) + bytes(table) + bytes(body)


def build_ogg_opus_mono_bytes() -> bytes:
    # 19-byte OpusHead (version=1, channels=1, pre_skip=0, input_sample_rate=48000, output_gain=0, mapping_family=0)
    opus_head = b"OpusHead" + struct.pack("<BBHIhB", 1, 1, 0, 48000, 0, 0)
    # Minimal OpusTags (vendor="fvid", 0 comments)
    opus_tags = b"OpusTags" + struct.pack("<I", 4) + b"fvid" + struct.pack("<I", 0)
    # 4-byte valid CELT FB 20 ms mono Opus packet (960 samples at 48 kHz)
    opus_pkt = bytes([0xF8, 0xFF, 0xFE, 0x00])
    return (
        _ogg_page(1, 0, 0, 0x02, [opus_head])
        + _ogg_page(1, 1, 0, 0x00, [opus_tags])
        + _ogg_page(1, 2, 960, 0x04, [opus_pkt])
    )


def build_g711_wave_stereo_alaw_bytes() -> bytes:
    # 8 kHz stereo A-law (format tag 6, 2 channels, block_align = 2, bits_per_sample = 8), 8 frames = 16 bytes
    alaw_data = bytes([0xD5, 0x55] * 8)
    fmt = struct.pack("<HHIIHH", 6, 2, 8000, 16000, 2, 8)
    riff_body = (
        b"WAVE"
        + b"fmt "
        + struct.pack("<I", len(fmt))
        + fmt
        + b"data"
        + struct.pack("<I", len(alaw_data))
        + alaw_data
    )
    return b"RIFF" + struct.pack("<I", len(riff_body)) + riff_body


def build_adts_mono_bytes() -> bytes:
    # 48 kHz mono AAC-LC ADTS frame (7-byte header + 6-byte silent SCE payload = 13 bytes)
    payload = bytes([0x21, 0x10, 0x04, 0x60, 0x8C, 0x1C])
    frame_len = 7 + len(payload)
    header = bytearray(7)
    header[0] = 0xFF
    header[1] = 0xF1
    header[2] = (1 << 6) | (3 << 2) | (0 << 1) | ((1 >> 2) & 0x01)
    header[3] = ((1 & 0x03) << 6) | ((frame_len >> 11) & 0x03)
    header[4] = (frame_len >> 3) & 0xFF
    header[5] = ((frame_len & 0x07) << 5) | 0x1F
    header[6] = 0xFC
    return bytes(header) + payload


def generate(out_dir: Path = OUT) -> dict[str, object]:
    out_dir.mkdir(parents=True, exist_ok=True)
    specs = [
        (
            "ogg-opus-mono-4byte-pkt.ogg",
            build_ogg_opus_mono_bytes(),
            "48 kHz mono Ogg Opus with a 4-byte encoded audio packet for packet-byte budget and stream-selection parity across plan/decode/loudness/loudnorm",
        ),
        (
            "wave-stereo-alaw-8khz.wav",
            build_g711_wave_stereo_alaw_bytes(),
            "8 kHz stereo G.711 A-law WAVE (2 bytes per channel frame) for packet-byte budget and stream-selection parity across plan/decode/loudness/loudnorm",
        ),
        (
            "adts-mono-6byte-pkt.aac",
            build_adts_mono_bytes(),
            "48 kHz mono AAC-LC ADTS with a 6-byte payload for loudnorm/loudness stream-selection parity",
        ),
    ]
    manifest_entries = []
    for name, data, purpose in specs:
        path = out_dir / name
        path.write_bytes(data)
        manifest_entries.append(
            {
                "name": name,
                "bytes": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
                "purpose": purpose,
            }
        )
    manifest = {
        "suite": "ogg-g711-loudnorm-budget",
        "improvement_id": "FVID-0256",
        "fixtures": manifest_entries,
    }
    (out_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )
    return manifest


if __name__ == "__main__":
    generate()
