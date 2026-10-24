#!/usr/bin/env python3
"""Generate deterministic synthetic fixtures for Batch 239 (`FVID-0246`).

Covers:
1. ADTS AAC (`adts-seg-a.aac`, `adts-seg-b.aac`) for `--streams 0` (`streams: [0]`),
   `--streams 1` refusal (`"ADTS has only stream 0"`), `--max-packets`,
   `--max-packet-bytes`, `--max-rss-mib`, and Matroska metadata mutations across
   `fvid::media::remux`, `fvid::media::plan_remux`, `fvid::media::concat`,
   `fvid::media::plan_concat`, and the `fvid media` CLI.
2. Compatible PCM WAVE (`wave-seg-a.wav`, `wave-seg-b.wav`) for `fvid media concat`
   and `fvid media plan concat` when targeting Matroska (`.mka` / `.mkv`) vs WAVE
   (`.wav`) with default options, stream selection, and Matroska metadata mutations.
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

from generate_audio_wave_metadata_mix_controls_regressions import build_tagged_wave_48k
from generate_loudnorm_budget_mix_concat_regressions import build_adts_48k


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/adts-wave-remux-concat-parity"


def generate_all(out_dir: Path = OUT) -> dict[str, bytes]:
    out_dir.mkdir(parents=True, exist_ok=True)
    adts_bytes = build_adts_48k()
    wave_bytes = build_tagged_wave_48k()
    fixtures = {
        "adts-seg-a.aac": adts_bytes,
        "adts-seg-b.aac": adts_bytes,
        "wave-seg-a.wav": wave_bytes,
        "wave-seg-b.wav": wave_bytes,
    }
    manifest_entries = []
    for name, data in fixtures.items():
        path = out_dir / name
        path.write_bytes(data)
        manifest_entries.append(
            {
                "name": name,
                "bytes": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
            }
        )
    manifest = {
        "batch": 239,
        "id": "FVID-0246",
        "fixtures": manifest_entries,
    }
    (out_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n",
        encoding="utf-8",
    )
    return fixtures


if __name__ == "__main__":
    generate_all()
