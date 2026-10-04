#!/usr/bin/env python3
"""Synthetic corner impulse; no external codecs or private media."""
from pathlib import Path
root = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
plane = bytes([90]) + bytes(80)
(root / 'boxblur-expression-9.y4m').write_bytes(
    b'YUV4MPEG2 W9 H9 F25:1 Ip C444\n' +
    (b'FRAME\n' + plane + bytes([40])*162)*3)
