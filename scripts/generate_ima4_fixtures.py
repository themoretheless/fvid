#!/usr/bin/env python3
"""Synthetic Apple IMA4 rising ramp; no external codecs or source media."""
from generate_pcm_precision_fixtures import ROOT, quicktime_pcm

if __name__ == '__main__':
    ROOT.mkdir(parents=True, exist_ok=True)
    # Predictor/index zero, low nibble then high nibble both 1: +2 per sample.
    packet = bytes(2) + bytes([0x11]) * 32
    (ROOT / 'ima4-ramp-edits.mov').write_bytes(quicktime_pcm(b'ima4', 16, packet, samples=64))
