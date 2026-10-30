#!/usr/bin/env python3
"""Authored QuickTime preview atoms preceding three synthetic VP9 frames."""
import hashlib
import json
import struct
from pathlib import Path
from generate_mp4_seek_io_regressions import generate
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom, word

OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/mp4-preview-prefix'

def generate_files():
    seed = generate()
    parts = list(atoms(seed['three-colors.mp4']))
    assert [k for k, _ in parts] == [b'ftyp', b'mdat', b'moov']
    ftyp, mdat, moov = [atom(k, b) for k, b in parts]
    rect = struct.pack('>hhhh', 0, 0, 16, 16)
    drawing = rect + b'\x11\x01\x01\x00\x0a' + rect + b'\x31' + rect + bytes(3) + b'\xff'
    pict = atom(b'PICT', struct.pack('>H', len(drawing) + 2) + drawing)
    assert len(pict) == 44
    pnot = atom(b'pnot', struct.pack('>IH4sH', 0, 0, b'PICT', 1))
    files = {n: b for n, b in seed.items() if n != 'three-colors.mp4'}
    cases = []
    def shifted(data, delta):
        out = b''
        for k, b in atoms(data):
            if k in [b'moov', b'trak', b'mdia', b'minf', b'stbl']:
                b = shifted(b, delta)
            elif k == b'stco':
                count = int.from_bytes(b[4:8], 'big')
                offsets = struct.unpack('>' + 'I' * count, b[8:])
                b = b[:8] + word(*[n + delta for n in offsets])
            out += atom(k, b)
        return out
    for name, prefix, suffix, old in [
        ('ftyp-control.mov', ftyp + pnot + pict, b'', True),
        ('wide-control.mov', atom(b'wide', b'') + pnot + pict, b'', True),
        ('mdat-control.mov', b'', pnot + pict, True),
        ('pnot-first.mov', pnot + pict, b'', False),
        ('pict-first.mov', pict + pnot, b'', False),
    ]:
        files[name] = prefix + mdat + shifted(moov, len(prefix) - len(ftyp)) + suffix
        cases.append({'file': name, 'previously_recognized': old})
    for k in [b'pnot', b'PICT']:
        files[f'bad-{k.decode()}-extent.mov'] = word(4) + k
        files[f'empty-{k.decode()}.mov'] = (pnot + pict) if k == b'pnot' else (pict + pnot)
    return files, cases

def main():
    files, cases = generate_files()
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in files.items():
        (OUT / name).write_bytes(data)
    (OUT / 'manifest.json').write_text(json.dumps({
        'provenance': 'Author-created 16x16 QuickDraw v1 clipped black rectangle and three analytic VP9 color frames. No copied media or codec parameters. No encoder/network.',
        'references': ['https://developer.apple.com/documentation/quicktime-file-format/preview_atom', 'https://leopard-adc.pepas.com/technotes/qd/qd_14.html'],
        'cases': cases,
        'sha256': {n: hashlib.sha256(b).hexdigest() for n, b in files.items()},
    }, indent=2) + '\n')

if __name__ == '__main__':
    main()
