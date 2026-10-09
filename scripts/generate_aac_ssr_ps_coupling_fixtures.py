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
