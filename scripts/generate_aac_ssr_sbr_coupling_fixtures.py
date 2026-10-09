#!/usr/bin/env python3
"""Authored SSR independent CCE with its own SBR FIL, no external codec."""
import json
from generate_aac_ssr_coupling_fixtures import program, silent
from generate_aac_ssr_fixtures import channel
from generate_he_aac_packet_fixtures import DEST, field, frequency, packed, video_fixture

def main():
    syntax = (DEST / 'aac-sbr-dsp-syntax.bin').read_bytes()
    ref = next(c for c in json.loads((DEST / 'aac-sbr-dsp-oracles.json').read_text())['cases']
               if c['slots'] == 16 and c['bands'] == 64 and c['limiter'] == 0 and not c['smoothing'])
    blob = bytearray()
    cases = []
    for sbr in [False, True]:
        rows = []
        for i, seq in enumerate([0, 1, 2, 2, 3, 0]):
            target = '0000000' + silent(seq, 0, 0, False, False)
            source = '0100001' + '1' + '000' + '0' + '0000' + '0' + '0' + '10' + channel(i, seq, 0, 0, True, False)
            fill = ''
            if sbr:
                row = ref['frames'][i % 3]
                raw = syntax[row['offset']:row['offset'] + row['byte_length']]
                n = len(raw)
                fill = '110' + (field(n, 4) if n < 15 else '1111' + field(n - 14, 8)) + ''.join(field(b, 8) for b in raw)
            packet = packed(target + source + fill + '111')
            rows.append(dict(offset=len(blob), bytes=len(packet)))
            blob.extend(packet)
        prefix = (field(5, 5) + frequency(24000) + '0000' + frequency(48000) + field(3, 5) + '000'
                  if sbr else field(3, 5) + frequency(24000) + '0000' + '000')
        case = dict(name='sbr' if sbr else 'core-control', asc=packed(program(prefix, 1, 3)).hex(), frames=rows,
                    slots=16, bands=64, container_rate=48000 if sbr else 24000,
                    container_frame_samples=2048 if sbr else 1024, samples=12288 if sbr else 6144,
                    pcm_offset=0, reference='aac-ssr-sbr-active-reference.f64le' if sbr else 'aac-ssr-sbr-active-core-reference.f32le')
        case['video'] = video_fixture([case], blob, filename='aac-ssr-sbr-cce-' + case['name'] + '-synthetic.mp4')
        cases.append(case)
    (DEST / 'aac-ssr-sbr-cce-packets.bin').write_bytes(blob)
    (DEST / 'aac-ssr-sbr-cce.json').write_text(json.dumps(dict(cases=cases,
        provenance='Owned silent target, active SSR CCE1, unit independent gain and authored SBR on CCE only. Existing independent SSR/IPQF and SBR reference PCM; no private media or external codec.'), indent=2) + '\n')

if __name__ == '__main__':
    main()
