#!/usr/bin/env python3
"""Generate synthetic WAVE, ADTS AAC, MP4 PCM, Matroska PCM, and Matroska FLAC fixtures for loudness, normalize-loudness, and loudnorm packet-budget, zero-packet, metadata, and budget-note parity regressions (FVID-0250)."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

from generate_loudnorm_budget_mix_concat_regressions import (
    build_adts_48k,
    build_matroska_flac_mka,
    build_mp4_pcm_m4a,
)
from generate_loudnorm_mix_packet_budget_parity_regressions import (
    build_matroska_pcm_mka,
    build_wave_s16le_48k,
)


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/loudness-normalize-budget-parity"


def generate() -> dict[str, bytes]:
    files = {
        "stereo-48k.wav": build_wave_s16le_48k(),
        "adts-mono-48k.aac": build_adts_48k(),
        "mp4-pcm-48k.m4a": build_mp4_pcm_m4a(),
        "matroska-pcm-48k.mka": build_matroska_pcm_mka(),
        "matroska-flac-48k.mka": build_matroska_flac_mka(),
    }
    manifest = {
        "batch_id": "FVID-0250",
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
