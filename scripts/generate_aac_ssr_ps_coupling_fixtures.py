#!/usr/bin/env python3
"""Own SSR/PS CCE programs with independent scalar SSR core controls."""
import json, struct
from generate_aac_ssr_coupling_fixtures import program, silent, packet, config
from generate_aac_ssr_fixtures import channel, SEQUENCES, SAMPLES
from generate_he_aac_packet_fixtures import DEST, field, frequency, packed, video_fixture
from generate_aac_ps_coupling_fixtures import fill
from generate_he_aac_dependent_coupling_fixtures import mono_payload, tables


def ps_packet(i,seq,shape,source_shape,point,active,tags,payload,bad=None,source_payload=b''):
    target = '0000000'+silent(seq,shape,0,active,False)+fill(bytes.fromhex(payload))
    sources = []
    for tag in tags:
        shape_for_source = 1-source_shape if bad=='shape' else source_shape
        target_tag = 1 if bad=='target' and tag==tags[-1] else 0
        source = ('010'+field(tag,4)+field(point==3,1)+'000'+'0'+field(target_tag,4)+field(point==1,1)+'0'+'10'
                  +channel(i,seq,shape_for_source,0,active and point==3,False))
        sources.append(source+fill(source_payload))
    return packed((''.join(sources)+target if i%2 else target+''.join(sources))+'111')


