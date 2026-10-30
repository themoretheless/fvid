#!/usr/bin/env python3
"""Author fixed reserved-flag tfdt witnesses from owned, synthetic VP9 fMP4 controls."""
from pathlib import Path
import hashlib
import json
from generate_vp9_chroma import atom, word
from generate_mp4_tfhd_defaults_regressions import atoms

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'tests/fixtures/playback-errors/mp4-tfdt-flags'
OWNED = ROOT / 'tests/fixtures/playback-errors/mp4-tfhd-defaults'


def with_flags(data, flags):
    result = b''
    for kind, body in atoms(data):
        if kind in [b'moof', b'traf']:
            body = with_flags(body, flags)
        elif kind == b'tfdt':
            assert int.from_bytes(body[:4], 'big') & 0xFFFFFF == 0
            body = word(int.from_bytes(body[:4], 'big') | flags) + body[4:]
        result += atom(kind, body)
    return result


def generate():
    files = {'frame.vp9': (OWNED / 'frame.vp9').read_bytes()}
    invalid = []
    for version, source in [(0, 'tfhd-overrides-trex-nonsync.mp4'), (1, 'tfdt-v1-exact.mp4')]:
        control = (OWNED / source).read_bytes()
        files[f'valid-v{version}.mp4'] = control
        for flags in [1, 0x800000]:
            name = f'tfdt-v{version}-flags-{flags:06x}.mp4'
            files[name] = with_flags(control, flags)
            assert len(files[name]) == len(control)
            invalid.append(name)
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in files.items():
        (OUT / name).write_bytes(data)
    (OUT / 'manifest.json').write_text(json.dumps({
        'provenance': 'Owned pure-Python 16x16 VP9, three independent frames, 50Hz/60ms. '
                      'Two complete fMP4 controls and four same-size tfdt reserved-flag companions. '
                      'No private media/parameters, FFmpeg or network. Only tfdt FullBox flags differ.',
        'invalid_flags': invalid,
        'sha256': {name: hashlib.sha256(data).hexdigest() for name, data in files.items()},
    }, indent=2) + '\n')


if __name__ == '__main__':
    generate()
