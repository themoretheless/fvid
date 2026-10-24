#!/usr/bin/env python3
"""Generate synthetic tagged WAVE, Ogg Opus, and multitrack Matroska fixtures for loudnorm and normalize-loudness WAVE INFO metadata preservation/mutations and top-level media routing (FVID-0221)."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_media_trim_loudness_regressions import build_multitrack_pcm_flac_mka
from generate_wave_ogg_flac_plan_regressions import ogg_page

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/loudnorm-wave-metadata-routing"


def info_entry(tag: bytes, text: str) -> bytes:
    assert len(tag) == 4
    raw = text.encode("utf-8") + b"\x00"
    size = len(raw)
    pad = b"\x00" if (size & 1) else b""
    return tag + struct.pack("<I", size) + raw + pad


def build_tagged_wave_48k_450ms() -> bytes:
    """Synthetic 48 kHz mono 16-bit PCM WAVE (21,600 frames = 450 ms > 400 ms loudness gating window) with RIFF INFO INAM/IART/ICMT."""
    rate = 48_000
    channels = 1
    bits = 16
    block_align = channels * (bits // 8)
    byte_rate = rate * block_align
    frames = 21_600
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
        + info_entry(b"ICMT", "Preserve WAV Comment")
    )
    list_body = b"INFO" + entries
    list_pad = b"\x00" if (len(list_body) & 1) else b""
    list_chunk = b"LIST" + struct.pack("<I", len(list_body)) + list_body + list_pad
    data_chunk = b"data" + struct.pack("<I", len(samples)) + bytes(samples)
    riff_body = b"WAVE" + fmt_chunk + list_chunk + data_chunk
    return b"RIFF" + struct.pack("<I", len(riff_body)) + riff_body


def build_tagged_ogg_opus_48k() -> bytes:
    """Synthetic 48 kHz mono Ogg Opus (1,920 samples = 40 ms) with OpusTags TITLE/ARTIST/COMMENT."""
    opus_head = b"OpusHead" + bytes([1, 1]) + struct.pack("<HIhB", 0, 48_000, 0, 0)
    comments = [
        b"TITLE=Original Ogg Title",
        b"ARTIST=Original Ogg Artist",
        b"COMMENT=Preserve Ogg Comment",
    ]
    opus_tags = bytearray(b"OpusTags" + struct.pack("<I", 4) + b"fvid")
    opus_tags += struct.pack("<I", len(comments))
    for c in comments:
        opus_tags += struct.pack("<I", len(c)) + c
    raw20 = bytes.fromhex("f8fffe")
    return b"".join(
        [
            ogg_page(opus_head, 0, 2, 0, 0x46560221),
            ogg_page(bytes(opus_tags), 0, 0, 1, 0x46560221),
            ogg_page(raw20, 960, 0, 2, 0x46560221),
            ogg_page(raw20, 1920, 4, 3, 0x46560221),
        ]
    )


def generate() -> dict[str, bytes]:
    fixtures: dict[str, bytes] = {
        "tagged-48k-450ms.wav": build_tagged_wave_48k_450ms(),
        "tagged-opus-48k.ogg": build_tagged_ogg_opus_48k(),
        "multitrack-pcm-flac.mka": build_multitrack_pcm_flac_mka(),
    }
    manifest = {
        "generator": "scripts/generate_loudnorm_wave_metadata_routing_regressions.py",
        "batch_id": "FVID-0221",
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
