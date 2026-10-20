#!/usr/bin/env python3
"""Synthetic VP9 videos qualifying exact Segment Duration conversion."""
import hashlib
import json
import struct
from fractions import Fraction
from pathlib import Path
from generate_matroska_clock_regressions import clip, nearest

OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/matroska-duration'


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    cases = []
    for name, ticks, scale in [('double-rounding', 55.0000005, 1_000_000),
                               ('integer-product', float(2**53-1), 3),
                               ('representable-upper-bound', 18446744073709.55, 1_000_000)]:
        data = clip()
        for tag, replacement in [(bytes.fromhex('448910000008'), struct.pack('>d', ticks)),
                                  (bytes.fromhex('2ad7b110000003'), scale.to_bytes(3, 'big'))]:
            assert data.count(tag) == 1
            offset = data.index(tag) + len(tag)
            data = data[:offset] + replacement + data[offset+len(replacement):]
        file = name + '.mkv'
        (OUT / file).write_bytes(data)
        cases.append({'file': file, 'ticks': ticks, 'scale': scale,
                      'expected_ns': nearest(Fraction.from_float(ticks) * scale),
                      'sha256': hashlib.sha256(data).hexdigest()})
    (OUT / 'manifest.json').write_text(json.dumps({
        'provenance': 'Authored three VP9 keyframes. Only Info Duration and TimestampScale differ. Exact Fraction oracles; no private media or external encoder.',
        'qualification': 'Upper-bound cases exercise declared metadata, not playback of a year-long clip.',
        'cases': cases,
    }, indent=2) + '\n')


if __name__ == '__main__':
    main()
