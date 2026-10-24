#!/usr/bin/env python3
"""Generate synthetic WAVE, ADTS AAC, and bare FLAC fixtures for WAVE INFO metadata mutations in decode-audio/trim and controlled mix/merge audio APIs (FVID-0220)."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_loudnorm_budget_mix_concat_regressions import (
    build_adts_48k,
    build_raw_flac_48k,
)

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/audio-wave-metadata-mix-controls"


def info_entry(tag: bytes, text: str) -> bytes:
    assert len(tag) == 4
    raw = text.encode("utf-8") + b"\x00"
    size = len(raw)
    pad = b"\x00" if (size & 1) else b""
    return tag + struct.pack("<I", size) + raw + pad


def build_tagged_wave_48k() -> bytes:
    """Synthetic 48 kHz mono 16-bit PCM WAVE (960 frames = 20 ms) with RIFF INFO INAM/IART/ICMT."""
    rate = 48_000
    channels = 1
    bits = 16
    block_align = channels * (bits // 8)
    byte_rate = rate * block_align
    frames = 960
    samples = bytearray()
    for i in range(frames):
        val = 4096 if (i // 48) % 2 == 0 else -4096
        samples += struct.pack("<h", val)

    fmt_body = struct.pack(
        "<HHIIHH",
        1,  # PCM
        channels,
        rate,
        byte_rate,
        block_align,
        bits,
    )
    fmt_chunk = b"fmt " + struct.pack("<I", len(fmt_body)) + fmt_body
    entries = (
        info_entry(b"INAM", "Original WAV Title")
        + info_entry(b"IART", "Original WAV Artist")
        + info_entry(b"ICMT", "Preserve Me")
    )
    list_body = b"INFO" + entries
    list_pad = b"\x00" if (len(list_body) & 1) else b""
    list_chunk = b"LIST" + struct.pack("<I", len(list_body)) + list_body + list_pad
    data_chunk = b"data" + struct.pack("<I", len(samples)) + bytes(samples)
    riff_body = b"WAVE" + fmt_chunk + list_chunk + data_chunk
    return b"RIFF" + struct.pack("<I", len(riff_body)) + riff_body


def generate() -> dict[str, bytes]:
    fixtures: dict[str, bytes] = {
        "tagged-48k.wav": build_tagged_wave_48k(),
        "adts-48k.aac": build_adts_48k(),
        "flac-48k.flac": build_raw_flac_48k(),
    }
    manifest = {
        "generator": "scripts/generate_audio_wave_metadata_mix_controls_regressions.py",
        "batch_id": "FVID-0220",
        "fixtures": {
            name: {
                "bytes": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
            }
            for name, data in sorted(fixtures.items())
        },
    }
    fixtures["manifest.json"] = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode(
        "utf-8"
    )
    return fixtures


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)


if __name__ == "__main__":
    main()
