#!/usr/bin/env python3
"""Explicit CCE absence preserves queued core PCM; authored SSR/SBR gold."""
import json
import struct
from generate_aac_ssr_fixtures import channel, SAMPLES
from generate_aac_ssr_coupling_fixtures import program, silent
from generate_aac_main_sbr_oracle import reference
from generate_he_aac_packet_fixtures import DEST, field, frequency, packed, video_fixture


def main():
    m = json.loads((DEST / 'aac-ssr-alignment.json').read_text())
    raw_core = (DEST / 'aac-ssr-alignment-pcm.f32le').read_bytes()
    syntax = (DEST / 'aac-sbr-dsp-syntax.bin').read_bytes()
    geometry = next(c for c in json.loads((DEST / 'aac-sbr-dsp-oracles.json').read_text())['cases']
                    if c['slots'] == 16 and c['bands'] == 64 and c['limiter'] == 0 and not c['smoothing'])
    blob = bytearray()
    cases = []
    for name, base_name, present in [('ahead', 'source-ahead', [1,1,0,0,0,0]),
                                   ('behind', 'opposite-switches', [1,1,0,0,0,0]),
                                   ('exact', 'source-ahead', [1,0,0,0,0,0]),
                                   ('resume', 'source-ahead', [1,1,0,1,1,0])]:
        base = next(c for c in m['cases'] if c['name'] == base_name)
        source = raw_core[base['pcm_offset']:base['pcm_offset'] + base['pcm_bytes']]
        source_pcm = [s[0] for s in struct.iter_unpack('<f', source)]
        pcm = []
        gains = []
        cursor = 0
        ordinal = 0
        for i, active in enumerate(present):
            if active:
                size = SAMPLES[base['source_sequences'][ordinal]]
                pcm += source_pcm[cursor:cursor + size]
                gains += [1] * size
                cursor += size
                ordinal += 1
            else:
                padding = max(0, (i + 1) * 1024 - len(pcm))
                pcm += [0.] * padding
                gains += [0] * padding
        assert len(pcm) == len(gains) == 6144
        core_name = 'aac-ssr-cce-absence-' + name + '-core.f32le'
        (DEST / core_name).write_bytes(struct.pack('<6144f', *pcm))
        gold_files = {}
        for rate in (24000,48000):
            ratio = rate // 24000
            gold = reference(pcm_override=pcm, bands=32 * ratio, sbr_frames=present)
            gold = [value * gains[i // ratio] for i, value in enumerate(gold)]
            filename = 'aac-ssr-cce-absence-' + name + '-' + str(rate) + '.f64le'
            (DEST / filename).write_bytes(struct.pack('<' + str(len(gold)) + 'd', *gold))
            gold_files[rate] = filename
        for rate, sbr in [(24000,False),(24000,True),(48000,True)]:
            frames = []
            ordinal = 0
            for i, active in enumerate(present):
                target = '0000000' + silent(0,0,0,False,False)
                coded = ''
                if active:
                    seq = base['source_sequences'][ordinal]
                    coded = '0100001' + '1' + '000' + '0' + '0000' + '0' + '0' + '10' + channel(ordinal,seq,ordinal % 2,0,True,False)
                    if sbr:
                        row = geometry['frames'][ordinal % 3]
                        payload = syntax[row['offset']:row['offset']+row['byte_length']]
                        n = len(payload)
                        coded += '110' + (field(n,4) if n<15 else '1111'+field(n-14,8)) + ''.join(field(b,8) for b in payload)
                    ordinal += 1
                packet = packed((coded + target if i % 2 else target + coded) + '111')
                frames.append(dict(offset=len(blob),bytes=len(packet)))
                blob.extend(packet)
            prefix = (field(5,5)+frequency(24000)+'0000'+frequency(rate)+field(3,5)+'000'
                      if sbr else field(3,5)+frequency(24000)+'0000'+'000')
            case_name = name + ('-sbr-' + str(rate) if sbr else '-core')
            case = dict(name=case_name,asc=packed(program(prefix,1,3)).hex(),frames=frames,channels=1,
                        container_rate=rate,container_frame_samples=1024 * (rate // 24000),samples=6144 * (rate // 24000),
                        slots=16,bands=32 * (rate // 24000),pcm_offset=0,core=not sbr,present=present,
                        reference=gold_files[rate] if sbr else core_name)
            case['video'] = video_fixture([case],blob,filename='aac-ssr-cce-absence-'+case_name+'-synthetic.mp4')
            cases.append(case)
    for base in list(cases):
        rows=[];previous=(1,)
        for i,row in enumerate(base['frames']):
            tags=(1,) if base['present'][i] else ()
            raw=bytes(blob[row['offset']:row['offset']+row['bytes']])
            if tags != previous:raw=packed(program('101',1,3,tags))+raw
            rows.append(dict(row,offset=len(blob),bytes=len(raw)));blob.extend(raw);previous=tags
        case=dict(base,name='pce-roster-'+base['name'],frames=rows,pce_roster=True)
        case['video']=video_fixture([case],blob,filename='aac-ssr-cce-absence-'+case['name']+'-synthetic.mp4')
        cases.append(case)
    invalid=[]
    base=next(c for c in cases if c['name']=='ahead-core')
    for name,channels,roster,error in [
        ('layout',2,(1,),'AAC in-band PCE changed the configured layout'),
        ('unconfigured-cce',1,(),'AAC coupling is absent from configured PCE')]:
        rows=[]
        for i,row in enumerate(base['frames']):
            raw=bytes(blob[row['offset']:row['offset']+row['bytes']])
            if i==1:raw=packed(program('101',channels,3,roster))+raw
            rows.append(dict(row,offset=len(blob),bytes=len(raw)));blob.extend(raw)
        case=dict(base,name='invalid-pce-'+name,frames=rows,error=error)
        case['video']=video_fixture([case],blob,filename='aac-ssr-cce-absence-'+case['name']+'-synthetic.mp4')
        invalid.append(case)
    (DEST/'aac-ssr-cce-absence-packets.bin').write_bytes(blob)
    (DEST/'aac-ssr-cce-absence.json').write_text(json.dumps(dict(cases=cases,invalid=invalid,
        provenance='Own SSR spectra/gain/window IPQF core oracle, explicit CCE1 gaps and paused coded-source history; retain queued PCM before zero absent intervals. Independent direct QMF/SBR with pure upsampling on absent FIL and preserved noise/smoothing state; apply original source chunk gains after DSP. No private media, decoder, FFmpeg or network.'),indent=2)+'\n')


if __name__=='__main__':main()
