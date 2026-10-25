#!/usr/bin/env python3
"""Generate synthetic WAVE, ADTS AAC, MP4 PCM, and Matroska PCM fixtures for decode-audio, trim, trim-pcm, and WAVE concat packet-budget, zero-packet, stream-0, and budget-note parity regressions (FVID-0249)."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_loudnorm_budget_mix_concat_regressions import (
    build_adts_48k,
    build_mp4_pcm_m4a,
)
from generate_loudnorm_mix_packet_budget_parity_regressions import (
    build_matroska_pcm_mka,
    build_wave_s16le_48k,
)


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/decode-trim-wave-budget-parity"


def build_second_wave_s16le_48k() -> bytes:
    """Second synthetic 48 kHz stereo 16-bit PCM WAVE (1920 sample frames, 4 bytes per frame)."""
    frames = 1920
    pcm = b"".join(
        struct.pack("<hh", 3072 if (i // 24) % 2 == 0 else -3072, -3072 if (i // 24) % 2 == 0 else 3072)
        for i in range(frames)
    )
    fmt = struct.pack("<HHIIHH", 1, 2, 48_000, 48_000 * 4, 4, 16)
    riff_payload = b"WAVEfmt " + struct.pack("<I", len(fmt)) + fmt + b"data" + struct.pack("<I", len(pcm)) + pcm
    return b"RIFF" + struct.pack("<I", len(riff_payload)) + riff_payload


def generate() -> dict[str, bytes]:
    files = {
        "stereo-48k.wav": build_wave_s16le_48k(),
        "second-stereo-48k.wav": build_second_wave_s16le_48k(),
        "adts-mono-48k.aac": build_adts_48k(),
        "mp4-pcm-48k.m4a": build_mp4_pcm_m4a(),
        "matroska-pcm-48k.mka": build_matroska_pcm_mka(),
    }
    manifest = {
        "batch_id": "FVID-0249",
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
