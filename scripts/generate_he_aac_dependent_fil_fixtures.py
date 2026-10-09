#!/usr/bin/env python3
"""Original dependent CCE FIL syntax, compared with qualified no-CCE-FIL PCM."""
import hashlib
import json
from generate_aac_ps_fixtures import ps, sbr
from generate_he_aac_dependent_coupling_fixtures import DEST, packet, mono_payload, tables, video_fixture


def main():
    baseline = json.loads((DEST / 'he-aac-dependent-sbr-oracles.json').read_text())
    blob, cases = bytearray(), []
    nhigh = len(tables(10, 27, 0, False, 0, 0)[1]) - 1
    for reference in baseline['cases']:
        for mode in ['all', 'missing']:
            frames = []
            for index, row in enumerate(reference['frames']):
                cce = mono_payload(nhigh, 2, True, index) if mode == 'all' or index != 2 else b''
                data = packet(index, reference['channels'] == 2, reference['point'], bytes.fromhex(row['sbr']), cce_raw=cce)
                frames.append(dict(offset=len(blob), bytes=len(data), sbr=row['sbr'], cce_sbr=cce.hex()))
                blob.extend(data)
            c = dict(reference, frames=frames, baseline_frames=reference['frames'], mode=mode, video=None)
            if c['bands'] == 64:
                name = reference['video']['file'].replace('dependent-sbr-', 'dependent-fil-').replace('-synthetic.mp4', f'-{mode}-synthetic.mp4')
                c['video'] = video_fixture([c], blob, channels=c['channels'], filename=name)
            cases.append(c)
    invalid = []
    for failure in ['crc', 'ps']:
        for stereo in [False, True]:
            reference = next(c for c in baseline['cases'] if c['slots'] == 16 and c['channels'] == (2 if stereo else 1)
                             and c['bands'] == 64 and c['kind'] == 'explicit' and c['frames'][0]['sbr'])
            frames = []
            for index, row in enumerate(reference['frames'][:3]):
                cce = bytearray(mono_payload(nhigh, 2, True, index))
                if index == 1:
                    if failure == 'crc':
                        cce[0] ^= 1
                    else:
                        cce = sbr(ps(1, 1, 0, slots=32)[0], 0)
                data = packet(index, stereo, reference['point'], bytes.fromhex(row['sbr']), cce_raw=bytes(cce))
                frames.append(dict(offset=len(blob), bytes=len(data), sbr=row['sbr'], cce_sbr=cce.hex()))
                blob.extend(data)
            c = dict(reference, frames=frames, samples=6144, baseline_frames=reference['frames'][:3])
            c['video'] = video_fixture([c], blob, channels=c['channels'], filename=f'he-aac-dependent-fil-{failure}-{int(stereo)}-synthetic.mp4')
            c['error'] = 'SBR CRC' if failure == 'crc' else 'SBR extended audio/PS synthesis is not yet implemented'
            invalid.append(c)
    (DEST / 'he-aac-dependent-fil-packets.bin').write_bytes(blob)
    (DEST / 'he-aac-dependent-fil-oracles.json').write_text(json.dumps(dict(
        kind='own dependent CCE FIL; PCM must equal already qualified spectral-coupling baseline',
        cases=cases, invalid=invalid, packet_sha256=hashlib.sha256(blob).hexdigest()), indent=2) + '\n')
    print(len(cases), 'dependent FIL cases;', sum(c['video'] is not None for c in cases), 'acceptance videos')


if __name__ == '__main__':
    main()
