#!/usr/bin/env python3
"""Authored VP9 clips with declared/undeclared presentation tails."""
import hashlib
import json
import struct
from pathlib import Path
from generate_audio_resample_window_fixture import ebml
from generate_vp9_chroma import frame

OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/matroska-tail'


def clip(times, durations, default, declared):
    header = ebml('1a45dfa3', ebml('4282', b'matroska') + ebml('4287', b'\3') + ebml('4285', b'\2'))
    info = ebml('2ad7b1', (1_000_000).to_bytes(3, 'big'))
    if declared is not None:
        info += ebml('4489', struct.pack('>d', declared / 1_000_000))
    track = ebml('d7', b'\1') + ebml('73c5', b'\1') + ebml('83', b'\1') + ebml('86', b'V_VP9')
    if default is not None:
        track += ebml('23e383', default.to_bytes(4, 'big'))
    track += ebml('e0', ebml('b0', b'\x10') + ebml('ba', b'\x10'))
    blocks = ebml('e7', b'\0')
    for i, (time, duration) in enumerate(zip(times, durations)):
        payload = b'\x81' + struct.pack('>h', time // 1_000_000)
        picture = frame(8, '420', *[[16, 16], [32, 16], [16, 32]][i])
        if duration is None:
            blocks += ebml('a3', payload + b'\x80' + picture)
        else:
            blocks += ebml('a0', ebml('a1', payload + b'\0' + picture)
                           + ebml('9b', (duration // 1_000_000).to_bytes(2, 'big')))
    return header + ebml('18538067', ebml('1549a966', info)
                         + ebml('1654ae6b', ebml('ae', track)) + ebml('1f43b675', blocks))


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    cases, files = [], {}
    for name, times, durations, default, expected in [
            ('explicit-tail', [0, 20_000_000, 40_000_000], [20_000_000, 20_000_000, 200_000_000], 10_000_000, 240_000_000),
            ('overlapping-explicit', [70_000_000, 90_000_000, 110_000_000], [200_000_000, 5_000_000, 10_000_000], None, 50_000_000),
            ('default-tail', [0, 20_000_000, 40_000_000], [None, None, None], 10_000_000, 50_000_000),
            ('single-default', [0], [None], 100_000_000, 100_000_000),
            ('inferred-gap', [0, 20_000_000, 40_000_000], [None, None, None], None, 60_000_000),
            ('previous-explicit', [0, 20_000_000, 40_000_000], [5_000_000, 5_000_000, None], None, 45_000_000),
            ('single-unknown', [0], [None], None, 33_333_333),
            ('offset-gap', [70_000_000, 90_000_000, 110_000_000], [None, None, None], None, 60_000_000)]:
        origin = times[0]
        intervals = []
        for i, start in enumerate(times):
            if i+1 == len(times):
                end = expected + origin
            elif durations[i] is None:
                end = times[i+1]
            else:
                end = min(start+durations[i], times[i+1])
            intervals.append([start-origin, end-origin])
        for suffix, declared in [('inferred', None), ('declared', expected + origin)]:
            files[f'{name}-{suffix}.mkv'] = clip(times, durations, default, declared)
        cases.append({'inferred': f'{name}-inferred.mkv', 'declared': f'{name}-declared.mkv',
                      'expected_ns': expected, 'origin_ns': origin, 'intervals': intervals,
                      'sizes': [[16, 16], [32, 16], [16, 32]][:len(times)]})
    for name, data in files.items():
        (OUT / name).write_bytes(data)
    (OUT / 'manifest.json').write_text(json.dumps({
        'provenance': 'Authored short neutral VP9 clips. Paired files differ only in Info Duration; no private media, network or encoder.',
        'cases': cases, 'files': {name: hashlib.sha256(data).hexdigest() for name, data in sorted(files.items())},
    }, indent=2) + '\n')


if __name__ == '__main__':
    main()
