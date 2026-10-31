#!/usr/bin/env python3
"""Own AAC-LD nearest-frequency-table reproducers; explicit/offline generation."""
import json
from math import gcd
from pathlib import Path
from generate_aac_ld_layout_fixtures import LAYOUTS, body as core_body, geometry
from generate_aac_ld_mps_stp_fixtures import config, body
from generate_aac_ld_mps_transport_fixtures import field, packed, envelope
from generate_he_aac_packet_fixtures import frequency, video_fixture
from generate_aac_eld_fixtures import mkv

ROOT = Path(__file__).resolve().parents[1]
FIX = ROOT/'tests/fixtures/playback-errors'
OUT = FIX/'aac-ld-nearest-rate'
RATES = [7350, 8000, 11025, 12000, 16000, 18782, 55426, 64000, 88200, 96000]


def raw(data):
    return ''.join(field(v, 8) for v in data)


def main():
    OUT.mkdir(exist_ok=True)
    parents = json.loads((FIX/'aac-ld-layout.json').read_text())['cases']
    original = (FIX/'aac-ld-layout-packets.bin').read_bytes()
    models = json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases']
    configs = (FIX/'aac-ld-mps-assemble/configs.bin').read_bytes()
    blob, cases = bytearray(), []

    def rows(wires):
        result = []
        for wire in wires:
            packet = packed(wire)
            result.append(dict(offset=len(blob), bytes=len(packet), syntax_bits=len(wire)))
            blob.extend(packet)
        return result

    def emit(case, wires):
        case.update(frames=rows(wires), video=case['name']+'.mp4', file=case['name']+'.mkv',
                    container_rate=case['rate'], container_frame_samples=case['n'],
                    samples=8*case['n'], pcm_offset=0, slots=8, bands=64,
                    edges=sorted({case['n'], case['n']*case.get('group', 1), 7*case['n']}))
        (OUT/case['video']).write_bytes(video_fixture([case], blob, case['channels'], write=False)['data'])
        (OUT/case['file']).write_bytes(mkv(bytes.fromhex(case['asc']), [packed(w) for w in wires], case['rate'], case['n'], case['channels']))
        cases.append(case)

    for n in [480, 512]:
        for rate in RATES:
            nominal = 24000 if rate < 18783 else 48000
            cores = {}
            for cfg, (elements, mapping, mask) in LAYOUTS.items():
                parent = next(c for c in parents if c['n'] == n and c['rate'] == nominal and c['configuration'] == cfg)
                count, high = geometry(n, nominal)
                wires = []
                for i in range(8):
                    wire, channel = '', 0
                    for element in elements:
                        wire += '0000'+('0' if element == 1 else '')
                        for _ in range(2 if element == 1 else 1):
                            encoded, _ = core_body(n, count, high, i, channel, lfe=element == 3)
                            wire += encoded
                            channel += 1
                    f = parent['frames'][i]
                    assert packed(wire) == original[f['offset']:f['offset']+f['bytes']]
                    wires.append(wire)
                head = field(23, 5)+frequency(rate)+field(cfg, 4)+field(n == 480, 1)+'0000'
                case = dict(name=f'n{n}-r{rate}-c{cfg}', n=n, rate=rate, nominal=nominal,
                            spatial=False, channels=len(mapping), configuration=cfg, mask=mask,
                            asc=packed(head).hex(), core_reference_offset=parent['reference_offset'],
                            core_reference_bytes=parent['reference_bytes'])
                emit(case, wires)
                cores[cfg] = (case, head, wires)
            bins = 32 if rate < 27713 else 128
            group = bins//gcd(n, bins)
            slots = n*group//bins
            assert 8 % group == 0
            for tree in range(8):
                cfg = 1 if tree in [0, 1, 7] else 2 if tree in [2, 3, 4] else 6
                core, head, wires = cores[cfg]
                model = next(c for c in models if c['tree'] == tree and c['variant'] == 1 and c['bands'] == 23)
                old = config(model, 48000, slots, configs[model['config_offset']:model['config_offset']+model['config_bytes']], 0)
                ssc = packed(frequency(rate)+raw(old)[4:])
                asc = packed(head+field(0x7cc, 11)+'1'+field(len(ssc), 8)+raw(ssc))
                packets, reference = [], []
                for i, wire in enumerate(wires):
                    if (i+1) % group == 0:
                        spatial, _ = body(model, i//group, slots, 'off', False)
                        reference.append(spatial.hex())
                        wire += envelope(9, 0, 3, spatial, True)
                    packets.append(wire)
                case = dict(core, name=f'n{n}-r{rate}-tree{tree}', spatial=True,
                            channels=[6, 6, 6, 8, 8, 8, 8, 2][tree], input_channels=core['channels'],
                            tree=tree, mask=([0x3f]*3+[0xff,0x63f,0xff,0x63f,3])[tree],
                            asc=asc.hex(), core_asc=core['asc'], ssc=ssc.hex(),
                            group=group, qmf_bins=bins, reference=reference)
                emit(case, packets)
    (OUT/'packets.bin').write_bytes(blob)
    (OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,
        exports=[dict(c, name=c['name']+'-matroska', video=c['file']) for c in cases],
        provenance='Own nearest-defined LD table inputs: 480/512, ten configured layouts, low/high indexed and explicit rates, all8 spatial trees. Frozen independent sparse-cosine core reference in ../aac-ld-layout-reference.f32le. No private/ISO media, audio or codec parameter sets copied; no FFmpeg/network/process generation.'), separators=(',', ':'))+'\n')
    print(len(cases), 'programmes, MP4/Matroska')


if __name__ == '__main__':
    main()
