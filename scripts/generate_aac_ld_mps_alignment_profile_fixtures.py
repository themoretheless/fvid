#!/usr/bin/env python3
"""Own three-AU LD-MPS alignment-profile controls. Explicit offline generation."""
import json
from pathlib import Path
from generate_aac_ld_mps_inband_fixtures import raw
from generate_aac_ld_mps_transport_fixtures import packed, envelope
from generate_he_aac_packet_fixtures import video_fixture

ROOT = Path(__file__).resolve().parents[1]
FIX = ROOT/'tests/fixtures/playback-errors'
OUT = FIX/'aac-ld-mps-alignment-profile'


def main():
    parent = FIX/'aac-ld-mps-inband'
    manifest = json.loads((parent/'manifest.json').read_text())
    base = next(c for c in manifest['cases'] if c['name'] == 'plain-tree7-repeat')
    source = (parent/'packets.bin').read_bytes()
    ssc, body = bytes.fromhex(base['configs'][0]), bytes.fromhex(base['body'])
    blob, cases = bytearray(), []
    OUT.mkdir(exist_ok=True)
    for value in [0, 1, 32767, 32768, 65535]:
        frames = []
        for index, row in enumerate(base['frames']):
            prefix = raw(source[row['offset']:row['offset']+row['bytes']])[:row['prefix_bits']]
            delay = value if index == 1 else 0
            payload = bytes([128+len(ssc)])+ssc+delay.to_bytes(2, 'big')+body
            packet = packed(prefix+envelope(9, 1, 3, payload, True))
            frames.append(dict(offset=len(blob), bytes=len(packet)))
            blob += packet
        name = f'alignment-{value}'
        case = dict(base, name=name, frames=frames, video=name+'.mp4', alignment=value)
        (OUT/case['video']).write_bytes(video_fixture([case], blob, 2, write=False)['data'])
        cases.append({k: case[k] for k in ['name', 'video', 'asc', 'frames', 'alignment', 'samples', 'rate']})
    (OUT/'packets.bin').write_bytes(blob)
    (OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,
        scope='Own ELD mono core, tree212 stereo output, explicit alignment flag '
        'with zero control and four forbidden nonzero 16-bit patterns. Not '
        'acceptance of nonzero alignment. No private or ISO media.'), indent=2)+'\n')


if __name__ == '__main__':
    main()
