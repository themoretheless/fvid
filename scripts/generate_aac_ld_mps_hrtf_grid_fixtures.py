#!/usr/bin/env python3
"""Own one-AU HRTF-grid admission reproducers; not unequal-grid acceptance.

Explicit offline generation from existing authored cores/spatial parameters.
No private/ISO media, external codec, FFmpeg or network access.
"""
import json
from pathlib import Path
from generate_he_aac_packet_fixtures import field, packed, video_fixture
from generate_aac_ld_mps_inband_fixtures import raw
from generate_aac_ld_mps_transport_fixtures import envelope, length

ROOT = Path(__file__).resolve().parents[1]
FIX = ROOT/'tests/fixtures/playback-errors'
OUT = FIX/'aac-ld-mps-hrtf-grids'
GRIDS = [23, 15, 12, 9, 7, 5, 4]


def main():
    parent = FIX/'aac-ld-mps-3d'
    manifest = json.loads((parent/'manifest.json').read_text())
    source = (parent/'packets.bin').read_bytes()
    blob, cases = bytearray(), []
    OUT.mkdir(exist_ok=True)
    for spatial in [23, 9, 4]:
        base = next(c for c in manifest['cases'] if c['tree'] == 2
                    and c['variant'] == 0 and c['bands'] == spatial
                    and c['pattern'] == 'asymmetric')
        old = bytes.fromhex(base['ssc'])
        asc_bits = raw(bytes.fromhex(base['asc']))
        marker = field(2, 4)+length(len(old))+raw(old)+'000000'
        start = asc_bits.rfind(marker)
        if start < 0:
            raise ValueError('authored ASC spatial descriptor not found')
        # Reuse the independent two-channel control's exact core prefix;
        # the spatial corpus rows contain the complete extended packets.
        control = next(c for c in manifest['controls'] if c['name'] == base['name'].split('-tree')[0])
        r = control['frames'][0]
        prefix = raw(source[r['offset']:r['offset']+r['bytes']])[:r['core_bits']]
        for grid in GRIDS:
            hrtf = '00'+field(GRIDS.index(grid)+1, 3)+'1'
            for channel in base['hrtf']:
                for key in ['left', 'right']:
                    hrtf += ''.join(field(channel[key][i % spatial], 6) for i in range(grid))
                hrtf += '1'+''.join(field(channel['phase'][i % spatial], 6) for i in range(grid))
            ssc = packed(raw(old)[:52]+hrtf)
            if grid == spatial and ssc != old:
                raise ValueError('equal-grid control changed the original configuration')
            asc = packed(asc_bits[:start]+field(2, 4)+length(len(ssc))+raw(ssc)+'000000')
            header = bytes([len(ssc)]) if len(ssc) < 127 else b'\x7f'+(len(ssc)-127).to_bytes(2, 'big')
            packet = packed(prefix+envelope(9, 1, 3, header+ssc+bytes.fromhex(base['reference'][0]), True))
            frame = dict(offset=len(blob), bytes=len(packet))
            blob += packet
            name = f'spatial{spatial}-hrtf{grid}'
            case = dict(base, name=name, asc=asc.hex(), ssc=ssc.hex(),
                        frames=[frame], samples=base['out_n'],
                        video=name+'.mp4', spatial_bands=spatial, hrtf_bands=grid,
                        source_video=base['video'], source_core=control['name'],
                        spatial_frame=base['reference'][0], acceptance=grid == spatial)
            if grid != spatial:
                case['error'] = 'HRTF grid conversion is not yet qualified'
            (OUT/case['video']).write_bytes(video_fixture([case], blob, case['channels'], write=False)['data'])
            # Keep metadata small; source provenance remains in the parent.
            cases.append({k: case[k] for k in ['name', 'asc', 'ssc', 'frames', 'samples',
                'rate', 'channels', 'video', 'spatial_bands', 'hrtf_bands', 'source_video',
                'source_core', 'spatial_frame', 'acceptance']}
                | ({'error': case['error']} if grid != spatial else {}))
    (OUT/'packets.bin').write_bytes(blob)
    (OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,
        scope='Three equal-grid playback controls and eighteen unequal-grid '
        'specific refusal reproducers. Own 1-AU video/audio; not unequal-grid '
        'playback acceptance or a grid-conversion oracle.'), indent=2)+'\n')
    print('3 equal-grid controls;18 unequal-grid refusal reproducers')


if __name__ == '__main__':
    main()
