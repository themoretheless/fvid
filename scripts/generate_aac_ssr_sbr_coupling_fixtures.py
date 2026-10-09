#!/usr/bin/env python3
"""Authored SSR independent CCE with its own SBR FIL, no external codec."""
import json, struct
from generate_aac_main_sbr_oracle import reference
from generate_aac_ssr_coupling_fixtures import program, silent
from generate_aac_ssr_fixtures import channel, info, SC, SL
from generate_he_aac_packet_fixtures import DEST, field, frequency, packed, video_fixture

def main():
    syntax = (DEST / 'aac-sbr-dsp-syntax.bin').read_bytes()
    ref = next(c for c in json.loads((DEST / 'aac-sbr-dsp-oracles.json').read_text())['cases']
               if c['slots'] == 16 and c['bands'] == 64 and c['limiter'] == 0 and not c['smoothing'])
    blob = bytearray()
    cases = []
    specs = [(sbr, channels, tags, deltas, 2, 0) for sbr, channels, tags, deltas in
             [(False,1,(1,),None), (True,1,(1,),None), (True,1,(1,15),None),
              (True,2,(1,),None), (True,2,(1,15),None), (True,2,(1,),[2]*6),
              (True,2,(1,),[0,2,-2,2,0,-2])]]
    specs += [(True,2,(1,),[0,1,-1,2,-2,1],scale,sign)
              for scale in range(4) for sign in range(2)]
    for sbr, channels, tags, deltas, scale, sign in specs:
        rows = []
        for i, seq in enumerate([0, 1, 2, 2, 3, 0]):
            target = ('0000000' + silent(seq, 0, 0, False, False) if channels == 1
                      else '0010000' + '1' + info(seq,0) + '00' + ''.join(silent(seq,0,c,False,True) for c in range(2)))
            sources = []
            fill = ''
            if sbr:
                row = ref['frames'][i % 3]
                raw = syntax[row['offset']:row['offset'] + row['byte_length']]
                n = len(raw)
                fill = '110' + (field(n, 4) if n < 15 else '1111' + field(n - 14, 8)) + ''.join(field(b, 8) for b in raw)
            for tag in tags:
                source = '010' + field(tag,4) + '1' + '000' + field(channels==2,1) + '0000' + (('11' if deltas else '00') if channels==2 else '') + '0' + field(sign,1) + field(scale,2) + channel(i, seq, 0, 0, True, False)
                if deltas:
                    source += field(SC[60+deltas[i]],SL[60+deltas[i]])
                sources.append(source + fill)
            packet = packed(target + ''.join(sources) + '111')
            rows.append(dict(offset=len(blob), bytes=len(packet)))
            blob.extend(packet)
        prefix = (field(5, 5) + frequency(24000) + '0000' + frequency(48000) + field(3, 5) + '000'
                  if sbr else field(3, 5) + frequency(24000) + '0000' + '000')
        case = dict(name=('sbr' if channels==1 and len(tags)==1 else 'sbr-'+str(channels)+'-'+str(len(tags))) if sbr else 'core-control', channels=channels, factor=len(tags), asc=packed(program(prefix, channels, 3,tags)).hex(), frames=rows,
                    slots=16, bands=64, container_rate=48000 if sbr else 24000,
                    container_frame_samples=2048 if sbr else 1024, samples=12288 if sbr else 6144,
                    pcm_offset=0, reference='aac-ssr-sbr-active-reference.f64le' if sbr else 'aac-ssr-sbr-active-core-reference.f32le')
        if deltas:
            case['name'] += '-gain-' + ('static' if len(set(deltas))==1 else 'varying')
            if deltas == [0,1,-1,2,-2,1]:
                case['name'] += '-scale-' + str(scale) + '-sign-' + str(sign)
            case['right_gain'] = [2.0**(-d*[0.125,0.25,0.5,1.0][scale]) for d in deltas]
            case['gain_scale'] = scale
            case['gain_sign'] = sign
            case['gain_core_rows'] = [1024,1472,1024,1024,576,1024]
        case['video'] = video_fixture([case], blob, channels=channels, filename='aac-ssr-sbr-cce-' + case['name'] + '-synthetic.mp4')
        cases.append(case)
    core = (DEST / 'aac-ssr-sbr-active-core-reference.f32le').read_bytes()
    scalar = reference(pcm_override=[v[0] for v in struct.iter_unpack('<f',core)], bands=32)
    reference_file = 'aac-ssr-sbr-downsampled-reference.f64le'
    (DEST / reference_file).write_bytes(struct.pack('<'+str(len(scalar))+'d',*scalar))
    for base in list(cases[1:]):
        channels=base['channels'];tags=(1,15) if base['factor']==2 else (1,)
        prefix=field(5,5)+frequency(24000)+'0000'+frequency(24000)+field(3,5)+'000'
        case=dict(base,name=base['name']+'-downsampled',asc=packed(program(prefix,channels,3,tags)).hex(),
                  container_rate=24000,container_frame_samples=1024,samples=6144,reference=reference_file)
        case['video']=video_fixture([case],blob,channels=channels,filename='aac-ssr-sbr-cce-'+case['name']+'-synthetic.mp4')
        cases.append(case)
    for base in list(cases):
        if not base['name'].endswith('-downsampled'):continue
        channels=base['channels'];tags=(1,15) if base['factor']==2 else (1,)
        prefix=field(3,5)+frequency(24000)+'0000'+'000'
        case=dict(base,name=base['name']+'-implicit',asc=packed(program(prefix,channels,3,tags)).hex())
        case['video']=video_fixture([case],blob,channels=channels,filename='aac-ssr-sbr-cce-'+case['name']+'-synthetic.mp4')
        cases.append(case)
    (DEST / 'aac-ssr-sbr-cce-packets.bin').write_bytes(blob)
    (DEST / 'aac-ssr-sbr-cce.json').write_text(json.dumps(dict(cases=cases,
        provenance='Owned silent target, active SSR CCE1, unit and positive separate-channel independent gain and authored SBR on CCE only. Existing independent SSR/IPQF and SBR reference PCM; no private media or external codec.'), indent=2) + '\n')

if __name__ == '__main__':
    main()
