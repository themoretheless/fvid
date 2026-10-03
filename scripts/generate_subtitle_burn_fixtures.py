#!/usr/bin/env python3
"""Create three constant YUV frames and a half-open SRT cue without external tools."""
from pathlib import Path
root = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
frame = bytes([32]) * (96 * 64) + bytes([64]) * (2 * 48 * 32)
(root / 'subtitle-burn.y4m').write_bytes(b'YUV4MPEG2 W96 H64 F2:1 Ip A1:1 C420\n' + (b'FRAME\n' + frame) * 3)
(root / 'subtitle-burn.srt').write_text('1\n00:00:00,500 --> 00:00:01,000\nFVid\n', encoding='utf-8')
