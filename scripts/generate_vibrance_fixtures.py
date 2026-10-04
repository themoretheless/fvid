#!/usr/bin/env python3
"""Generate three-frame 8x8 packed RGBA videos using only arithmetic samples."""
from pathlib import Path
import struct

root = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
for depth in (8, 16):
    values = [((i + frame * 17) * multiplier) % 256
              for frame in range(3) for i in range(64)
              for multiplier in (37, 71, 131, 11)]
    data = bytes(values) if depth == 8 else b''.join(struct.pack('<H', v * 257) for v in values)
    (root / f'vibrance-grid-{depth}.rgba').write_bytes(data)
