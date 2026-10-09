#!/usr/bin/env python3
"""Owned independently switched SSR target/CCE windows with scalar SBR gold."""
import json
import struct
from generate_aac_ssr_fixtures import channel
from generate_aac_ssr_coupling_fixtures import program, silent
from generate_aac_main_sbr_oracle import reference
from generate_he_aac_packet_fixtures import DEST, field, frequency, packed, video_fixture


def main():
    alignment = json.loads((DEST / 'aac-ssr-alignment.json').read_text())
    core_blob = (DEST / 'aac-ssr-alignment-pcm.f32le').read_bytes()
    syntax = (DEST / 'aac-sbr-dsp-syntax.bin').read_bytes()
    geometry = next(c for c in json.loads((DEST / 'aac-sbr-dsp-oracles.json').read_text())['cases']
                    if c['slots'] == 16 and c['bands'] == 64 and c['limiter'] == 0 and not c['smoothing'])
    blob = bytearray()
    cases = []
    for original in alignment['cases']:
        name = original['name']
        offset = original['pcm_offset']
        raw_core = core_blob[offset:offset + original['pcm_bytes']]
        assert len(raw_core) == 6144 * 4
        pcm = [s[0] for s in struct.iter_unpack('<f', raw_core)]
        gold_files = {}
        for rate in (24000, 48000):
            gold = reference(pcm_override=pcm, bands=rate // 750)
            filename = 'aac-ssr-sbr-drift-' + name + '-' + str(rate) + '.f64le'
            (DEST / filename).write_bytes(struct.pack('<' + str(len(gold)) + 'd', *gold))
            gold_files[rate] = filename
        for signal, rate in [('core', 24000)] + [(signal, rate) for signal in ('explicit', 'sync', 'implicit') for rate in (24000, 48000)]:
            frames = []
            for i, (target_seq, source_seq) in enumerate(zip(original['target_sequences'], original['source_sequences'])):
                target = '0000000' + silent(target_seq, 0, 0, False, False)
                source = '0100001' + '1' + '000' + '0' + '0000' + '0' + '0' + '10' + channel(i, source_seq, i % 2, 0, True, False)
                fill = ''
                if signal != 'core':
                    row = geometry['frames'][i % 3]
                    payload = syntax[row['offset']:row['offset'] + row['byte_length']]
                    n = len(payload)
                    fill = '110' + (field(n, 4) if n < 15 else '1111' + field(n - 14, 8))
                    fill += ''.join(field(b, 8) for b in payload)
                # FIL immediately follows its own CCE, even when element order changes.
                packet = packed((source + fill + target if i % 2 else target + source + fill) + '111')
                frames.append(dict(offset=len(blob), bytes=len(packet)))
                blob.extend(packet)
            if signal == 'explicit':
                prefix = field(5, 5) + frequency(24000) + '0000' + frequency(rate) + field(3, 5) + '000'
            else:
                prefix = field(3, 5) + frequency(24000) + '0000' + '000'
            config = program(prefix, 1, 3)
            if signal == 'sync':
                config += field(0x2b7, 11) + field(5, 5) + '1' + frequency(rate)
            case_name = name + '-' + signal + '-' + str(rate)
            case = dict(name=case_name, asc=packed(config).hex(), frames=frames, channels=1,
                        container_rate=rate, container_frame_samples=rate // 24000 * 1024,
                        samples=rate // 24000 * 6144, slots=16, bands=rate // 750,
                        pcm_offset=0, core=signal == 'core', core_offset=offset,
                        reference='aac-ssr-alignment-pcm.f32le' if signal == 'core' else gold_files[rate],
                        target_sequences=original['target_sequences'], source_sequences=original['source_sequences'])
            case['video'] = video_fixture([case], blob, filename='aac-ssr-sbr-drift-' + case_name + '-synthetic.mp4')
            cases.append(case)
    (DEST / 'aac-ssr-sbr-drift-packets.bin').write_bytes(blob)
    (DEST / 'aac-ssr-sbr-drift.json').write_text(json.dumps(dict(cases=cases,
        provenance='Own SSR spectra/gain and alternating sine/KBD source windows; independent core IPQF oracle from aac-ssr-alignment, then scalar QMF/HF/noise convolution. Five unequal target/source schedules, explicit/sync/implicit at both clocks. No private media, decoder, FFmpeg or network.'), indent=2) + '\n')


if __name__ == '__main__':
    main()
