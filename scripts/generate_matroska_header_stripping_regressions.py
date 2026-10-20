#!/usr/bin/env python3
"""Authored Matroska frame-compression controls, without external encoders."""
import hashlib
import bz2
import json
import struct
import zlib
from pathlib import Path
from generate_audio_resample_window_fixture import ebml
from generate_vp9_chroma import frame

OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/matroska-header-stripping'


def encoding(prefix, implicit=False, algorithm=3, omit_algorithm=False, empty_scope=False):
    compression = b'' if omit_algorithm else ebml('4254', bytes([algorithm]))
    if prefix:
        compression += ebml('4255', prefix)
    fields = b'' if implicit else ebml('5031', b'\0') + ebml('5032', b'' if empty_scope else b'\1') + ebml('5033', b'\0')
    return ebml('6d80', ebml('6240', fields + ebml('5034', compression)))


def clip(prefix_length=0, laced=False, encoded=False, implicit=False, algorithm=3, identical=False, late_tracks=False, omit_algorithm=False, empty_scope=False):
    pictures = [frame(8, '420', *size) for size in ([(16, 16)] * 3 if identical else [(16, 16), (32, 16), (16, 32)])]
    if prefix_length == -1:
        prefix_length = len(pictures[0])
    prefix = pictures[0][:prefix_length]
    assert all(picture.startswith(prefix) for picture in pictures)
    payloads = [(zlib.compress(picture) if algorithm == 0 else bz2.compress(picture)) if algorithm in (0, 1) and encoded else picture[len(prefix):]
                for picture in pictures]
    if algorithm == 0 and encoded:
        assert all(zlib.decompress(data) == picture for data, picture in zip(payloads, pictures))
    header = ebml('1a45dfa3', ebml('4282', b'matroska') + ebml('4287', b'\3') + ebml('4285', b'\2'))
    info = ebml('1549a966', ebml('2ad7b1', (1_000_000).to_bytes(3, 'big')) + ebml('4489', struct.pack('>d', 55.)))
    track = ebml('d7', b'\1') + ebml('73c5', b'\1') + ebml('83', b'\1') + ebml('86', b'V_VP9')
    track += ebml('23e383', (10_000_000).to_bytes(4, 'big'))
    track += ebml('e0', ebml('b0', b'\x10') + ebml('ba', b'\x10'))
    if encoded:
        track += encoding(prefix, implicit, algorithm, omit_algorithm, empty_scope)
    def group(time, payload):
        block = b'\x81' + struct.pack('>h', time) + b'\0' + payload
        return ebml('a0', ebml('a1', block) + ebml('9b', b'\5'))
    if laced:
        size = len(payloads[0])
        lace_size = b'\xff' * (size // 255) + bytes([size % 255])
        first = ebml('a3', b'\x81\0\0\x82\1' + lace_size + payloads[0] + payloads[1])
    else:
        first = group(0, payloads[0]) + group(10, payloads[1])
    clusters = ebml('1f43b675', ebml('e7', b'\0') + first)
    clusters += ebml('1f43b675', ebml('e7', b'\x3c') + group(-10, payloads[2]))
    tracks = ebml('1654ae6b', ebml('ae', track))
    return header + ebml('18538067', info + (clusters + tracks if late_tracks else tracks + clusters))


def opus_lace_clip(encoded, prefix_length=1, algorithm=3):
    header = ebml('1a45dfa3', ebml('4282', b'matroska') + ebml('4287', b'\3') + ebml('4285', b'\2'))
    info = ebml('1549a966', ebml('2ad7b1', (1_000_000).to_bytes(3, 'big')) + ebml('4489', struct.pack('>d', 60.)))
    video = ebml('d7', b'\1') + ebml('73c5', b'\1') + ebml('83', b'\1') + ebml('86', b'V_VP9')
    video += ebml('23e383', (20_000_000).to_bytes(4, 'big')) + ebml('e0', ebml('b0', b'\x10') + ebml('ba', b'\x10'))
    # Minimal synthetic CELT packets and an authored mono mapping-family-zero header.
    cookie = b'OpusHead\1\1' + struct.pack('<HIhB', 0, 48000, 0, 0)
    audio = ebml('d7', b'\2') + ebml('73c5', b'\2') + ebml('83', b'\2') + ebml('86', b'A_OPUS')
    audio += ebml('63a2', cookie) + ebml('e1', ebml('b5', struct.pack('>d', 48000.)) + ebml('9f', b'\1'))
    if encoded:
        audio += encoding(b'\xf8\xff\xfe'[:prefix_length] if algorithm == 3 else b'', algorithm=algorithm)
    blocks = ebml('e7', b'\0')
    for time, size in zip([0, 20, 40], [(16, 16), (32, 16), (16, 32)]):
        blocks += ebml('a3', b'\x81' + struct.pack('>h', time) + b'\x80' + frame(8, '420', *size))
    packet = (zlib.compress(b'\xf8\xff\xfe') if algorithm == 0 else b'\xf8\xff\xfe'[prefix_length:]) if encoded else b'\xf8\xff\xfe'
    blocks += ebml('a3', b'\x82\0\0\x84\2' + packet * 3)
    return header + ebml('18538067', info + ebml('1654ae6b', ebml('ae', video) + ebml('ae', audio))
                         + ebml('1f43b675', blocks))


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    files = {'control.mkv': clip(), 'laced-control.mkv': clip(laced=True),
             'identical-control.mkv': clip(identical=True), 'identical-laced-control.mkv': clip(laced=True, identical=True),
             'unsupported-zlib.mkv': clip(encoded=True, algorithm=0),
             'unsupported-bzip2.mkv': clip(encoded=True, algorithm=1),
             'opus-control.mkv': opus_lace_clip(False), 'opus-laced.mkv': opus_lace_clip(True),
             'opus-whole.mkv': opus_lace_clip(True, 3)}
    cases = []
    for name, length, laced, implicit in [('one-byte', 1, False, False), ('implicit-defaults', 4, False, True),
                                         ('empty-prefix', 0, False, True), ('laced', 4, True, False)]:
        file = name + '.mkv'
        files[file] = clip(length, laced, True, implicit)
        cases.append({'file': file, 'control': 'laced-control.mkv' if laced else 'control.mkv',
                      'prefix_length': length, 'packet_count': 3, 'probe_packet': 0,
                      'pts_ns': [0, 10_000_000, 50_000_000], 'sizes': [[16, 16], [32, 16], [16, 32]]})
    for name, laced, late in [('whole-frame', False, False), ('whole-laced', True, False), ('whole-late-track', False, True)]:
        file = name + '.mkv'
        files[file] = clip(-1, laced, True, identical=True, late_tracks=late)
        cases.append({'file': file, 'control': 'identical-laced-control.mkv' if laced else 'identical-control.mkv',
                      'prefix_length': len(frame(8, '420', 16, 16)), 'packet_count': 3, 'probe_packet': 0,
                      'pts_ns': [0, 10_000_000, 50_000_000], 'sizes': [[16, 16]] * 3})
    cases.append({'file': 'opus-laced.mkv', 'control': 'opus-control.mkv', 'prefix_length': 1,
                  'packet_count': 6, 'probe_packet': 3, 'pts_ns': [0, 20_000_000, 40_000_000],
                  'sizes': [[16, 16], [32, 16], [16, 32]]})
    cases.append({'file': 'opus-whole.mkv', 'control': 'opus-control.mkv', 'prefix_length': 3,
                  'packet_count': 6, 'probe_packet': 3, 'pts_ns': [0, 20_000_000, 40_000_000],
                  'sizes': [[16, 16], [32, 16], [16, 32]]})
    block = ebml('a1', b'\x81\0\0\0')
    assert files['whole-frame.mkv'].count(block) == 1
    files['missing-lace-count.mkv'] = files['whole-frame.mkv'].replace(block, ebml('a1', b'\x81\0\0\2'), 1)
    for name, data in files.items():
        (OUT / name).write_bytes(data)
    (OUT / 'manifest.json').write_text(json.dumps({
        'provenance': 'Authored VP9 keyframes. RFC 9559 section 5.1.4.1.31.6 ContentCompAlgo=3 strips only an original synthetic common prefix from each frame. No private media or codec parameters.',
        'cases': cases, 'unsupported': ['unsupported-bzip2.mkv'],
        'formerly_unsupported_now_accepted': ['unsupported-zlib.mkv'],
        'malformed': {'missing-lace-count.mkv': 'truncated Matroska lace header'},
        'files': {name: hashlib.sha256(data).hexdigest() for name, data in sorted(files.items())},
    }, indent=2) + '\n')


if __name__ == '__main__':
    main()
