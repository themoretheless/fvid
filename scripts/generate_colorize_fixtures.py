#!/usr/bin/env python3
"""Write three-frame, odd-size planar YUV tint regressions without an encoder."""
from pathlib import Path
root = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
for depth in (8, 12, 16):
    maximum = (1 << depth) - 1
    frames = []
    for index in range(3):
        samples = [(maximum * n // 8 + index) % (maximum + 1) for n in range(9)]
        samples += [maximum // 4] * 4 + [3 * maximum // 4] * 4
        body = bytes(samples) if depth == 8 else b''.join(n.to_bytes(2, 'little') for n in samples)
        frames.append(b'FRAME\n' + body)
    layout = '420' if depth == 8 else f'420p{depth}'
    (root / f'colorize-grid-{depth}.y4m').write_bytes(f'YUV4MPEG2 W3 H3 F2:1 Ip A1:1 C{layout}\n'.encode() + b''.join(frames))
