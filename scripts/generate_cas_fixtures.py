#!/usr/bin/env python3
"""Three-frame CAS boundaries and flat black/white fixtures, without encoders."""
from pathlib import Path
root = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
for depth in (8, 12, 16):
    maximum = (1 << depth) - 1
    frames = []
    for frame in range(3):
        if frame < 2:
            samples = [frame * maximum] * 43
        else:
            samples = [((n * 37 + 19) % 256) * maximum // 255 for n in range(25)]
            samples += [((n * 79 + 11) % 256) * maximum // 255 for n in range(9)]
            samples += [((n * 43 + 101) % 256) * maximum // 255 for n in range(9)]
        body = bytes(samples) if depth == 8 else b''.join(n.to_bytes(2, 'little') for n in samples)
        frames.append(b'FRAME\n' + body)
    layout = '420' if depth == 8 else f'420p{depth}'
    (root / f'cas-grid-{depth}.y4m').write_bytes(f'YUV4MPEG2 W5 H5 F2:1 Ip A1:1 C{layout}\n'.encode() + b''.join(frames))