def main():
    core_manifest = json.loads((DEST/'aac-ssr-coupling.json').read_text())
    core_gold = (DEST/'aac-ssr-coupling-pcm.f32le').read_bytes()
    ps = json.loads((DEST/'aac-ssr-ps.json').read_text())['payloads']
    blob = bytearray(); gold = bytearray(); controls = []; cases = []; invalid = []
    for point in (0,1,3):
        for active in (False,True):
            base = next(c for c in core_manifest['cases'] if c['channels']==1 and c['point']==point and c['active']==active and 'tags' not in c)
            for tags in ((1,), (1,15)):
                name = f'{point}-{int(active)}-{len(tags)}'
                reference = core_gold[base['pcm_offset']:base['pcm_offset']+base['pcm_bytes']]
                reference = b''.join(struct.pack('<f',v[0]*len(tags)) for v in struct.iter_unpack('<f',reference))
                offset = len(gold); gold.extend(reference)
                core_rows = []; ps_rows = []
                for i,seq in enumerate(SEQUENCES):
                    shape = base['frames'][i]['shape']; source_shape = base['frames'][i]['source_shape']
                    raw = packet(i,seq,shape,source_shape,1,point,active,tags=tags)
                    core_rows.append(dict(offset=len(blob),bytes=len(raw))); blob.extend(raw)
                    raw = ps_packet(i,seq,shape,source_shape,point,active,tags,ps[i%3])
                    ps_rows.append(dict(offset=len(blob),bytes=len(raw),payload=ps[i%3])); blob.extend(raw)
                control = dict(name=name,asc=config(1,point,tags).hex(),frames=core_rows,slots=16,bands=32,
                               pcm_offset=offset,pcm_bytes=len(reference),container_rate=24000,container_frame_samples=1472,
                               durations=[SAMPLES[s] for s in SEQUENCES],samples=6144)
                control['video'] = video_fixture([control],blob,filename='aac-ssr-ps-cce-'+name+'-core-control-synthetic.mp4')
                controls.append(control)
                for rate,bands in ((24000,32),(48000,64)):
                    prefix = field(29,5)+frequency(24000)+'0000'+frequency(rate)+field(3,5)+'000'
                    case = dict(name=name+'-'+str(rate),asc=packed(program(prefix,1,point,tags)).hex(),frames=ps_rows,
                                point=point,tags=list(tags),active=active,slots=16,bands=bands,container_rate=rate,
                                container_frame_samples=rate//24000*1024,samples=rate//24000*6144,
                                pcm_offset=offset,pcm_bytes=len(reference),provenance='independent original scalar SSR/IPQF core; separately qualified owned SBR/PS composition')
                    case['video'] = video_fixture([case],blob,channels=2,filename='aac-ssr-ps-cce-'+case['name']+'-synthetic.mp4')
                    cases.append(case)
    for active in (False,True):
        baseline = next(c for c in cases if c['point']==0 and c['active']==active and c['tags']==[1] and c['bands']==32)
        core_rows = []; ps_rows = []
        for i,seq in enumerate(SEQUENCES):
            shape = int(i%2==0)
            target = '0000000'+channel(i,seq,shape,0,active,False)
            raw = packed(target+'111')
            core_rows.append(dict(offset=len(blob),bytes=len(raw)));blob.extend(raw)
            raw = packed(target+fill(bytes.fromhex(ps[i%3]))+'111')
            ps_rows.append(dict(offset=len(blob),bytes=len(raw),payload=ps[i%3]));blob.extend(raw)
        control = dict(controls[0],name='mono-'+str(int(active)),asc=config(1,0,()).hex(),frames=core_rows,
                       pcm_offset=baseline['pcm_offset'],pcm_bytes=baseline['pcm_bytes'])
        control['video'] = video_fixture([control],blob,filename='aac-ssr-ps-cce-'+control['name']+'-core-control-synthetic.mp4')
        controls.append(control)
        for rate,bands in ((24000,32),(48000,64)):
            prefix = field(29,5)+frequency(24000)+'0000'+frequency(rate)+field(3,5)+'000'
            case = dict(baseline,name=control['name']+'-'+str(rate),asc=packed(program(prefix,1,0,())).hex(),frames=ps_rows,
                        point=-1,tags=[],container_rate=rate,bands=bands,container_frame_samples=rate//24000*1024,samples=rate//24000*6144)
            case['video'] = video_fixture([case],blob,channels=2,filename='aac-ssr-ps-cce-'+case['name']+'-synthetic.mp4')
            cases.append(case)
    for base in list(cases):
        if base['point']!=3:continue
        rows = []
        for i,seq in enumerate(SEQUENCES):
            shape = int(i%2==0)
            source = mono_payload(len(tables(10,27,0,False,0,0)[1])-1,2,True,i)
            raw = ps_packet(i,seq,shape,1-shape,3,base['active'],base['tags'],ps[i%3],source_payload=source)
            rows.append(dict(offset=len(blob),bytes=len(raw),payload=ps[i%3],source_payload=source.hex()));blob.extend(raw)
        case = dict(base,name=base['name']+'-source-sbr',frames=rows)
        case['video'] = video_fixture([case],blob,channels=2,filename='aac-ssr-ps-cce-'+case['name']+'-synthetic.mp4')
        cases.append(case)
    # Unlike the original matrix, these sources have independent sine/KBD
    # histories and a 2:1 spectral amplitude ratio. Their wire order alternates.
    from generate_aac_ssr_fixtures import oracle
    for active in (False,True):
        target_shapes = [int(i%2==0) for i in range(6)]
        source_shapes = {1:[1-s for s in target_shapes],15:target_shapes}
        source_refs = {}
        for tag in (1,15):
            pcm,_ = oracle(source_shapes[tag],1,active)
            if tag==15:pcm=b''.join(struct.pack('<f',v[0]*2) for v in struct.iter_unpack('<f',pcm))
            source_refs[str(tag)] = dict(pcm_offset=len(gold),pcm_bytes=len(pcm))
            gold.extend(pcm)
        mono = {tag:[v[0] for v in struct.iter_unpack('<f',gold[r['pcm_offset']:r['pcm_offset']+r['pcm_bytes']])]
                for tag,r in ((tag,source_refs[str(tag)]) for tag in (1,15))}
        combined = b''.join(struct.pack('<f',a+b) for a,b in zip(mono[1],mono[15]))
        combined_ref = dict(pcm_offset=len(gold),pcm_bytes=len(combined));gold.extend(combined)
        def distinct_packet(i,selected,payload,sbr_payloads):
            seq=SEQUENCES[i]
            target='0000000'+silent(seq,target_shapes[i],0,False,False)+fill(payload)
            sources=[]
            for tag in selected:
                raw=channel(i,seq,source_shapes[tag][i],0,active,False)
                if tag==15:raw=field(144,8)+raw[8:]
                sources.append('010'+field(tag,4)+'1'+'000'+'0'+'0000'+'0'+'0'+'10'+raw+fill(sbr_payloads.get(tag,b'')))
            if i%2:sources.reverse()
            return packed((''.join(sources)+target if i%2 else target+''.join(sources))+'111')
        for selected in ((1,),(15,),(1,15)):
            rows=[]
            for i in range(6):
                raw=distinct_packet(i,selected,b'',{})
                rows.append(dict(offset=len(blob),bytes=len(raw)));blob.extend(raw)
            ref=combined_ref if len(selected)==2 else source_refs[str(selected[0])]
            control=dict(name='distinct-'+str(int(active))+'-'+''.join(map(str,selected)),asc=config(1,3,selected).hex(),
                         frames=rows,slots=16,bands=32,container_rate=24000,container_frame_samples=1472,
                         durations=[SAMPLES[s] for s in SEQUENCES],samples=6144,**ref)
            control['video']=video_fixture([control],blob,filename='aac-ssr-ps-cce-'+control['name']+'-core-control-synthetic.mp4')
            controls.append(control)
        for mode in ('none','both','asymmetric'):
            rows=[]
            for i in range(6):
                source=mono_payload(len(tables(10,27,0,False,0,0)[1])-1,2,True,i)
                payloads={} if mode=='none' else ({1:source,15:source} if mode=='both' else {15:source})
                raw=distinct_packet(i,(1,15),bytes.fromhex(ps[i%3]),payloads)
                rows.append(dict(offset=len(blob),bytes=len(raw),payload=ps[i%3],
                                 source_payloads={str(tag):value.hex() for tag,value in payloads.items()}));blob.extend(raw)
            for rate,bands in ((24000,32),(48000,64)):
                prefix=field(29,5)+frequency(24000)+'0000'+frequency(rate)+field(3,5)+'000'
                case=dict(name=f'distinct-{int(active)}-{mode}-{rate}',asc=packed(program(prefix,1,3,(1,15))).hex(),
                          point=3,tags=[1,15],active=active,frames=rows,slots=16,bands=bands,container_rate=rate,
                          container_frame_samples=rate//24000*1024,samples=rate//24000*6144,source_pcm=source_refs,
                          distinct_sources=True,**combined_ref)
                case['video']=video_fixture([case],blob,channels=2,filename='aac-ssr-ps-cce-'+case['name']+'-synthetic.mp4')
                cases.append(case)
    # Reuse original scalar SSR alignment, with explicit absent-lane output
    # gains. Coded source ordinal pauses while target packet time continues.
    alignment = json.loads((DEST/'aac-ssr-alignment.json').read_text())
    for name,base_name,present in [('ahead','source-ahead',[1,1,0,0,0,0]),
                                 ('behind','opposite-switches',[1,1,0,0,0,0]),
                                 ('exact','source-ahead',[1,0,0,0,0,0]),
                                 ('resume','source-ahead',[1,1,0,1,1,0])]:
        base = next(c for c in alignment['cases'] if c['name']==base_name)
        source_pcm = (DEST/('aac-ssr-cce-absence-'+name+'-core.f32le')).read_bytes()
        ref=dict(pcm_offset=len(gold),pcm_bytes=len(source_pcm));gold.extend(source_pcm)
        gains=[];ordinal=0
        for i,on in enumerate(present):
            if on:
                gains += [1]*SAMPLES[base['source_sequences'][ordinal]];ordinal+=1
            else:gains += [0]*max(0,(i+1)*1024-len(gains))
        assert len(gains)==6144
        gain_ranges=[]
        for i in range(6):
            row=[];start=None
            for j,value in enumerate(gains[i*1024:(i+1)*1024]+[0]):
                if value and start is None:start=j
                if not value and start is not None:row.append([start,j]);start=None
            gain_ranges.append(row)
        for sbr in (False,True):
            rows=[];ordinal=0
            for i,on in enumerate(present):
                target='0000000'+silent(0,0,0,False,False)+fill(bytes.fromhex(ps[i%3]))
                coded='';payloads={}
                if on:
                    seq=base['source_sequences'][ordinal]
                    coded='0100001'+'1'+'000'+'0'+'0000'+'0'+'0'+'10'+channel(ordinal,seq,ordinal%2,0,True,False)
                    if sbr:
                        payload=mono_payload(len(tables(10,27,0,False,0,0)[1])-1,2,True,ordinal)
                        coded+=fill(payload);payloads['1']=payload.hex()
                    ordinal+=1
                raw=packed((coded+target if i%2 else target+coded)+'111')
                rows.append(dict(offset=len(blob),bytes=len(raw),payload=ps[i%3],source_payloads=payloads));blob.extend(raw)
            for rate,bands in ((24000,32),(48000,64)):
                prefix=field(29,5)+frequency(24000)+'0000'+frequency(rate)+field(3,5)+'000'
                case=dict(name=f'absence-{name}-{int(sbr)}-{rate}',asc=packed(program(prefix,1,3,(1,))).hex(),
                          point=3,tags=[1],active=True,frames=rows,slots=16,bands=bands,container_rate=rate,
                          container_frame_samples=rate//24000*1024,samples=rate//24000*6144,
                          source_pcm={'1':ref},distinct_sources=True,source_gain_ranges=gain_ranges,present=present,**ref)
                case['video']=video_fixture([case],blob,channels=2,filename='aac-ssr-ps-cce-'+case['name']+'-synthetic.mp4')
                cases.append(case)
    base = next(c for c in cases if c['point']==1 and c['active'] and c['tags']==[1,15] and c['bands']==64)
    for failure in ('shape','target'):
        rows = []
        for i,seq in enumerate(SEQUENCES):
            shape = int(i%2==0)
            raw = ps_packet(i,seq,shape,shape,1,True,(1,15),ps[i%3],failure if i==1 else None)
            rows.append(dict(offset=len(blob),bytes=len(raw),payload=ps[i%3]));blob.extend(raw)
        case = dict(base,name=failure,frames=rows,error='AAC SSR dependent coupling window shape mismatch' if failure=='shape' else 'AAC coupling target is absent')
        case['video'] = video_fixture([case],blob,channels=2,filename='aac-ssr-ps-cce-invalid-'+failure+'-synthetic.mp4')
        invalid.append(case)
    base = next(c for c in cases if c['point']==3 and c['active'] and c['tags']==[1,15] and c['bands']==64 and c['name'].endswith('source-sbr'))
    rows = []
    for i,seq in enumerate(SEQUENCES):
        shape = int(i%2==0)
        source = bytearray(mono_payload(len(tables(10,27,0,False,0,0)[1])-1,2,True,i))
        if i==1:source[0] ^= 1
        raw = ps_packet(i,seq,shape,1-shape,3,True,(1,15),ps[i%3],source_payload=bytes(source))
        rows.append(dict(offset=len(blob),bytes=len(raw),payload=ps[i%3]));blob.extend(raw)
    case = dict(base,name='crc',frames=rows,error='SBR CRC mismatch')
    case['video'] = video_fixture([case],blob,channels=2,filename='aac-ssr-ps-cce-invalid-crc-synthetic.mp4')
    invalid.append(case)
    (DEST/'aac-ssr-ps-cce-packets.bin').write_bytes(blob)
    (DEST/'aac-ssr-ps-cce-core.f32le').write_bytes(gold)
    (DEST/'aac-ssr-ps-cce.json').write_text(json.dumps(dict(controls=controls,cases=cases,invalid=invalid,
        acceptance='Enabled native SSR/PS coupling waveform for points 0/1/3, tags 1/15, gain and window transitions; stage composition uses independent scalar SSR core.',
        provenance='Own SSR gain/windows and tags 1/15, configured mono PCE and original PS payloads; no private media, foreign codec, FFmpeg or network.'),indent=2)+'\n')


if __name__=='__main__':main()
