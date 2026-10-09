#!/usr/bin/env python3
"""Original AAC gain-control syntax videos. No network or external media tools."""
import json,hashlib
from generate_aac_ps_coupling_fixtures import DEST,field,config,packet,video_fixture,core,ps,sbr

def main():
    blob=bytearray();pcm=bytearray();cases=[];invalid=[]
    for slots in [15,16]:
        gold=core(slots*64,False,1)
        offset=len(pcm);pcm.extend(gold)
        for kind in ['LC','PS']:
            rate=48000 if kind=='PS' else 24000
            base=[]
            for i in range(6):
                raw=sbr(ps(1,1,i,slots=slots*2,count=1)[0],i) if kind=='PS' else b''
                data=packet(i,[15],1,raw,{15:b''})
                base.append(dict(offset=len(blob),bytes=len(data)));blob.extend(data)
            c=dict(slots=slots,bands=64 if kind=='PS' else 32,kind=kind,asc=config(slots,rate,kind,[15],1).hex(),
                   output_rate=rate,container_rate=rate,container_frame_samples=slots*64*(2 if kind=='PS' else 1),channels=2 if kind=='PS' else 1,samples=slots*64*6*(2 if kind=='PS' else 1),pcm_offset=0,
                   core_offset=offset,frames=base)
            baseline=video_fixture([c],blob,channels=c['channels'],filename=f'aac-gain-{slots*64}-{kind.lower()}-baseline-synthetic.mp4')
            for location in ['target','source']:
                for bands in range(4):
                    frames=[]
                    for i in range(6):
                        raw=sbr(ps(1,1,i,slots=slots*2,count=1)[0],i) if kind=='PS' else b''
                        gain=field(bands,2)+'000'*bands
                        data=packet(i,[15],1,raw,{15:b''},**{location+'_gain':gain})
                        frames.append(dict(offset=len(blob),bytes=len(data)));blob.extend(data)
                    row=dict(c,frames=frames,location=location,bands=bands,baseline=baseline)
                    row['video']=video_fixture([row],blob,channels=c['channels'],filename=f'aac-gain-{slots*64}-{kind.lower()}-{location}-{bands}-synthetic.mp4')
                    cases.append(row)
            if kind=='LC':
                frames=[]
                for i in range(3):
                    gain=field(1,2)+field(1,3)+field(8,4)+field(0,5) if i==1 else None
                    data=packet(i,[15],1,b'',{15:b''},target_gain=gain)
                    frames.append(dict(offset=len(blob),bytes=len(data)));blob.extend(data)
                row=dict(c,frames=frames,samples=slots*64*3,error='owned AAC active gain control synthesis is not implemented')
                row['video']=video_fixture([row],blob,channels=1,filename=f'aac-gain-{slots*64}-active-synthetic.mp4');invalid.append(row)
    (DEST/'aac-gain-control-packets.bin').write_bytes(blob)
    (DEST/'aac-gain-control-core.f32le').write_bytes(pcm)
    (DEST/'aac-gain-control.json').write_text(json.dumps(dict(cases=cases,invalid=invalid,sha256=hashlib.sha256(blob).hexdigest()),indent=2)+'\n')
    print(len(cases),'empty gain acceptance cases;',len(invalid),'active gain refusals')
if __name__=='__main__':main()
