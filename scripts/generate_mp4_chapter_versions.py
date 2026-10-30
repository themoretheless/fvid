#!/usr/bin/env python3
"""Authored chapter-list version layouts; generation is separate from tests."""
from pathlib import Path
import hashlib
import json
import struct
from generate_mp4_seek_io_regressions import generate
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom, word

OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/mp4-chapter-versions'


def main():
    seed = generate()
    files = {n: b for n, b in seed.items() if n != 'three-colors.mp4'}
    titles = ['Opening', 'Середина', 'End 🎬']
    cases = []
    for version in [0, 1]:
        for clock in ['movie', 'ten-mhz']:
            for state in ['known', 'unknown', 'zero']:
                starts = [0, 20, 40] if clock == 'movie' else [0, 200000, 400000]
                entries = b''.join(struct.pack('>Q', t) + bytes([len(s.encode())]) + s.encode()
                                   for t, s in zip(starts, titles))
                header = word(version << 24) + (word(0) if version else b'')
                name = f'v{version}-{clock}-{state}.mp4'
                chpl = header + bytes([3]) + entries
                def rewrite(data):
                    out = b''
                    for k, b in atoms(data):
                        if k == b'moov':
                            b = rewrite(b) + atom(b'udta', atom(b'chpl', chpl))
                        elif k == b'trak':
                            b = rewrite(b)
                        elif k == b'mvhd':
                            duration = {'known': 60, 'unknown': 0xffffffff, 'zero': 0}[state]
                            b = b[:12] + word(1000, duration) + b[20:]
                        elif k == b'tkhd':
                            b = b[:20] + word(60) + b[24:]
                        out += atom(k, b)
                    return out
                files[name] = rewrite(seed['three-colors.mp4'])
                cases.append({'file': name, 'version': version, 'clock': clock,
                              'state': state, 'titles': titles, 'starts_ns': [0, 20000000, 40000000]})
        for kind in ['empty', 'truncated-header', 'truncated-first', 'truncated-tail']:
            header = word(version << 24) + (word(0) if version else b'')
            first = struct.pack('>Q', 0) + bytes([7]) + b'Opening'
            body = {'empty': header + b'\0', 'truncated-header': header[:-1],
                    'truncated-first': header + bytes([1]) + first[:-1],
                    'truncated-tail': header + bytes([2]) + first + bytes(3)}[kind]
            name = f'v{version}-{kind}.mp4'
            parts = list(atoms(seed['three-colors.mp4']))
            files[name] = b''.join(atom(k, b + atom(b'udta', atom(b'chpl', body)) if k == b'moov' else b)
                                   for k, b in parts)
            cases.append({'file': name, 'version': version, 'kind': kind,
                          'titles': ['Opening'] if kind == 'truncated-tail' else [],
                          'starts_ns': [0] if kind == 'truncated-tail' else []})
    OUT.mkdir(parents=True, exist_ok=True)
    for name, body in files.items():
        (OUT / name).write_bytes(body)
    (OUT / 'manifest.json').write_text(json.dumps({
        'provenance': 'Authored16x16 VP9 colors and invented UTF8 chapter titles. Version0 count follows version/flags; version1 adds four reserved bytes. No private media, FFmpeg or network.',
        'reference': 'https://github.com/FFmpeg/FFmpeg/blob/n8.0/libavformat/mov.c#L566-L600',
        'cases': cases, 'sha256': {n: hashlib.sha256(b).hexdigest() for n, b in files.items()},
    }, indent=2, ensure_ascii=False) + '\n')


if __name__ == '__main__':
    main()
