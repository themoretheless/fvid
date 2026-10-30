#!/usr/bin/env python3
"""Authored chunk storage permutations; all sample/PTS orders stay unchanged."""
from pathlib import Path
import hashlib
import itertools
import json
import struct
from generate_mp4_seek_io_regressions import generate
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom, word

OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/mp4-chunk-storage-order'


def main():
    seed = generate()
    files = {n: b for n, b in seed.items() if n != 'three-colors.mp4'}
    packets = [seed[f'frame-{i}.packet'] for i in range(3)]
    parts = list(atoms(seed['three-colors.mp4']))
    prefix = atom(parts[0][0], parts[0][1])
    moov = parts[2][1]
    cases = []
    def movie(order, kind, layout, damage=None):
        data = b''
        offsets = [0] * 3
        if layout == 'one-mdat':
            for i in order:
                offsets[i] = len(prefix) + 8 + len(data)
                data += packets[i]
            data = atom(b'mdat', data)
        else:
            for i in order:
                offsets[i] = len(prefix) + len(data) + 8
                data += atom(b'mdat', packets[i])
        if damage == 'partial-overlap':
            offsets[1] = offsets[0] + 1
        elif damage == 'duplicate-span':
            offsets[1] = offsets[0]
        elif damage == 'backward-overlap':
            offsets = [offsets[1], offsets[1] + 1, offsets[0]]
        elif damage == 'outside-mdat':
            offsets[0] = len(prefix) - 1
        def rewrite(body):
            out = b''
            for k, b in atoms(body):
                if k in [b'trak', b'mdia', b'minf', b'stbl']:
                    b = rewrite(b)
                elif k == b'stsc':
                    b = word(0, 1, 1, 1, 1)
                elif k == b'stco':
                    k = kind.encode()
                    b = word(0, 3) + b''.join(struct.pack('>I' if kind == 'stco' else '>Q', x) for x in offsets)
                out += atom(k, b)
            return out
        return prefix + data + atom(b'moov', rewrite(moov)), offsets
    for kind in ['stco', 'co64']:
        for layout in ['one-mdat', 'three-mdats']:
            for order in itertools.permutations(range(3)):
                name = f'{kind}-{layout}-{"".join(map(str, order))}.mp4'
                files[name], offsets = movie(order, kind, layout)
                cases.append({'file': name, 'kind': kind, 'layout': layout,
                              'physical_order': order, 'offsets': offsets})
        for damage in ['partial-overlap', 'duplicate-span', 'backward-overlap', 'outside-mdat']:
            name = f'{kind}-{damage}.mp4'
            files[name], offsets = movie([0, 1, 2], kind, 'one-mdat', damage)
            cases.append({'file': name, 'damage': damage, 'offsets': offsets})
    OUT.mkdir(parents=True, exist_ok=True)
    for n, b in files.items():
        (OUT / n).write_bytes(b)
    (OUT / 'manifest.json').write_text(json.dumps({
        'provenance': 'Authored16x16 VP9 colors. Sample order remains0/1/2 and PTS0/1/2 at50Hz; physical chunk storage uses all six permutations in one or three mdats. Separate deliberate overlap/out-of-mdat refusal witnesses. No private media/parameters, FFmpeg or network.',
        'reference': 'https://developer.apple.com/documentation/quicktime-file-format/chunk_offset_atom',
        'accounting': 'Uncounted review of over-restrictive FVID-0189 chunk ordering guard; do not add an improvement ID.',
        'cases': cases, 'sha256': {n: hashlib.sha256(b).hexdigest() for n, b in files.items()},
    }, indent=2) + '\n')


if __name__ == '__main__':
    main()
