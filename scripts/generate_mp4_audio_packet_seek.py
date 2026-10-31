#!/usr/bin/env python3
"""Author IMA4 seek controls and a separate manual CPU workload."""
import hashlib
import json
import struct
import sys
from pathlib import Path
from generate_vp9_chroma import frame, mp4, atom, word
from generate_mp4_fragment_description_regressions import atoms

PREDICTORS = [128, 1152, 2176]
ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'tests/fixtures/playback-errors/mp4-audio-packet-seek'


def movie(count, pts=None):
    assert 6 <= count <= 1000000
    video = dict(atoms(mp4([frame(8, '420', 16, 16)] * 6, 8, '420', (16, 16), list(range(1, 7)))))
    header = atom(b'ftyp', video[b'ftyp'])
    offset = len(header) + 8 + len(video[b'mdat'])
    blocks = [struct.pack('>H', p) + bytes([0x11]) * 32 for p in PREDICTORS]
    payload = b''.join(blocks[i % 3] for i in range(count))
    entry = bytearray(28)
    entry[6:8] = struct.pack('>H', 1)
    entry[16:20] = struct.pack('>HH', 1, 16)
    entry[24:28] = word(8000 << 16)
    tables = (atom(b'stsd', word(0, 1) + atom(b'ima4', entry))
              + atom(b'stts', word(0, 1, count, 64))
              + atom(b'stsz', word(0, 34, count))
              + atom(b'stsc', word(0, 1, 1, count, 1))
              + atom(b'stco', word(0, 1, offset)))
    if pts is not None:
        assert len(pts) == count
        tables += atom(b'ctts', word(1 << 24, count) + b''.join(word(1) + struct.pack('>i', p - i * 64) for i, p in enumerate(pts)))
    duration = count * 64
    tkhd = bytearray(84)
    tkhd[:4] = word(3)
    tkhd[12:16] = word(2)
    tkhd[20:24] = word((duration * 50 + 7999) // 8000)
    tkhd[40:44] = tkhd[56:60] = word(65536)
    tkhd[72:76] = word(0x40000000)
    mdhd = bytearray(24)
    mdhd[12:20] = word(8000, duration)
    track = atom(b'trak', atom(b'tkhd', tkhd) + atom(b'mdia', atom(b'mdhd', mdhd)
                 + atom(b'hdlr', word(0, 0) + b'soun' + bytes(12))
                 + atom(b'minf', atom(b'dinf', atom(b'dref', word(0, 1) + atom(b'url ', word(1))))
                        + atom(b'stbl', tables))))
    movie_body = b''
    for kind, body in atoms(video[b'moov']):
        if kind == b'mvhd':
            body = bytearray(body)
            body[16:20] = word(max(6, (duration * 50 + 7999) // 8000))
        movie_body += atom(kind, body)
    return header + atom(b'mdat', video[b'mdat'] + payload) + atom(b'moov', movie_body + track)


def fixtures():
    queries = [-1, 0, 63, 64, 65, 127, 128, 2**63 - 1]
    cases = [
        ('ordered', [0, 64, 128, 192, 256, 320], queries, [0, 0, 0, 1, 1, 1, 2, 5]),
        ('reordered', [0, 128, 64, 192, 256, 320], queries, [0, 0, 0, 2, 2, 2, 1, 5]),
        ('duplicate', [0, 64, 64, 192, 256, 320], queries, [0, 0, 0, 2, 2, 2, 2, 5]),
        ('negative', [-64, 0, 64, 128, 192, 256], [-65, -64, -1, 0, 1, 63, 64, 2**63 - 1], [0, 0, 0, 1, 1, 1, 2, 5]),
        ('equal', [0] * 6, queries, [0, 5, 5, 5, 5, 5, 5, 5]),
    ]
    files = {name + '.mp4': movie(6, None if name == 'ordered' else pts) for name, pts, _, _ in cases}
    manifest = {'provenance': 'New analytic IMA4 ramps and neutral16x16 VP9 authored in code; no private media/codec parameters, encoder or network.',
                'scope': 'Ordered audio playback acceptance; all variants preserve direct raw-packet navigation and independently decoded block PCM. Reordered/equal variants do not assert new full-playback support.',
                'predictors': PREDICTORS, 'sample_rate': 8000, 'samples_per_packet': 64,
                'cases': [{'file': name + '.mp4', 'pts': pts, 'queries': queries, 'expected_indices': indices} for name, pts, queries, indices in cases],
                'sha256': {name: hashlib.sha256(data).hexdigest() for name, data in files.items()}}
    files['manifest.json'] = (json.dumps(manifest, indent=2) + '\n').encode()
    return files


if __name__ == '__main__':
    if len(sys.argv) > 1:
        path = Path(sys.argv[1])
        count = int(sys.argv[2]) if len(sys.argv) > 2 else 160000
        data = movie(count)
        path.write_bytes(data)
        path.with_suffix('.json').write_text(json.dumps({'provenance': 'Authored analytic IMA4 ramps and neutral VP9; separately generated manual benchmark input, no encoder/network/private media.',
            'packets': count, 'sample_rate': 8000, 'packet_ticks': 64, 'sha256': hashlib.sha256(data).hexdigest(), 'bytes': len(data)}, indent=2) + '\n')
    else:
        OUT.mkdir(parents=True, exist_ok=True)
        for name, data in fixtures().items():
            (OUT / name).write_bytes(data)
