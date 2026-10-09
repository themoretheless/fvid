#!/usr/bin/env python3
"""Own spectral-band selection oracle; generation is separate from tests."""
import json
from pathlib import Path
root = Path(__file__).resolve().parent.parent / 'tests/fixtures/playback-errors'
cases = []
for n in (960, 1024):
    offsets = [0, 1, 7, 16, 33, 64, 127, 511, n]
    residual = [(i % 29 - 14) * 0.25 for i in range(n)]
    prediction = [(i % 17 - 8) * 0.125 for i in range(n)]
    for mask in range(256):
        # Per-bin membership in a union of selected half-open intervals.
        selected = [(offsets[b], offsets[b+1]) for b in range(8) if mask & (1 << b)]
        expected = [r + prediction[i] if any(lo <= i < hi for lo, hi in selected) else r
                    for i, r in enumerate(residual)]
        cases.append(dict(n=n, mask=mask, offsets=offsets, expected=expected))
(root / 'aac-ltp-bands.json').write_text(json.dumps(dict(cases=cases), separators=(',', ':')) + '\n')
print('generated', len(cases), 'own band-selection cases')
