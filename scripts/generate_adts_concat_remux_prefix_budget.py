#!/usr/bin/env python3
"""Generate synthetic ADTS fixtures for Batch 271 (FVID-0278).

Fixtures in `tests/fixtures/playback-errors/adts-concat-remux-prefix-budget/`:
1. `seg-a-small.aac`:
   2 ADTS frames (48 kHz mono AAC-LC, 8-byte payload each)
2. `seg-b-large.aac`:
   1 ADTS frame (48 kHz mono AAC-LC, 32-byte payload with FIL padding)
3. `seg-c-invalid-rate.aac`:
   1 ADTS frame (44.1 kHz mono AAC-LC, 8-byte payload)
4. `sbr-24k-to-48k.aac`:
   2 ADTS frames (24 kHz mono AAC-LC with implicit SBR FIL in frame 0 negotiating to 48 kHz,
   and 40-byte payload in frame 1)
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import sys

SCRIPTS_DIR = Path(__file__).resolve().parent
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

from generate_adts_loudness_prefix_sbr import BitWriter, adts_frame
from generate_aac_ps_probe_packet_limit import build_mono_lc_packet


def build_sbr_24k_to_48k_packet(amp: int = 80, pad_bytes: int = 0) -> bytes:
    """Build a 24 kHz mono AAC-LC raw data block with an implicit SBR FIL (48 kHz output)."""
    from generate_aac_ps_probe_packet_limit import write_mono_sce, write_sbr_Header_and_single_channel

    sbr = BitWriter()
    sbr.write(1, 1)  # bs_header_flag = 1
    write_sbr_Header_and_single_channel(sbr, ps_flag=False)
    sbr.byte_align()
    if len(sbr.bytes) > 14:
        raise ValueError(f"SBR payload too large for 4-bit FIL count: {len(sbr.bytes)}")

    w = BitWriter()
    write_mono_sce(w, amp=amp)
    # ID_FIL (6) with 4-bit count = 1 + len(sbr.bytes)
    count = 1 + len(sbr.bytes)
    w.write(6, 3)
    w.write(count, 4)
    w.write(13, 4)  # EXT_SBR_DATA
    for byte in sbr.bytes:
        w.write(byte, 8)
    while pad_bytes > 0:
        chunk = min(pad_bytes, 14)
        w.write(6, 3)
        w.write(chunk, 4)
        for _ in range(chunk):
            w.write(0, 8)
        pad_bytes -= chunk
    w.write(7, 3)  # ID_END
    w.byte_align()
    return bytes(w.bytes)


def generate_fixtures(out_dir: Path) -> dict:
    out_dir.mkdir(parents=True, exist_ok=True)

    # 1. seg-a-small.aac: 2 frames of 48 kHz (freq_index=3) mono, 8-byte payload
    pkt_small = build_mono_lc_packet(amp=80, pad_bytes=0)
    assert len(pkt_small) == 8, f"expected 8-byte payload, got {len(pkt_small)}"
    seg_a = adts_frame(pkt_small, freq_index=3, chan_config=1) + adts_frame(
        pkt_small, freq_index=3, chan_config=1
    )
    seg_a_path = out_dir / "seg-a-small.aac"
    seg_a_path.write_bytes(seg_a)

    # 2. seg-b-large.aac: 1 frame of 48 kHz (freq_index=3) mono, >24-byte payload
    pkt_large = build_mono_lc_packet(amp=80, pad_bytes=24)
    assert len(pkt_large) > 24, f"expected >24-byte payload, got {len(pkt_large)}"
    seg_b = adts_frame(pkt_large, freq_index=3, chan_config=1)
    seg_b_path = out_dir / "seg-b-large.aac"
    seg_b_path.write_bytes(seg_b)

    # 3. seg-c-invalid-rate.aac: 1 frame of 44.1 kHz (freq_index=4) mono, 8-byte payload
    seg_c = adts_frame(pkt_small, freq_index=4, chan_config=1)
    seg_c_path = out_dir / "seg-c-invalid-rate.aac"
    seg_c_path.write_bytes(seg_c)

    # 4. sbr-24k-to-48k.aac: 2 frames of 24 kHz (freq_index=6) mono with implicit SBR
    pkt_sbr0 = build_sbr_24k_to_48k_packet(amp=80, pad_bytes=0)
    pkt_sbr1 = build_sbr_24k_to_48k_packet(amp=80, pad_bytes=24)
    sbr_stream = adts_frame(pkt_sbr0, freq_index=6, chan_config=1) + adts_frame(
        pkt_sbr1, freq_index=6, chan_config=1
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
