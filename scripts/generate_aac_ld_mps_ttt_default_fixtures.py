#!/usr/bin/env python3
"""Authored default/hold predictive ICC movies and explicit-unity controls."""
import json
from pathlib import Path
from generate_aac_ld_mps_inband_fixtures import raw
from generate_aac_ld_mps_config_fixtures import TREES, BANDS
from generate_aac_ld_mps_transport_fixtures import field, packed, envelope, length
from generate_aac_ld_mps_entropy_fixtures import pcm
from generate_he_aac_packet_fixtures import video_fixture
from generate_aac_eld_fixtures import mkv

ROOT = Path(__file__).resolve().parents[1]
FIX = ROOT/'tests/fixtures/playback-errors'
OUT = FIX/'aac-ld-mps-ttt-default'


def main():
    parent = FIX/'aac-ld-mps-inband'
    source = json.loads((parent/'manifest.json').read_text())
    data = (parent/'packets.bin').read_bytes()
    blob, cases = bytearray(), []
    OUT.mkdir(exist_ok=True)
    for tree in [2, 3, 4]:
        base = next(c for c in source['cases'] if c['name'] == f'plain-tree{tree}-repeat')
        old = bytes.fromhex(base['configs'][0])
        asc_bits = raw(bytes.fromhex(base['asc']))
        marker = field(2, 4)+length(len(old))+raw(old)+'000000'
        start = asc_bits.rfind(marker)
        assert start >= 0
        for bands in [4, 9, 23]:
            for mode, split in [(0, None), (1, None)] + ([(0, 4), (1, 4)] if bands == 9 else []):
                ttt = ('0'+field(mode, 3) if split is None else
                    '1'+field(mode, 3)+field(1-mode, 3)+field(split, 5))
                ssc = packed(field(3, 4)+field(7, 5)+field(BANDS.index(bands), 3)
                    +field(tree, 4)+'00'+'0'+'0'+'000000'+'000'+'0'+'0000'+'0'
                    +field(bands, 5)+ttt)
                asc = packed(asc_bits[:start]+field(2, 4)+length(len(ssc))+raw(ssc)+'000000')
                regions = [('CLD', bands, (box % 3-1)*3, False) for box in range(len(TREES[tree][2]))]
                regions += [('ICC', bands, 2, False) for lfe in TREES[tree][2] if not lfe]
                for count in ([bands] if split is None else [split, bands-split]):
                    regions += [('CPC', count, 10, False), ('CPC', count, 6, False), ('ICC', count, 0, True)]
                for pattern in ['initial', 'reset-hold']:
                    suffix = '' if split is None else f'-high{1-mode}-split{split}'
                    record = dict(name=f'tree{tree}-b{bands}-m{mode}{suffix}-{pattern}', tree=tree,
                        bands=bands, mode=mode, split=split, pattern=pattern, asc=asc.hex(), ssc=ssc.hex(),
                        rate=48000, samples=1536, channels=TREES[tree][1])
                    for control in [False, True]:
                        frames, spatial = [], []
                        for index, row in enumerate(base['frames']):
                            prefix = raw(data[row['offset']:row['offset']+row['bytes']])[:row['prefix_bits']]
                            unity = index != 1 if pattern == 'initial' else index != 0
                            bits = '01'+field(index == 0, 1)
                            for kind, count, value, predictive in regions:
                                coding = 0 if predictive and unity and not control else 3
                                if predictive and pattern == 'reset-hold' and index == 2 and not control:
                                    coding = 1
                                bits += field(coding, 2)*2
                                if coding == 3:
                                    levels, offset = {'CLD': (31, 15), 'ICC': (8, 0), 'CPC': (51, 20)}[kind]
                                    if predictive:
                                        value = 0 if unity else 2
                                    bits += '10001'+pcm([value+offset]*(2*count), levels)
                            body = packed(bits+'0000')  # Two disabled smoothing controls.
                            spatial.append(body.hex())
                            payload = bytes([len(ssc)])+ssc+body if index == 0 else body
                            packet = packed(prefix+envelope(9, 1 if index == 0 else 0, 3, payload, True))
                            frames.append(dict(offset=len(blob), bytes=len(packet)))
                            blob += packet
                        suffix = '-control' if control else ''
                        case = dict(base, asc=asc.hex(), frames=frames, samples=1536)
                        filename = record['name']+suffix+'.mp4'
                        (OUT/filename).write_bytes(video_fixture([case], blob, record['channels'], write=False)['data'])
                        matroska = record['name']+suffix+'.mkv'
                        packets = [bytes(blob[r['offset']:r['offset']+r['bytes']]) for r in frames]
                        (OUT/matroska).write_bytes(mkv(asc, packets, 48000, 512, record['channels']))
                        key = 'control' if control else 'default'
                        record[key] = dict(video=filename, file=matroska, frames=frames, spatial=spatial)
                    cases.append(record)
    (OUT/'packets.bin').write_bytes(blob)
    (OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,
        scope='48 authored predictive TTT default/reset/hold programmes and '
        '48 explicit-unity coding controls. Own ELD core, three trees, modes0/1, '
        'grids4/9/23; twelve dual-region cases use opposite predictive modes '
        'below/above band4 of grid9. Index0 dequantizes to coherence1; no forbidden physical '
        'coherence0 is authored. No private or ISO media.'), indent=2)+'\n')


if __name__ == '__main__':
    main()
