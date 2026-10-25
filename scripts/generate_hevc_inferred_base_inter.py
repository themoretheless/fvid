#!/usr/bin/env python3
"""Nine-picture, two-IDR owned I/P/B witnesses; no codec process or network."""
import json
from pathlib import Path
from generate_hevc_matroska_formats import unpack
from generate_ffv1_gray_fixtures import element, uint

ROOT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
OUT = ROOT / 'hevc-inferred-base-inter'

def generate():
    files, cases = {}, []
    existing = json.loads((ROOT / 'hevc-matroska-reorder.json').read_text())['cases']
    for depth, name in [(8, 'main-ipb'), (10, 'main10-ipb')]:
        private, packets, width, height, _ = unpack(ROOT.parent / 'hevc' / (name + '.mp4'))
        control = next(c for c in existing if c['file'] == f'hevc-matroska-ipb-{depth}-0-0-0.mkv')
        # The first GOP has reordered P/B pictures. The ninth coded packet
        # starts a second IDR sequence while two old output pictures remain.
        packets = packets[:9]
        rank = control['decode_to_display'][:9]
        assert sorted(rank) == list(range(9))
        track = (uint(0xd7, 1) + uint(0x83, 1)
                 + element(0x86, b'V_MPEGH/ISO/HEVC')
                 + element(0x63a2, private) + uint(0x23e383, 40000000)
                 + element(0xe0, uint(0xb0, width) + uint(0xba, height)))
        clusters = b''.join(element(0x1f43b675, uint(0xe7, rank[i] * 40)
                   + element(0xa3, b'\x81\0\0' + (b'\x80' if i in [0, 8] else b'\0') + packet))
                   for i, packet in enumerate(packets))
        data = (element(0x1a45dfa3, element(0x4282, b'matroska'))
                + element(0x18538067, element(0x1549a966, uint(0x2ad7b1, 1000000))
                          + element(0x1654ae6b, element(0xae, track)) + clusters))
        file = f'repeated-idr-{depth}.mkv'
        files[file] = data
        cases.append(dict(file='hevc-inferred-base-inter/' + file,
                          frames=control['frames'][:9], depth=depth))
    files['manifest.json'] = (json.dumps(dict(cases=cases, provenance=
        'Nine owned synthetic I/P/B packets from repository main/main10 fixtures, '
        'including the second IDR with pending old output. Existing independent '
        'saved pixel oracles; own Matroska muxing, no private media, FFmpeg or network.'), indent=2) + '\n').encode()
    return files

if __name__ == '__main__':
    OUT.mkdir(exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)
