#!/usr/bin/env python3
"""Authored self-contained data-reference tables and bounded entry indices."""
from pathlib import Path
import hashlib
import json
import struct
from generate_mp4_seek_io_regressions import generate
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom, word

OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/mp4-local-data-references'


def main():
    seed = generate()
    parent = OUT.parent / 'mp4-compact-sizes'
    sources = {'video': seed['three-colors.mp4'],
               'pcm': (parent / 'pcm-stsz-4.mp4').read_bytes(),
               'mace': (parent / 'mace-stsz-4.mp4').read_bytes(),
               'text': (parent / 'text-stsz-4-3.mp4').read_bytes(),
               'fragment': (parent / 'fragment-init-stsz.mp4').read_bytes()}
    files = {n: b for n, b in seed.items() if n != 'three-colors.mp4'}
    for n in ['pcm.s16le', 'pcm.f32le', *[f'text-4-3-{i}.packet' for i in range(3)],
              *[f'fragment-text-{i}.packet' for i in range(3)]]:
        files[n] = (parent / n).read_bytes()
    files['mace.packet'] = bytes(2)
    local = lambda kind: atom(kind.encode(), word(1))
    cases = []
    damaged = []

    def movie(kind, refs, selected, alternating=False, declared=None, opaque_ref=None):
        def rewrite(data):
            out = b''
            for k, b in atoms(data):
                if k in [b'moov', b'trak', b'mdia', b'minf', b'dinf', b'stbl']:
                    b = rewrite(b)
                elif k == b'dref':
                    b = word(0, len(refs) if declared is None else declared) + b''.join(refs)
                elif k == b'stsd':
                    entries = list(atoms(b[8:]))
                    assert len(entries) == 1
                    ek, eb = entries[0]
                    indices = [1, 2, 3] if alternating else [selected]
                    b = word(0, len(indices) + int(opaque_ref is not None))
                    for index in indices:
                        b += atom(ek, eb[:6] + struct.pack('>H', index) + eb[8:])
                    if opaque_ref is not None:
                        b += atom(b'zzzz', bytes(6) + struct.pack('>H', opaque_ref))
                elif k == b'stsc' and alternating:
                    b = word(0, 3, 1, 1, 1, 2, 1, 2, 3, 1, 3)
                elif k == b'stco' and alternating:
                    start = int.from_bytes(b[8:12], 'big')
                    step = len(seed['frame-0.packet'])
                    b = word(0, 3, start, start + step, start + step * 2)
                out += atom(k, b)
            return out
        return rewrite(sources[kind])

    def valid(name, kind, types, selected, alternating=False):
        files[name] = movie(kind, list(map(local, types)), selected, alternating)
        cases.append({'file': name, 'kind': kind, 'types': types, 'selected': selected,
                      'alternating': alternating, 'single': len(types) == 1})

    for kind in ['url ', 'alis']:
        valid(f'single-{kind.strip()}.mp4', 'video', [kind], 1)
    for types in [['url ', 'url '], ['alis', 'alis'], ['url ', 'alis'], ['alis', 'url ']]:
        for selected in [1, 2]:
            valid(f'video-{"-".join(t.strip() for t in types)}-{selected}.mp4', 'video', types, selected)
    valid('video-third-local.mp4', 'video', ['url ', 'alis', 'url '], 3)
    valid('video-largest-reference.mp4', 'video', ['url '] * 65535, 65535)
    valid('video-alternating-descriptions.mp4', 'video', ['url ', 'alis', 'url '], 1, True)
    for kind in ['pcm', 'mace', 'text', 'fragment']:
        valid(f'{kind}-third-local.mp4', kind, ['url ', 'alis', 'url '], 3)
        if kind in ['pcm', 'mace']:
            valid(f'{kind}-single-control.mp4', kind, ['url '], 1)

    negatives = [
        ('index-zero', [local('url ')], 0, None, None, 'invalid sample data reference'),
        ('index-two-single', [local('url ')], 2, None, None, 'invalid sample data reference'),
        ('index-four-three', [local('url ')] * 3, 4, None, None, 'invalid sample data reference'),
        ('index-max-three', [local('url ')] * 3, 65535, None, None, 'invalid sample data reference'),
        ('opaque-unused-index-four', [local('url ')] * 3, 1, None, 4, 'invalid sample data reference'),
        ('empty-table', [], 1, None, None, 'only self-contained MP4 data references are supported'),
        ('count-mismatch', [local('url ')] * 2, 1, 3, None, 'only self-contained MP4 data references are supported'),
        ('external-single', [atom(b'url ', word(0) + b'authored-external.bin\0')], 1, None, None, 'only self-contained MP4 data references are supported'),
        ('external-unused-second', [local('url '), atom(b'url ', word(0) + b'authored-external.bin\0')], 1, None, None, 'only self-contained MP4 data references are supported'),
        ('version-one', [atom(b'url ', word(0x01000001))], 1, None, None, 'only self-contained MP4 data references are supported'),
        ('local-with-location', [atom(b'url ', word(1) + b'authored.bin\0')], 1, None, None, 'only self-contained MP4 data references are supported'),
        ('wrong-type', [atom(b'free', word(1))], 1, None, None, 'only self-contained MP4 data references are supported'),
    ]
    for label, refs, selected, declared, opaque, expected in negatives:
        name = f'bad-{label}.mp4'
        files[name] = movie('video', refs, selected, declared=declared, opaque_ref=opaque)
        damaged.append({'file': name, 'expected': expected,
                        'new_specific_index_check': len(refs) > 1 and expected == 'invalid sample data reference'})
    OUT.mkdir(parents=True, exist_ok=True)
    for n, b in files.items():
        (OUT / n).write_bytes(b)
    (OUT / 'manifest.json').write_text(json.dumps({
        'provenance': 'Authored16x16 three-color VP9, PCM formula, zero-coded MACE, ASCII captions and real text fragments from earlier authored witnesses. Only dref catalogs and common sample-entry reference indices change; alternating local descriptions also split the same original video chunk. No private media/parameters, FFmpeg or network.',
        'reference': 'https://developer.apple.com/documentation/quicktime-file-format/sample_description_atom',
        'cases': cases, 'damaged': damaged,
        'sha256': {n: hashlib.sha256(b).hexdigest() for n, b in files.items()},
    }, indent=2) + '\n')


if __name__ == '__main__':
    main()
