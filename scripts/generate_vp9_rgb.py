#!/usr/bin/env python3
"""Authored VP9 identity RGB packets and analytic sample oracles; no external codecs."""
import hashlib
import json
from generate_vp9_chroma import DEST, coloured, mp4, webm

def main():
    cases = []
    invalid = []
    for depth in (8, 10, 12):
        packet, planar = coloured(depth, '444', rgb=True)
        plane = 64 * 64 * 2
        rgb = bytearray()
        for i in range(64 * 64):
            for channel in (2, 0, 1):
                sample = planar[channel * plane + i * 2:channel * plane + i * 2 + 2]
                rgb.extend(sample[:1] if depth == 8 else sample)
        prefix = f'vp9-rgb-depth{depth}'
        files = {'packet': packet, 'rgb': bytes(rgb), 'webm': webm([packet, packet]),
                 'mp4': mp4([packet, packet], depth, '444')}
        # vpcC: full range, BT.709 primaries, sRGB transfer, identity matrix.
        container = bytearray(files['mp4'])
        at = container.index(b'vpcC') + 4
        container[at + 6] |= 1
        container[at + 7:at + 10] = bytes((1, 13, 0))
        files['mp4'] = bytes(container)
        if depth == 8:
            for name, offset, value in [('profile', 4, 0), ('depth', 6, (10 << 4) | 7),
                                        ('range', 6, (8 << 4) | 6), ('subsampling', 6, (8 << 4) | 5)]:
                malformed = bytearray(container)
                malformed[at + offset] = value
                file = f'{prefix}-config-{name}-mismatch.mp4'
                (DEST / file).write_bytes(malformed)
                invalid.append({'file': file, 'sha256': hashlib.sha256(malformed).hexdigest(),
                                'error': 'MP4 VP9 configuration disagrees with picture format'})
        entries = {}
        for extension, data in files.items():
            name = f'{prefix}.{extension}'
            (DEST / name).write_bytes(data)
            entries[extension] = {'file': name, 'sha256': hashlib.sha256(data).hexdigest()}
        cases.append({'depth': depth, 'width': 64, 'height': 64, 'frames': 2, 'files': entries})
    (DEST / 'vp9-rgb.json').write_text(json.dumps({'cases': cases, 'invalid': invalid, 'provenance':
        'Authored VP9 profile 1/3 identity headers, owned Python Boolean encoder, independent DC prediction/WHT sample oracle; G/B/R planes interleaved as RGB. No private media, external codec or network.'}, indent=2) + '\n')

if __name__ == '__main__':
    main()
