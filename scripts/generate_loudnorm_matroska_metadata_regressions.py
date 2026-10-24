#!/usr/bin/env python3
"""Generate synthetic WAVE, ADTS AAC, and bare MP3 fixtures for FVID-0231 loudnorm Matroska output and metadata regressions."""

from __future__ import annotations

import hashlib
import json
import math
from pathlib import Path
import struct


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/loudnorm-matroska-metadata"


def build_tagged_wav_48k() -> bytes:
    sample_rate = 48_000
    channels = 1
    bits_per_sample = 16
    frames = 24_000  # 0.5 s at 48 kHz (exceeds 400 ms BS.1770 gating block)
    pcm = bytearray()
    for i in range(frames):
        val = int(round(8192.0 * math.sin(2.0 * math.pi * 440.0 * i / sample_rate)))
        pcm.extend(struct.pack("<h", val))

    fmt_chunk = struct.pack(
        "<HHIIHH",
        1,  # PCM
        channels,
        sample_rate,
        sample_rate * channels * (bits_per_sample // 8),
        channels * (bits_per_sample // 8),
        bits_per_sample,
    )

    def info_subchunk(fourcc: bytes, text: str) -> bytes:
        raw = text.encode("utf-8") + b"\x00"
        padded = raw + (b"\x00" if len(raw) % 2 == 1 else b"")
        return fourcc + struct.pack("<I", len(raw)) + padded

    info_body = (
        b"INFO"
        + info_subchunk(b"INAM", "Source Wave Loudnorm")
        + info_subchunk(b"IART", "Legacy Loudnorm Artist")
        + info_subchunk(b"ICMT", "Drop Me Comment")
    )
    list_chunk = b"LIST" + struct.pack("<I", len(info_body)) + info_body
    data_chunk = b"data" + struct.pack("<I", len(pcm)) + bytes(pcm)
    riff_body = (
        b"WAVE"
        + b"fmt "
        + struct.pack("<I", len(fmt_chunk))
        + fmt_chunk
        + list_chunk
        + data_chunk
    )
    return b"RIFF" + struct.pack("<I", len(riff_body)) + riff_body


def build_adts_48k() -> bytes:
    base_frame = (
        ROOT
        / "tests/fixtures/playback-errors/adts-matroska-audio-metadata/adts-seg-a-48k.aac"
    ).read_bytes()
    # Repeat 8 times -> 24 AAC frames (~0.512 s at 48 kHz) so dual-pass gating has > 0.4 s
    return base_frame * 8


def build_mp3_48k() -> bytes:
    return (
        ROOT / "tests/fixtures/playback-errors/mp3-id3-metadata/tagged-id3v24.mp3"
    ).read_bytes()


def generate() -> dict[str, bytes]:
    return {
        "tone-tagged-48k.wav": build_tagged_wav_48k(),
        "tone-48k.aac": build_adts_48k(),
        "tone-tagged-48k.mp3": build_mp3_48k(),
    }


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    files = generate()
    sha256_map: dict[str, str] = {}
    for name, data in files.items():
        path = OUT / name
        path.write_bytes(data)
        sha256_map[name] = hashlib.sha256(data).hexdigest()

    manifest = {
        "batch_id": "FVID-0231",
        "description": "Synthetic WAVE, ADTS AAC, and bare MP3 fixtures for loudnorm Matroska (.mka/.mkv) output and container/stream metadata mutations",
        "files": list(files.keys()),
        "sha256": sha256_map,
    }
    (OUT / "manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )


if __name__ == "__main__":
    main()
