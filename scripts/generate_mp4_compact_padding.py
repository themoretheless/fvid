#!/usr/bin/env python3
"""Authored stz2 padding witnesses, independent of ordinary test execution."""
from pathlib import Path
import hashlib
import json
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom

OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/mp4-compact-padding'


def main():
    source = OUT.parent / 'mp4-compact-sizes'
    parent = json.loads((source / 'manifest.json').read_text())
    files = {f'frame-{i}.yuv16': (source / f'frame-{i}.yuv16').read_bytes() for i in range(3)}
    controls = []
    names = ['text-stz2-4-3.mp4', 'text-stz2-4-4.mp4', 'text-stz2-8-3.mp4', 'text-stz2-16-3.mp4']
    for name in names:
        c = next(c for c in parent['cases'] if c['file'] == name)
        controls.append(c)
        files[name] = (source / name).read_bytes()
        for i in range(len(c['texts'])):
            n = f'text-{c["width"]}-{len(c["texts"])}-{i}.packet'
            files[n] = (source / n).read_bytes()
    changes = []
    for low in [1, 15]:
        def padding(data):
            out = b''
            for k, b in atoms(data):
                if k in [b'moov', b'trak', b'mdia', b'minf', b'stbl']:
                    b = padding(b)
                elif k == b'stz2':
                    assert b[7] == 4 and b[8:12] == bytes([0, 0, 0, 3])
                    assert b[-1] & 15 == 0
                    b = b[:-1] + bytes([b[-1] | low])
                out += atom(k, b)
            return out
        name = f'nonzero-padding-{low}.mp4'
        original = files['text-stz2-4-3.mp4']
        damaged = padding(original)
        diff = [i for i, (a, b) in enumerate(zip(original, damaged)) if a != b]
        assert len(original) == len(damaged) and len(diff) == 1
        assert original[diff[0]] >> 4 == damaged[diff[0]] >> 4
        files[name] = damaged
        changes.append({'file': name, 'changed_byte': diff[0], 'before': original[diff[0]], 'after': damaged[diff[0]]})
    OUT.mkdir(parents=True, exist_ok=True)
    for n, b in files.items():
        (OUT / n).write_bytes(b)
    (OUT / 'manifest.json').write_text(json.dumps({
        'provenance': 'Only previously authored VP9/tx3g witnesses. Damaged files differ solely in unused low padding bits; no private media, codec parameters, FFmpeg or network.',
        'reference': 'https://b.goeswhere.com/ISO_IEC_14496-12_2015.pdf#page=62',
        'accounting': 'Uncounted FVID-0432 padding-integrity follow-up.',
        'controls': controls, 'damaged': changes,
        'sha256': {n: hashlib.sha256(b).hexdigest() for n, b in files.items()},
    }, indent=2) + '\n')


if __name__ == '__main__':
    main()
