#!/usr/bin/env python3
"""Short authored composition-order playback refusals and positive controls."""
import hashlib
import json
import struct
from pathlib import Path
from generate_vp9_chroma import frame, mp4, atom, word
from generate_mp4_fragment_description_regressions import atoms
OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/mp4-audio-composition-order'
PREDICTORS = [384, 1408, 2432]


def generate():
    video = dict(atoms(mp4([frame(8, '420', 16, 16)] * 3, 8, '420', (16, 16), [1, 2, 3])))
    header = atom(b'ftyp', video[b'ftyp'])
    offset = len(header) + 8 + len(video[b'mdat'])
    packets = [struct.pack('>H', p) + bytes([0x11]) * 32 for p in PREDICTORS]
    pcm = [b''.join(struct.pack('<f', (p + i) / 32768) for i in range(1, 65)) for p in PREDICTORS]
    entry = bytearray(28)
    entry[6:8] = struct.pack('>H', 1)
    entry[16:20] = struct.pack('>HH', 1, 16)
    entry[24:28] = word(8000 << 16)

    def movie(pts, explicit=True):
        tables = (atom(b'stsd', word(0, 1) + atom(b'ima4', entry))
                  + atom(b'stts', word(0, 1, 3, 64)) + atom(b'stsz', word(0, 34, 3))
                  + atom(b'stsc', word(0, 1, 1, 3, 1)) + atom(b'stco', word(0, 1, offset)))
        if explicit:
            tables += atom(b'ctts', word(1 << 24, 3) + b''.join(word(1) + struct.pack('>i', p - i * 64) for i, p in enumerate(pts)))
        tkhd = bytearray(84)
        tkhd[:4] = word(3)
        tkhd[12:16] = word(2)
        tkhd[20:24] = word(2)
        tkhd[40:44] = tkhd[56:60] = word(65536)
        tkhd[72:76] = word(0x40000000)
        mdhd = bytearray(24)
        mdhd[12:20] = word(8000, 192)
        track = atom(b'trak', atom(b'tkhd', tkhd) + atom(b'mdia', atom(b'mdhd', mdhd)
                     + atom(b'hdlr', word(0, 0) + b'soun' + bytes(12))
                     + atom(b'minf', atom(b'dinf', atom(b'dref', word(0, 1) + atom(b'url ', word(1))))
                            + atom(b'stbl', tables))))
        return header + atom(b'mdat', video[b'mdat'] + b''.join(packets)) + atom(b'moov', video[b'moov'] + track)

    rows = [('ordered', [0, 64, 128], True), ('negative', [-64, 0, 64], True),
            ('gap', [0, 128, 192], True), ('reordered', [0, 128, 64], False),
            ('duplicate', [0, 0, 128], False), ('equal', [0, 0, 0], False),
            ('overlap', [0, 32, 128], False)]
    files = {name + '.mp4': movie(pts, name != 'ordered') for name, pts, _ in rows}
    for i, payload in enumerate(packets):
        files[f'packet-{i}.ima4'] = payload
    files['ordered.f32le'] = b''.join(pcm)
    files['negative.f32le'] = pcm[1] + pcm[2]
    files['gap.f32le'] = pcm[0] + bytes(64 * 4) + pcm[1] + pcm[2]
    manifest = {'provenance': 'Fresh analytic IMA4 ramps with predictors384,1408,2432 and three authored neutral16x16 VP9 frames. No private media,codec parameter sets,encoder or network.',
                'policy': 'Ordered,negative-origin and positive-gap movies are playback acceptance controls. Reordered/overlapping composition is unsupported by the linear playback scheduler: require an explicit refusal before PCM,not a false successful decode. Raw packet extraction remains supported.',
                'cases': [{'file': n + '.mp4', 'pts': pts, 'playback_acceptance': ok} for n, pts, ok in rows],
                'rate': 8000, 'packet_frames': 64, 'predictors': PREDICTORS,
                'sha256': {n: hashlib.sha256(b).hexdigest() for n, b in files.items()}}
    files['manifest.json'] = (json.dumps(manifest, indent=2) + '\n').encode()
    return files


if __name__ == '__main__':
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)
