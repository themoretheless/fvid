#!/usr/bin/env python3
"""Authored 16x16 two-frame AVC I_PCM component-depth acceptance targets."""
import json,hashlib
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer
from avc_fixture_mp4 import mux
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
def configuration(y,c,weighted=False,bipred=0,max_refs=1,transform8=False):
    b=Writer()
    for v in [244,0,10]:b.u(v,8)
    b.ue(0);b.ue(1);b.ue(y-8);b.ue(c-8);b.u(0);b.u(0)
    b.ue(0);b.ue(0);b.ue(0);b.ue(max_refs);b.u(0);b.ue(0);b.ue(0)
    b.u(1);b.u(1);b.u(0);b.u(0)
    sps=b.nal(0x67)
    b=Writer();b.ue(0);b.ue(0);b.u(0);b.u(0);b.ue(0);b.ue(0);b.ue(0);b.u(int(weighted));b.u(bipred,2)
    b.se(0);b.se(0);b.se(0);b.u(1);b.u(0);b.u(0)
    if transform8:b.u(1);b.u(0);b.se(0)
    pps=b.nal(0x68)
    return bytes([1,244,0,10,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([1])+len(pps).to_bytes(2,'big')+pps

def main():
    cases=[]
    for y in range(8,15):
        for c in range(8,15):
            name=f'avc-component-depth-y{y}-c{c}';config=configuration(y,c);frames=[];records=[]
            for i in range(2):
                planes=[]
                for p,(count,depth) in enumerate([(256,y),(64,c),(64,c)]):
                    maximum=(1<<depth)-1
                    values=[(n*31+p*43+i*17)&maximum for n in range(count)]
                    values[:4]=[0,maximum,1,maximum-1];planes.append(values)
                b=Writer();b.ue(0);b.ue(2);b.ue(0);b.u(i,4)
                if i==0:b.ue(0)
                b.u(i*2,4)
                if i==0:b.u(0);b.u(0)
                else:b.u(0)
                b.se(0);b.ue(1);b.ue(25);b.align()
                for values,depth in zip(planes,[y,c,c]):
                    for v in values:b.u(v,depth)
                nal=b.nal(0x65 if i==0 else 0x41);packet=len(nal).to_bytes(4,'big')+nal
                f={}
                for ext,data in [('packet',packet),('y',b''.join(v.to_bytes(2,'little') for v in planes[0])),('cb',b''.join(v.to_bytes(2,'little') for v in planes[1])),('cr',b''.join(v.to_bytes(2,'little') for v in planes[2]))]:
                    file=f'{name}-{i}.{ext}';(D/file).write_bytes(data);f[ext]=file
                records.append(f);frames.append((i,i==0,packet))
            file=name+'.mp4';data=mux(config,frames,16,16,50);(D/file).write_bytes(data)
            configuration_file=name+'.avcc';(D/configuration_file).write_bytes(config)
            cases.append(dict(file=file,configuration=configuration_file,luma_depth=y,chroma_depth=c,frames=records,sha256=hashlib.sha256(data).hexdigest()))
    (D/'avc-component-depths.json').write_text(json.dumps(dict(cases=cases,provenance='Authored high-444 profile, progressive 4:2:0 I_PCM, no source media, external codec or network. Independent per-component integer samples.'),indent=2)+'\n')
    motion=[]
    for case in cases:
        first=case['frames'][0];name=case['file'][:-4]+'-pskip'
        b=Writer();b.ue(0);b.ue(0);b.ue(0);b.u(1,4);b.u(2,4)
        b.u(0);b.u(0);b.u(0);b.se(0);b.ue(1);b.ue(1)
        nal=b.nal(0x41);packet=len(nal).to_bytes(4,'big')+nal
        file=name+'-1.packet';(D/file).write_bytes(packet)
        second=dict(first,packet=file)
        data=mux((D/case['configuration']).read_bytes(),[(0,True,(D/first['packet']).read_bytes()),(1,False,packet)],16,16,50)
        file=name+'.mp4';(D/file).write_bytes(data)
        motion.append(dict(case,file=file,frames=[first,second],sha256=hashlib.sha256(data).hexdigest()))
    (D/'avc-component-motion.json').write_text(json.dumps(dict(cases=motion,provenance='Original authored IDR I_PCM followed by P-skip with exact unchanged source-depth sample oracles. No external codec, source media or network.'),indent=2)+'\n')

if __name__=='__main__':main()
