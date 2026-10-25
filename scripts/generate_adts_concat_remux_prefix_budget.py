#!/usr/bin/env python3
"""Generate synthetic ADTS fixtures for Batch 271 (FVID-0278).

Fixtures in `tests/fixtures/playback-errors/adts-concat-remux-prefix-budget/`:
1. `seg-a-small.aac`:
   2 ADTS frames (48 kHz mono AAC-LC, 8-byte payload each)
2. `seg-b-large.aac`:
   1 ADTS frame (48 kHz mono AAC-LC, 35-byte payload)
3. `seg-c-invalid-rate.aac`:
   1 ADTS frame (44.1 kHz mono AAC-LC, 8-byte payload)
4. `sbr-24k-to-48k.aac`:
   2 ADTS frames (24 kHz mono AAC-LC with implicit SBR in frame 0 negotiating to 48 kHz,
   and 35-byte payload in frame 1)
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import sys

SCRIPTS_DIR = Path(__file__).resolve().parent
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

from generate_aac_ps_fixtures import ps as build_ps_bits, sbr as build_sbr_fill
from generate_aac_sbr_dsp_fixtures import payload as build_sbr_payload
from generate_aac_sbr_frequency_oracles import tables
from generate_aac_source_info_prefix_budget import adts_frame
from generate_he_aac_packet_fixtures import packet as build_sbr_packet
from generate_mp4_audio_selection_regressions import build_aac_pulse_packet


def build_sbr_48000_packet(stage: int = 0) -> bytes:
    # For sbr_rate = 48000 (24000 Hz core rate doubled), start=2, stop=3 gives k0=9, k2=22.
    high_bands = len(tables(9, 22, 0, False, 0, 0)[1]) - 1
    sbr_raw = build_sbr_payload(high_bands, 2, True, stage)
    return build_sbr_packet(sbr_raw)


def build_large_packet(stage: int = 0) -> bytes:
    ps_bits, _ = build_ps_bits(1, 1, stage)
    sbr_raw = build_sbr_fill(ps_bits, stage)
    return build_sbr_packet(sbr_raw)


def generate_fixtures(out_dir: Path) -> dict:
    out_dir.mkdir(parents=True, exist_ok=True)

    # 1. seg-a-small.aac: 2 frames of 48 kHz (freq_index=3) mono, 8-byte payload
    pkt_small = build_aac_pulse_packet(140, 1, 7)
    assert len(pkt_small) == 8, f"expected 8-byte payload, got {len(pkt_small)}"
    seg_a = adts_frame(pkt_small, freq_index=3, channels=1) + adts_frame(
        pkt_small, freq_index=3, channels=1
    )
    seg_a_path = out_dir / "seg-a-small.aac"
    seg_a_path.write_bytes(seg_a)

    # 2. seg-b-large.aac: 1 frame of 48 kHz (freq_index=3) mono, 35-byte payload
    pkt_large = build_large_packet(0)
    assert len(pkt_large) > 24, f"expected >24-byte payload, got {len(pkt_large)}"
    seg_b = adts_frame(pkt_large, freq_index=3, channels=1)
    seg_b_path = out_dir / "seg-b-large.aac"
    seg_b_path.write_bytes(seg_b)

    # 3. seg-c-invalid-rate.aac: 1 frame of 44.1 kHz (freq_index=4) mono, 8-byte payload
    seg_c = adts_frame(pkt_small, freq_index=4, channels=1)
    seg_c_path = out_dir / "seg-c-invalid-rate.aac"
    seg_c_path.write_bytes(seg_c)

    # 4. sbr-24k-to-48k.aac: 2 frames of 24 kHz (freq_index=6) mono with implicit SBR in frame 0 (<=24 bytes)
    # and >24-byte packet in frame 1
    pkt_sbr0 = build_sbr_48000_packet(0)
    assert len(pkt_sbr0) <= 24, f"expected <=24-byte SBR payload, got {len(pkt_sbr0)}"
    pkt_sbr1 = build_large_packet(0)
    assert len(pkt_sbr1) > 24, f"expected >24-byte payload, got {len(pkt_sbr1)}"
    sbr_stream = adts_frame(pkt_sbr0, freq_index=6, channels=1) + adts_frame(
        pkt_sbr1, freq_index=6, channels=1
    )
    sbr_path = out_dir / "sbr-24k-to-48k.aac"
    sbr_path.write_bytes(sbr_stream)

    manifest = {
        "generator": "scripts/generate_adts_concat_remux_prefix_budget.py",
        "files": {},
    }
    for path in (seg_a_path, seg_b_path, seg_c_path, sbr_path):
        data = path.read_bytes()
        manifest["files"][path.name] = {
            "bytes": len(data),
            "sha256": hashlib.sha256(data).hexdigest(),
        }
    (out_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    return manifest


def main() -> None:
    repo_root = Path(__file__).resolve().parent.parent
    out_dir = repo_root / "tests" / "fixtures" / "playback-errors" / "adts-concat-remux-prefix-budget"
    generate_fixtures(out_dir)


if __name__ == "__main__":
    main()
