#!/usr/bin/env python3
"""Synthetic Apple IMA4 rising ramp; no external codecs or source media."""
from generate_pcm_precision_fixtures import ROOT, quicktime_pcm
import struct

if __name__ == '__main__':
    ROOT.mkdir(parents=True, exist_ok=True)
    # Predictor/index zero, low nibble then high nibble both 1: +1 per sample.
    packet = bytes(2) + bytes([0x11]) * 32
    (ROOT / 'ima4-ramp-edits.mov').write_bytes(quicktime_pcm(b'ima4', 16, packet, samples=64))
    fmt = struct.pack('<HHIIHHHH', 17, 2, 48000, 48000 * 16 // 9, 16, 4, 2, 9)
    stereo = bytes(4) + struct.pack('<hBB', -128, 0, 0) + bytes([0x11]) * 4 + bytes(4)
    (ROOT / 'ima-wav-stereo-edits.mov').write_bytes(quicktime_pcm(
        bytes([109, 115, 0, 17]), 16, stereo, samples=9, channels=2, configuration=fmt))
    adaptive = struct.pack('>H', 1024 | 20) + bytes((i * 73 + 19) & 255 for i in range(32))
    adaptive += struct.pack('>H', (65536 - 1024) | 60) + bytes((i * 31 + 237) & 255 for i in range(32))
    (ROOT / 'ima4-adaptive-stereo-edits.mov').write_bytes(quicktime_pcm(
        b'ima4', 16, adaptive, samples=64, channels=2))
