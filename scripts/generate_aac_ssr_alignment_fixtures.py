#!/usr/bin/env python3
"""Original independent SSR window drift cases with scalar PCM; offline only."""
import json,hashlib
from generate_aac_ssr_fixtures import DEST,field,frequency,packed,video_fixture,SAMPLES,oracle,channel
from generate_aac_ssr_coupling_fixtures import packet,config

def main():
    cases=[];blob=bytearray();gold=bytearray()
    for name,target,source in [
        ('source-ahead',[0]*6,[0,1,2,3,0,0]),
        ('source-behind',[0,1,2,3,0,0],[0]*6),
        ('opposite-switches',[0,1,2,3,0,0],[2,3,0,1,2,2]),
    ]:
        shapes=[i%2 for i in range(6)]
        pcm,source_frames=oracle(shapes,1,True,sequences=source)
        off=len(gold);gold.extend(pcm);frames=[]
        for i,(t,s) in enumerate(zip(target,source)):
            data=packet(i,t,0,shapes[i],1,3,True,source_sequence=s)
            row=dict(offset=len(blob),bytes=len(data),samples=SAMPLES[t]);blob.extend(data);frames.append(row)
            mono=packed('0000000'+channel(i,s,shapes[i],0,True,False)+'111')
            source_frames[i].update(offset=len(blob),bytes=len(mono));blob.extend(mono)
        asc=config(1,3)
        case=dict(name=name,slots=16,bands=32,asc=asc.hex(),channels=1,frames=frames,source_frames=source_frames,source_asc=packed(field(3,5)+frequency(24000)+'0001'+'000').hex(),pcm_offset=off,pcm_bytes=len(pcm),samples=6144,container_rate=24000,container_frame_samples=1472,durations=[SAMPLES[s] for s in target],target_sequences=target,source_sequences=source,error='AAC SSR independent coupling window extents require alignment')
        case['video']=video_fixture([case],blob,channels=1,filename=f'aac-ssr-alignment-{name}-synthetic.mp4')
        source_case=dict(case,asc=case['source_asc'],frames=source_frames,durations=[SAMPLES[s] for s in source])
        case['source_video']=video_fixture([source_case],blob,channels=1,filename=f'aac-ssr-alignment-{name}-source-synthetic.mp4')
        cases.append(case)
    (DEST/'aac-ssr-alignment-packets.bin').write_bytes(blob);(DEST/'aac-ssr-alignment-pcm.f32le').write_bytes(gold)
    (DEST/'aac-ssr-alignment.json').write_text(json.dumps(dict(cases=cases,packet_sha256=hashlib.sha256(blob).hexdigest(),pcm_sha256=hashlib.sha256(gold).hexdigest(),qualification='native standalone source plus alignment primitive acceptance; coupled native playback still refuses'),indent=2)+'\n')
    print(len(cases),'independent SSR alignment scenarios; 6 original videos')
if __name__=='__main__':main()
