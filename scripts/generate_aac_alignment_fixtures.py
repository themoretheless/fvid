#!/usr/bin/env python3
"""Own AAC-LD/ELD byte-alignment movies. Explicit offline generation only."""
import json
import struct
from pathlib import Path
from generate_aac_eld_fixtures import generate, mkv
from generate_aac_ld_layout_fixtures import LAYOUTS, geometry, body
from generate_aac_ld_mps_transport_fixtures import envelope, field, packed
from generate_he_aac_packet_fixtures import frequency, video_fixture

ROOT = Path(__file__).resolve().parents[1]
FIX = ROOT / 'tests/fixtures/playback-errors'
OUT = FIX / 'aac-alignment'


def raw(data):
    return ''.join(field(v, 8) for v in data)


def main():
    OUT.mkdir(exist_ok=True)
    blob = bytearray()
    cases = []

    def programme(name, asc, wires, channels, n=512, rate=48000):
        variants = []
        for pattern in ['zero', 'ones', 'alternating', 'last-one']:
            rows = []
            packets = []
            changed = 0
            for wire in wires:
                padding = -len(wire) % 8
                tail = ('0' * padding if pattern == 'zero' else
                        '1' * padding if pattern == 'ones' else
                        ('10' * 4)[:padding] if pattern == 'alternating' else
                        '0' * max(0, padding - 1) + ('1' if padding else ''))
                packet = packed(wire + tail)
                changed += '1' in tail
                rows.append(dict(offset=len(blob), bytes=len(packet), syntax_bits=len(wire)))
                blob.extend(packet)
                packets.append(packet)
            assert pattern == 'zero' or changed > 0
            stem = name + '-' + pattern
            case = dict(name=stem, asc=asc.hex(), channels=channels,
                        container_rate=rate, container_frame_samples=n, slots=8, bands=64,
                        samples=3*n, pcm_offset=0, frames=rows, video=stem+'.mp4', file=stem+'.mkv')
            (OUT / case['video']).write_bytes(video_fixture([case], blob, channels, write=False)['data'])
            (OUT / case['file']).write_bytes(mkv(asc, packets, rate, n, channels))
            variants.append(case)
        cases.append(dict(name=name, variants=variants))

    parent = FIX / 'aac-eld-mps-core'
    for tree in range(8):
        channels = [6, 6, 6, 8, 8, 8, 8, 2][tree]
        wires = []
        for i in range(3):
            _, end = struct.unpack('<II', (parent / f'tree{tree}-{i}.bits').read_bytes())
            wires.append(raw((parent / f'tree{tree}-{i}.au').read_bytes())[:end])
        programme(f'eld-tree{tree}', (parent / f'tree{tree}.spatial.asc').read_bytes(), wires, channels)
        inputs = 1 if tree in (0, 1, 7) else 2 if tree in (2, 3, 4) else 6
        ssc = (parent / f'tree{tree}.ssc').read_bytes()
        asc = packed(field(23, 5)+frequency(48000)+field(inputs, 4)+'00000'+field(0x7cc, 11)+'1'+field(len(ssc), 8)+raw(ssc))
        count, high = geometry(512, 48000)
        wires = []
        for i in range(3):
            wire = ''
            channel = 0
            for element in LAYOUTS[inputs][0]:
                wire += '0000' + ('0' if element == 1 else '')
                for _ in range(2 if element == 1 else 1):
                    encoded, _ = body(512, count, high, i, channel, lfe=element == 3)
                    wire += encoded
                    channel += 1
            wire += envelope(9, 0, 3, (parent / f'tree{tree}-{i}.spatial').read_bytes(), True)
            wires.append(wire)
        programme(f'ld-tree{tree}', asc, wires, channels)

    generated = generate(include_payload_bits=True, specs=[(n, 48000, c, 'base', 0, 0, False) for n in (480, 512) for c in (1, 2, 6)])
    for c in json.loads(generated['manifest.json'])['cases']:
        programme(f'eld-plain-n{c["n"]}-c{c["channels"]}', bytes.fromhex(c['asc']), [f['bits'] for f in c['frames'][:3]], c['channels'], c['n'])
    # Shared spatial LD-SBR at both bank sizes, including CRC and coupled stereo.
    parent = FIX / 'aac-eld-spatial-sbr'
    data = (parent / 'packets.bin').read_bytes()
    for c in json.loads((parent / 'manifest.json').read_text())['cases']:
        if c['configuration'] != 2 or c['mode'] != 'coupled' or not c['name'].endswith('crc1'):
            continue
        wires = []
        for f in c['frames'][:3]:
            padding = -(f['core_bits'] + f['sbr_bits']) % 8
            packet = data[f['offset']:f['offset']+f['bytes']]
            assert not padding or raw(packet)[-padding:] == '0'*padding
            wires.append(raw(packet)[:len(packet)*8-padding])
        programme('shared-sbr-'+c['name'], bytes.fromhex(c['asc']), wires, 6,
                  c['container_frame_samples'], c['rate'])
    # SBR via CLDFB PCM ingress, including adapted and 96-kHz spatial clocks.
    for corpus in ['aac-ld-mps-sbr-pcm', 'aac-ld-mps-sbr-rates']:
        parent = FIX / corpus
        data = (parent / 'packets.bin').read_bytes()
        seen = set()
        for c in json.loads((parent / 'manifest.json').read_text())['cases']:
            key = c['rate'], c['factor']
            if key in seen or c['group'] != 1 or not any(f['core_bits'] % 8 for f in c['frames'][:3]):
                continue
            seen.add(key)
            wires = [raw(data[f['offset']:f['offset']+f['bytes']])[:f['core_bits']] for f in c['frames'][:3]]
            programme('pcm-sbr-'+c['name'], bytes.fromhex(c['asc']), wires, c['channels'], c['out_n'], c['rate'])
    (OUT / 'packets.bin').write_bytes(blob)
    (OUT / 'manifest.json').write_text(json.dumps(dict(programmes=cases, cases=[v for c in cases for v in c['variants']], exports=[dict(v, video=v['file'], name=v['name']+'-matroska') for c in cases for v in c['variants']], scope='Own nonzero AAC-LD/ELD cores and spatial payloads; only final AAC byte-alignment bits differ. No ISO/private media copied.'), separators=(',', ':'))+'\n')
    print(len(cases), 'programmes, four alignment patterns, MP4 and Matroska')


if __name__ == '__main__':
    main()
