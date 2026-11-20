#!/usr/bin/env python3
"""Own explicit-Strict LD-SBR Q31 movies and exact inverse-filter controls.

Default Q31 acceptance is qualified separately in aac-ld-sbr-q31.
No ISO/private AAC bits or parameter sets are copied. Explicit offline generation.
"""
import json
from pathlib import Path
from generate_aac_eld_fixtures import generate, mkv
from generate_aac_eld_sbr_fixtures import sbr_bits
from generate_aac_sbr_frequency_oracles import tables
from generate_he_aac_packet_fixtures import packed, video_fixture

ROOT = Path(__file__).resolve().parents[1]
FIX = ROOT/'tests/fixtures/playback-errors'
OUT = FIX/'aac-eld-sbr-noise-range'


def main():
    OUT.mkdir(exist_ok=True)
    generated = generate(True, [(n, 24000, 2, 'base', 0, 0, False) for n in [480, 512]], frame_count=8)
    cores = json.loads(generated['manifest.json'])['cases']
    parent = FIX/'aac-eld-sbr'
    models = json.loads((parent/'manifest.json').read_text())['cases']
    saved = (parent/'packets.bin').read_bytes()
    _, high, low, _ = tables(10, 27, 0, False, 0, 0)
    blob, cases, errors = bytearray(), [], []
    for core in cores:
        n = core['n']
        model = next(c for c in models if c['name'] == f'n{n}-c2-invf')
        for poison in [False, True]:
            frames, packets = [], []
            for i, source in enumerate(core['frames']):
                bits = source['bits']+sbr_bits(2, len(high)-1, len(low)-1, n//32, i, True, 'invf', noise_level=31 if poison and i == 3 else None)
                wire = packed(bits)
                old = model['frames'][i]
                original = saved[old['offset']:old['offset']+old['bytes']]
                assert (wire != original) == (poison and i == 3)
                frames.append(dict(offset=len(blob), bytes=len(wire)))
                packets.append(wire)
                blob.extend(wire)
            name = f'n{n}-'+('q31' if poison else 'control')
            c = dict(model, name=name, frames=frames, samples=8*n*2, video=name+'.mp4', file=name+'.mkv',
                     golden=f'n{n}-control.f32le', live_prefix_packets=2, edges=[2*n, 4*n, 6*n])
            (OUT/c['video']).write_bytes(video_fixture([c], blob, 2, write=False)['data'])
            (OUT/c['file']).write_bytes(mkv(bytes.fromhex(c['asc']), packets, 48000, n*2, 2))
            if poison:
                c.update(error='SBR noise level exceeds normative range: 31', bad_frame=3,
                         expectation='explicit Strict policy refusal; default AllowQ31 accepts')
                errors.append(c)
            else:
                (OUT/c['golden']).write_bytes((parent/(model['name']+'.f32le')).read_bytes()[:8*n*2*2*4])
                cases.append(c)
    (OUT/'packets.bin').write_bytes(blob)
    (OUT/'manifest.json').write_text(json.dumps(dict(cases=cases, strict_errors=errors,
        exports=[dict(c, video=c['file'], name=c['name']+'-matroska') for c in cases],
        scope='Own stereo480/512 inverse filtering,8 AUs,dual-rate LD-SBR/CRC. Only fourth-frame noise absolute scalefactors change from7 to31; CRC regenerated. Q31 metadata denotes explicit Strict refusal; default acceptance is qualified by sibling aac-ld-sbr-q31.'), separators=(',', ':'))+'\n')
    print(len(cases), 'controls;', len(errors), 'normative Q31 refusal movies; both containers')


if __name__ == '__main__':
    main()
