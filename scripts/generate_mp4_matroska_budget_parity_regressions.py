#!/usr/bin/env python3
"""Generate synthetic MP4 avc1, PCM WAVE, and ADTS AAC fixtures for MP4-to-Matroska remux/concat packet-budget and single-stream validation parity regressions (FVID-0254)."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_rotated_mp4_decode_export_probe_regressions import (
    build_avc_32x16_ipcm,
    build_single_rotated_mp4,
)


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/mp4-matroska-budget-parity"


def build_wave_bytes() -> bytes:
    pcm = struct.pack("<hhhh", 256, -256, 512, -512)
    fmt = struct.pack("<HHIIHH", 1, 1, 48000, 96000, 2, 16)
    riff_body = (
        b"WAVE"
        + b"fmt "
        + struct.pack("<I", len(fmt))
        + fmt
        + b"data"
        + struct.pack("<I", len(pcm))
        + pcm
    )
    return b"RIFF" + struct.pack("<I", len(riff_body)) + riff_body


def build_adts_bytes() -> bytes:
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


def generate() -> dict[str, bytes]:
    avcc, avc_packets = build_avc_32x16_ipcm()
    mp4 = build_single_rotated_mp4(avcc, avc_packets)
    wav = build_wave_bytes()
    adts = build_adts_bytes()

    files = {
        "avc-32x16.mp4": mp4,
        "pcm-48k-mono.wav": wav,
        "aac-48k-mono.aac": adts,
    }
    manifest = {
        "batch_id": "FVID-0254",
        "generator": "scripts/generate_mp4_matroska_budget_parity_regressions.py",
        "sha256": {
            name: hashlib.sha256(data).hexdigest()
            for name, data in sorted(files.items())
        },
    }
    files["manifest.json"] = (
        json.dumps(manifest, indent=2, sort_keys=True) + "\n"
    ).encode("utf-8")
    return files


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        if name == "manifest.json":
            (OUT / name).write_bytes(data)


if __name__ == "__main__":
    main()
