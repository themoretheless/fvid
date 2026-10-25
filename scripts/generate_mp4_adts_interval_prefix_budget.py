#!/usr/bin/env python3
"""Generate synthetic MP4 and ADTS fixtures for Batch 272 (FVID-0279).

Fixtures in `tests/fixtures/playback-errors/mp4-adts-interval-prefix-budget/`:
1. `mp4-mono-then-ps.m4a`:
   2 packets of 48 kHz mono HE-AAC (2048 samples / 42667 us per packet).
   Packet 0 is an 8-byte mono AAC pulse block (no PS).
   Packet 1 is a 35-byte SBR + in-band PS block.
2. `adts-small-then-large.aac`:
   2 frames of 48 kHz mono AAC-LC (1024 samples / 21334 us per frame).
   Frame 0 has an 8-byte payload (15-byte ADTS frame).
   Frame 1 has a 35-byte payload (42-byte ADTS frame).
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import sys

SCRIPTS_DIR = Path(__file__).resolve().parent
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

from generate_aac_source_info_prefix_budget import (
    adts_frame,
    build_mp4_mono_then_ps,
    build_ps_packet,
)
from generate_mp4_audio_selection_regressions import build_aac_pulse_packet


def generate_fixtures(out_dir: Path) -> dict:
    out_dir.mkdir(parents=True, exist_ok=True)

    mp4_bytes = build_mp4_mono_then_ps()
    mp4_path = out_dir / "mp4-mono-then-ps.m4a"
    mp4_path.write_bytes(mp4_bytes)

    pkt_0 = build_aac_pulse_packet(140, 1, 7)
    pkt_1 = build_ps_packet()
    assert len(pkt_0) == 8 and len(pkt_1) > 24
    adts_bytes = adts_frame(pkt_0, freq_index=3, channels=1) + adts_frame(
        pkt_1, freq_index=3, channels=1
    )
    adts_path = out_dir / "adts-small-then-large.aac"
    adts_path.write_bytes(adts_bytes)

    manifest = {
        "generator": "scripts/generate_mp4_adts_interval_prefix_budget.py",
        "batch_id": "FVID-0279",
        "files": {},
    }
    for path in (mp4_path, adts_path):
        data = path.read_bytes()
        manifest["files"][path.name] = {
            "bytes": len(data),
            "sha256": hashlib.sha256(data).hexdigest(),
        }
    (out_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    return manifest


def main() -> None:
    repo_root = Path(__file__).resolve().parent.parent
    out_dir = (
        repo_root
        / "tests"
        / "fixtures"
        / "playback-errors"
        / "mp4-adts-interval-prefix-budget"
    )
    generate_fixtures(out_dir)


if __name__ == "__main__":
    main()
