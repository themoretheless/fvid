#!/usr/bin/env python3
"""Generate synthetic ADTS AAC and WAVE fixtures for Batch 217 (FVID-0224):
Matroska container and stream metadata mutations across ADTS remux/concat and
Matroska PCM audio decode-audio/trim.
"""

from __future__ import annotations

import pathlib

from generate_audio_wave_metadata_mix_controls_regressions import build_tagged_wave_48k
from generate_loudnorm_budget_mix_concat_regressions import build_adts_48k

ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT_DIR = ROOT / "tests" / "fixtures" / "playback-errors" / "adts-matroska-audio-metadata"


def generate_all(out_dir: pathlib.Path = OUT_DIR) -> dict[str, bytes]:
    out_dir.mkdir(parents=True, exist_ok=True)
    adts_frame_stream = build_adts_48k()
    files = {
        "adts-seg-a-48k.aac": adts_frame_stream,
        "adts-seg-b-48k.aac": adts_frame_stream,
        "pcm-source-48k.wav": build_tagged_wave_48k(),
    }
    for name, payload in files.items():
        (out_dir / name).write_bytes(payload)
    return files


if __name__ == "__main__":
    generate_all()
