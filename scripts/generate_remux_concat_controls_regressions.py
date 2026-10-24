#!/usr/bin/env python3
"""Generate synthetic PCM WAVE, G.711 WAVE, ADTS AAC, Ogg Opus, Matroska, and MP4 fixtures for remux and concat budget/metadata controls (FVID-0215)."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_audio_reader_eof_regressions import generate as owned_eof
from generate_loudnorm_budget_mix_concat_regressions import (
    build_adts_48k,
    build_matroska_flac_mka,
    build_mp4_pcm_m4a,
)
from generate_wave_ogg_flac_plan_regressions import make_g711_wave, ogg_page


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/remux-concat-controls"


def make_pcm_wave(sample_rate: int, samples: list[int], title: str | None = None) -> bytes:
    pcm = b"".join(struct.pack("<h", s) for s in samples)
    fmt = struct.pack("<HHIIHH", 1, 1, sample_rate, sample_rate * 2, 2, 16)
    chunks = [b"fmt " + struct.pack("<I", len(fmt)) + fmt]
    if title is not None:
        raw = title.encode("utf-8") + b"\x00"
        padded = raw + (b"\x00" if len(raw) % 2 else b"")
        inam = b"INAM" + struct.pack("<I", len(raw)) + padded
        info = b"INFO" + inam
        chunks.append(b"LIST" + struct.pack("<I", len(info)) + info)
    chunks.append(b"data" + struct.pack("<I", len(pcm)) + pcm)
    body = b"WAVE" + b"".join(chunks)
    return b"RIFF" + struct.pack("<I", len(body)) + body


def generate() -> dict[str, bytes]:
    samples_a = [3000 if (i % 16 < 8) else -3000 for i in range(6000)]
    samples_b = [2000 if (i % 16 < 8) else -2000 for i in range(4000)]
    pcm_a = make_pcm_wave(48_000, samples_a, title="Original Segment A")
    pcm_b = make_pcm_wave(48_000, samples_b, title="Original Segment B")

    codes_alaw = bytes((0x80 if (i % 16 < 8) else 0x00) for i in range(1920))
    g711_alaw = make_g711_wave(law="alaw", sample_rate=48_000, channels=1, codes=codes_alaw)

    opus_head = b"OpusHead" + bytes([1, 1]) + struct.pack("<HIhB", 0, 48_000, 0, 0)
    opus_tags = b"OpusTags" + struct.pack("<I", 4) + b"fvid" + struct.pack("<I", 0)
    raw20 = bytes.fromhex("f8fffe")
    ogg_opus = b"".join(
        [
            ogg_page(opus_head, 0, 2, 0, 0x46560215),
            ogg_page(opus_tags, 0, 0, 1, 0x46560215),
            ogg_page(raw20, 960, 0, 2, 0x46560215),
            ogg_page(raw20, 1920, 4, 3, 0x46560215),
        ]
    )

    files = {
        "video.mkv": owned_eof()["video.mkv"],
        "pcm-48k-a.wav": pcm_a,
        "pcm-48k-b.wav": pcm_b,
        "g711-alaw-48k.wav": g711_alaw,
        "adts-mono-48k.aac": build_adts_48k(),
        "ogg-opus-48k.ogg": ogg_opus,
        "matroska-flac-48k.mka": build_matroska_flac_mka(),
        "mp4-pcm-48k.m4a": build_mp4_pcm_m4a(),
    }
    manifest = {
        "batch_id": "FVID-0215",
        "sha256": {name: hashlib.sha256(data).hexdigest() for name, data in files.items()},
    }
    files["manifest.json"] = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode("utf-8")
    return files


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)


if __name__ == "__main__":
    main()
