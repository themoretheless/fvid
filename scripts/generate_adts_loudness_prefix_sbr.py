#!/usr/bin/env python3
"""Generate synthetic ADTS fixtures for FVID-0277 loudness/loudnorm prefix & SBR negotiation."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

from generate_aac_ps_fixtures import ps as build_ps_bits, sbr as build_sbr_fill
from generate_aac_sbr_dsp_fixtures import payload as build_sbr_payload
from generate_aac_sbr_frequency_oracles import tables
from generate_aac_source_info_prefix_budget import adts_frame
from generate_he_aac_packet_fixtures import packet as build_sbr_packet
from generate_mp4_audio_selection_regressions import build_aac_pulse_packet


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/adts-loudness-prefix-sbr"


def build_sbr_22050_packet(stage: int) -> bytes:
    # For sbr_rate = 22050 (11025 Hz core rate doubled), start=2, stop=3 gives k0=14, k2=40.
    high_bands = len(tables(14, 40, 0, False, 0, 0)[1]) - 1
    sbr_raw = build_sbr_payload(high_bands, 2, True, stage)
    return build_sbr_packet(sbr_raw)


def build_ps_packet(stage: int = 0) -> bytes:
    ps_bits, _ = build_ps_bits(1, 1, stage)
    sbr_raw = build_sbr_fill(ps_bits, stage)
    return build_sbr_packet(sbr_raw)


def build_sbr_11k_to_22k_adts() -> bytes:
    # 11025 Hz core rate in ADTS header (freq_index = 10, channels = 1).
    # Every packet has implicit SBR expanding output clock to 22050 Hz (qualified_rate = True).
    # 12 packets = 12 * 2048 = 24576 samples (> 0.4 s at 22050 Hz = 8820 samples).
    frames = []
    for i in range(12):
        pkt = build_sbr_22050_packet(0 if i == 0 else 2)
        frames.append(adts_frame(pkt, freq_index=10, channels=1))
    return b"".join(frames)


def build_lc_then_ps_24k_adts() -> bytes:
    # 24000 Hz core rate in ADTS header (freq_index = 6, channels = 1).
    # Packet 0: 8-byte mono AAC-LC pulse packet.
    # Packet 1: 35-byte SBR + in-band PS packet (expands output to 48000 Hz stereo).
    pkt_0 = build_aac_pulse_packet(140, 1, 7)
    pkt_1 = build_ps_packet(0)
    assert len(pkt_0) == 8 and len(pkt_1) > 16
    return adts_frame(pkt_0, freq_index=6, channels=1) + adts_frame(
        pkt_1, freq_index=6, channels=1
    )


def generate() -> dict[str, bytes]:
    files = {
        "sbr-11k-to-22k.aac": build_sbr_11k_to_22k_adts(),
        "lc-then-ps-24k.aac": build_lc_then_ps_24k_adts(),
    }
    manifest = {
        "generator": "scripts/generate_adts_loudness_prefix_sbr.py",
        "batch_id": "FVID-0277",
        "provenance": "100% synthetic ADTS fixtures: sbr-11k-to-22k.aac (11025 Hz core clock with implicit SBR expanding to 22050 Hz) and lc-then-ps-24k.aac (8-byte mono LC packet 0 followed by 35-byte SBR+PS packet 1); no FFmpeg, network, or private media.",
        "sha256": {name: hashlib.sha256(data).hexdigest() for name, data in files.items()},
    }
    files["manifest.json"] = (
        json.dumps(manifest, indent=2, sort_keys=True) + "\n"
    ).encode("utf-8")
    return files


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)


if __name__ == "__main__":
    main()
