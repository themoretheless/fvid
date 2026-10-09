#!/usr/bin/env python3
"""Authored Main/LC late FIL at fixed core clock; no external decoder."""
import json
import struct
from generate_aac_main_prediction_fixtures import CODES, LENS, SC, SL
from generate_aac_main_sbr_oracle import core, reference
from generate_he_aac_packet_fixtures import DEST, field, frequency, packed, video_fixture


def channel(frame, object_type, active):
    prediction = object_type == 1 and active and frame >= 3
    info = '0000' + field(1, 6) + ('101' if prediction else '0')
    if not active:
        return field(140, 8) + info + '0000' + field(1, 5) + '000'
    index = 0
    for value in ([1, -1, 1, -1] if frame % 2 else [-1, 1, -1, 1]):
        index = index * 3 + value + 1
    return (field(140, 8) + info + field(1, 4) + field(1, 5)
            + field(SC[60], SL[60]) + '000' + field(CODES[index], LENS[index]))


def asc(object_type, coupled):
    prefix = field(object_type, 5) + frequency(24000) + field(0 if coupled else 1, 4) + '000'
    if not coupled:
        return packed(prefix)
    # One front SCE0 and one independent CCE1; byte alignment belongs to ASC.
    pce = (field(0, 4) + field(object_type - 1, 2) + frequency(24000)
           + field(1, 4) + field(0, 4) + field(0, 4) + field(0, 2)
           + field(0, 3) + field(1, 4) + '000' + '00000' + '10001')
    bits = prefix + pce
    return packed(bits + '0' * (-len(bits) % 8) + field(0, 8))


def main():
    syntax = (DEST / 'aac-sbr-dsp-syntax.bin').read_bytes()
    geometry = next(c for c in json.loads((DEST / 'aac-sbr-dsp-oracles.json').read_text())['cases']
                    if c['slots'] == 16 and c['bands'] == 64 and c['limiter'] == 0 and not c['smoothing'])
    blob = bytearray()
    cases = []
    for object_type, name in [(1, 'main'), (2, 'lc')]:
        pcm = core(prediction=object_type == 1)
        warm = reference(pcm_override=pcm, bands=32, first_sbr_frame=3)
        gold = pcm[:3072] + warm[3072:]
        reference_name = 'aac-late-sbr-fixed-' + name + '-reference.f64le'
        core_name = 'aac-late-sbr-fixed-' + name + '-core.f32le'
        (DEST / reference_name).write_bytes(struct.pack('<6144d', *gold))
        (DEST / core_name).write_bytes(struct.pack('<6144f', *pcm))
        for coupled in [False, True]:
            for late in [False, True]:
                frames = []
                for i in range(6):
                    target = '0000000' + channel(i, object_type, not coupled)
                    source = ('0100001' + '1' + '000' + '0' + '0000' + '0' + '0' + '10'
                              + channel(i, object_type, True)) if coupled else ''
                    fill = ''
                    if late and i >= 3:
                        row = geometry['frames'][(i - 3) % 3]
                        payload = syntax[row['offset']:row['offset'] + row['byte_length']]
                        n = len(payload)
                        fill = '110' + (field(n, 4) if n < 15 else '1111' + field(n - 14, 8))
                        fill += ''.join(field(b, 8) for b in payload)
                    packet = packed(target + source + fill + '111')
                    frames.append(dict(offset=len(blob), bytes=len(packet)))
                    blob.extend(packet)
                case_name = name + ('-cce' if coupled else '-sce') + ('-late' if late else '-core')
                case = dict(name=case_name, asc=asc(object_type, coupled).hex(), frames=frames,
                            container_rate=24000, container_frame_samples=1024, samples=6144,
                            slots=16, bands=32, pcm_offset=0, late=late,
                            reference=reference_name if late else core_name)
                case['video'] = video_fixture([case], blob, filename='aac-late-sbr-fixed-' + case_name + '-synthetic.mp4')
                cases.append(case)
    (DEST / 'aac-late-sbr-fixed-packets.bin').write_bytes(blob)
    (DEST / 'aac-late-sbr-fixed.json').write_text(json.dumps(dict(cases=cases, first_sbr_packet=3,
        provenance='Own integer spectra, Main scalar predictor/direct IMDCT, full 32-band pre-FIL QMF and direct SBR convolutions; LC disables Main prediction. Independent unit CCE1 or direct SCE0. No private media, decoder or network.'), indent=2) + '\n')


if __name__ == '__main__':
    main()
