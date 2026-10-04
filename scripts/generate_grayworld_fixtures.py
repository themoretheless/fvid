#!/usr/bin/env python3
"""Arithmetic linear-light float RGBA white-balance video, without an encoder."""
from pathlib import Path
import struct
root = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
frames = []
for frame in range(3):
    for y in range(8):
        for x in range(8):
            rgb = [(0.05 + ((x * (37 + c * 11) + y * (19 + c * 7) + frame * 31) % 256) / 255 * 2) * (0.3 + c * 0.45) for c in range(3)]
            if x == 0 and y == 0:
                rgb = [0., 0., 0.]
            alpha = ((x * 29 + y * 17 + frame * 23) % 256) / 255
            frames.extend(rgb + [alpha])
(root / 'grayworld-grid.rgba_f32').write_bytes(struct.pack('<' + 'f' * len(frames), *frames))

for depth in (8, 12, 16):
    maximum = (1 << depth) - 1
    output = []
    for frame in range(3):
        samples = [(n * 37 + frame * 11) * maximum // 255 % (maximum + 1) for n in range(25)]
        samples += [(n * 29 + frame * 43 + 17) * maximum // 255 % (maximum + 1) for n in range(9)]
        samples += [(n * 53 + frame * 31 + 91) * maximum // 255 % (maximum + 1) for n in range(9)]
        body = bytes(samples) if depth == 8 else b''.join(n.to_bytes(2, 'little') for n in samples)
        output.append(b'FRAME\n' + body)
    layout = '420' if depth == 8 else f'420p{depth}'
    (root / f'grayworld-gamut-{depth}.y4m').write_bytes(f'YUV4MPEG2 W5 H5 F2:1 Ip A1:1 C{layout}\n'.encode() + b''.join(output))
