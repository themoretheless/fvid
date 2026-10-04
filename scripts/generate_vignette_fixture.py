#!/usr/bin/env python3
"""Synthetic shading input only; reference bytes come from the explicit benchmark."""
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
ROOT.mkdir(parents=True, exist_ok=True)
frames = [bytes((i * 37 + n * 23 + 101) % 256 for i in range(288)) for n in range(4)]
(ROOT / 'vignette-dither.y4m').write_bytes(
    b'YUV4MPEG2 W16 H12 F25:1 Ip A1:1 C420jpeg\n' +
    b''.join(b'FRAME\n' + frame for frame in frames))

(ROOT / 'vignette-fractional.rgb24').write_bytes(b''.join(bytes((i * 37 + n * 23 + 101) % 256 for i in range(576)) for n in range(4)))
