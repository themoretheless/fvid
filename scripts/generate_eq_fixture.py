#!/usr/bin/env python3
"""Synthetic one-frame equalization ramps; no external codecs."""
from pathlib import Path
root = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
samples = bytes(range(256)) + bytes(reversed(range(256))) + bytes(v ^ 85 for v in range(256))
(root / 'eq-ramp.y4m').write_bytes(b'YUV4MPEG2 W16 H16 F30:1 Ip C444\nFRAME\n'+samples)
