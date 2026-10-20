#!/usr/bin/env python3
"""Authored three-frame Matroska clock regressions and exact dyadic time oracles."""
import hashlib
import json
import math
import struct
from fractions import Fraction
from pathlib import Path
from generate_audio_resample_window_fixture import ebml
from generate_vp9_chroma import frame

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/matroska-clock"


def nearest(value):
    sign = -1 if value < 0 else 1
    value = abs(value)
    q, r = divmod(value.numerator, value.denominator)
    return sign * (q + (2 * r >= value.denominator))


def clip(factor=1.0, repeated_scale=None, wire_factor=None, late_tracks=False, laced=False, block_duration=5):
    header = ebml('1a45dfa3', ebml('4282', b'matroska') + ebml('4287', b'\3') + ebml('4285', b'\2'))
    def info(scale):
        return ebml('1549a966', ebml('2ad7b1', scale.to_bytes(3, 'big'))
                    + ebml('4489', struct.pack('>d', 60 - 5 * factor))
                    + ebml('4d80', b'fvid-synthetic') + ebml('5741', b'fvid-synthetic'))
    track = ebml('d7', b'\1') + ebml('73c5', b'\1') + ebml('83', b'\1') + ebml('86', b'V_VP9')
    track += ebml('23e383', (10_000_000).to_bytes(4, 'big'))
    if wire_factor is not None:
        track += ebml('23314f', struct.pack('>d', wire_factor))
    track += ebml('e0', ebml('b0', b'\x10') + ebml('ba', b'\x10'))
    def group(relative, size):
        packet = frame(8, '420', *size)
        block = b'\x81' + struct.pack('>h', relative) + b'\0' + packet
        return ebml('a0', ebml('a1', block) + ebml('9b', bytes([block_duration])))
    first = ebml('1f43b675', ebml('e7', b'\0') + group(0, (16, 16)) + group(10, (32, 16)))
    if laced:
        a, b = frame(8, '420', 16, 16), frame(8, '420', 32, 16)
        size = b'\xff' * (len(a) // 255) + bytes([len(a) % 255])
        block = b'\x81\0\0\x82\1' + size + a + b
        first = ebml('1f43b675', ebml('e7', b'\0') + ebml('a3', block))
    second = ebml('1f43b675', ebml('e7', b'\x3c') + group(-10, (16, 32)))
    repeat = b'' if repeated_scale is None else info(repeated_scale)
    tracks = ebml('1654ae6b', ebml('ae', track))
    body = first + repeat + second
    return header + ebml('18538067', info(1_000_000) + (body + tracks if late_tracks else tracks + body))


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    files = {'control.mkv': clip(), 'same-segment-scale.mkv': clip(repeated_scale=1_000_000),
             'changing-segment-scale.mkv': clip(repeated_scale=2_000_000)}
    cases = []
    for name, factor in [('half', .5), ('one', 1.), ('three-halves', 1.5), ('double', 2.)]:
        file = f'track-scale-{name}.mkv'
        files[file] = clip(factor=factor, wire_factor=factor)
        exact = Fraction.from_float(factor)
        cases.append({'file': file, 'pts_ns': [nearest(Fraction(cluster * 1_000_000) + relative * exact * 1_000_000)
            for cluster, relative in [(0, 0), (0, 10), (60, -10)]],
            'duration_ns': nearest(5 * exact * 1_000_000), 'sizes': [[16, 16], [32, 16], [16, 32]]})
    invalid = []
    files['track-scale-late.mkv'] = clip(factor=2., wire_factor=2., late_tracks=True)
    cases.append({'file': 'track-scale-late.mkv', 'pts_ns': [0, 20_000_000, 40_000_000],
                  'duration_ns': 10_000_000, 'sizes': [[16, 16], [32, 16], [16, 32]]})
    files['track-scale-laced.mkv'] = clip(factor=2., wire_factor=2., laced=True)
    cases.append({'file': 'track-scale-laced.mkv', 'pts_ns': [0, 10_000_000, 40_000_000],
                  'duration_ns': 10_000_000, 'sizes': [[16, 16], [32, 16], [16, 32]]})
    for name, factor in [('zero', 0.), ('negative', -1.), ('nan', math.nan), ('infinity', math.inf)]:
        file = f'track-scale-invalid-{name}.mkv'
        files[file] = clip(wire_factor=factor)
        invalid.append(file)
    overflow = {'track-scale-timestamp-overflow.mkv': 'WebM timestamp overflow',
                'track-scale-duration-overflow.mkv': 'WebM block duration overflow'}
    files['track-scale-timestamp-overflow.mkv'] = clip(wire_factor=1e300, block_duration=0)
    files['track-scale-duration-overflow.mkv'] = clip(wire_factor=1e300)
    for name, data in files.items():
        (OUT / name).write_bytes(data)
    oracles = []
    for factor in [1., .5, 1.5, 2., .1, float.fromhex('0x1p-76'), float.fromhex('0x1p-100'), float.fromhex('0x0.0000000000001p-1022'), 1e300, float(2**63), float(2**64-2048)]:
        for cluster in [0, 1, 17, 2**53+1, 2**63-1, 2**64-2048]:
            for relative in [-32768, -1, 0, 1, 32767]:
                for scale in [1, 3, 1_000_000, 2**64-1]:
                    result = nearest((cluster + relative * Fraction.from_float(factor)) * scale)
                    oracles.append({'factor_bits': struct.unpack('>Q', struct.pack('>d', factor))[0],
                                    'cluster': cluster, 'relative': relative, 'scale': scale,
                                    'expected': result if -2**63 <= result < 2**63 else None})
    durations = []
    for factor in [.5, 1., 1.5, .1, float.fromhex('0x1p-76'), float.fromhex('0x1p-100'), 1e300, float.fromhex('0x0.0000000000001p-1022')]:
        for ticks in [0, 1, 3, 2**53+1, 2**64-1]:
            for scale in [1, 3, 1_000_000, 2**64-1]:
                result = nearest(ticks * Fraction.from_float(factor) * scale)
                durations.append({'factor_bits': struct.unpack('>Q', struct.pack('>d', factor))[0],
                                  'ticks': ticks, 'scale': scale,
                                  'expected': result if result < 2**64 else None})
    (OUT / 'manifest.json').write_text(json.dumps({
        'provenance': 'Authored VP9 keyframes and Matroska structures. Rational timestamp oracles follow RFC 9559 section 11.2; no private media, encoder or network.',
        'cases': cases, 'invalid': invalid, 'overflow': overflow,
        'timestamp_oracles': oracles, 'duration_oracles': durations,
        'files': {name: hashlib.sha256(data).hexdigest() for name, data in sorted(files.items())},
    }, indent=2) + '\n')


if __name__ == '__main__':
    main()
