#!/usr/bin/env python3
"""Authored mixed I/P MBAFF slices with independent component arrays."""
import hashlib,json
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer
from avc_fixture_mp4 import mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-mbaff-mixed-component-slices'
def config(y,c):
    b=Writer()
    for v in [244,0,30]:b.u(v,8)
    b.ue(0);b.ue(1);b.ue(y-8);b.ue(c-8);b.u(0);b.u(0);b.ue(0);b.ue(0);b.ue(0);b.ue(2);b.u(0);b.ue(1);b.ue(0);b.u(0);b.u(1);b.u(1);b.u(0);b.u(0)
    sps=b.nal(0x67);b=Writer();b.ue(0);b.ue(0);b.u(0);b.u(0);b.ue(0);b.ue(0);b.ue(0);b.u(0);b.u(0,2);b.se(0);b.se(0);b.se(0);b.u(1);b.u(0);b.u(0);pps=b.nal(0x68)
    return bytes([1,244,0,30,255,225])+len(sps).to_bytes(2,'big')+sps+b'\1'+len(pps).to_bytes(2,'big')+pps

def header(kind,first,frame):
    b=Writer();b.ue(first);b.ue(2 if kind=='I' else 0);b.ue(0);b.u(frame,4);b.u(0)
    if frame==0:b.ue(0)
    b.u(frame*2,4)
    if kind=='P':b.u(0);b.u(0)
    if frame==0:b.u(0);b.u(0)
    else:b.u(0)
    b.se(0);b.ue(1);return b

def generate():
    files={};cases=[]
    for y,c in [(8,8),(8,10),(10,8),(8,14),(14,8)]:
        for field in [False,True]:
            name=f'y{y}-c{c}-'+('field' if field else 'frame');configuration=config(y,c);files[name+'.avcc']=configuration
            planes=[[0]*1024,[0]*256,[0]*256];b=header('I',0,0)
            for address in range(4):
                if address%2==0:b.u(int(field))
                b.ue(25);b.align()
                for p,depth in enumerate([y,c,c]):
                    side=16 if p==0 else 8;stride=side*2;x=address//2*side
                    for row in range(side):
                        dest=row*2+address%2 if field else row+address%2*side
                        for col in range(side):
                            maximum=(1<<depth)-1;value=(row*73+(x+col)*31+p*43+address*19)&maximum
                            if row==0 and col<4:value=[0,maximum,1,maximum-1][col]
                            b.u(value,depth);planes[p][dest*stride+x+col]=value
            nal=b.nal(0x65);first=len(nal).to_bytes(4,'big')+nal
            left=header('I',0,1)
            for address in range(2):
                if address==0:left.u(int(field))
                left.ue(3);left.ue(0);left.se(0);left.u(1)
            right=header('P',1,1)
            for address in range(2):
                right.ue(0)
                if address==0:right.u(int(field))
                right.ue(0)
                if field:right.u(1)
                right.se(0);right.se(0);right.ue(0)
            left,right=left.nal(0x41),right.nal(0x41)
            second=len(left).to_bytes(4,'big')+left+len(right).to_bytes(4,'big')+right
            reconstructed=[p.copy() for p in planes]
            for p,depth in enumerate([y,c,c]):
                side=16 if p==0 else 8;stride=side*2
                for row in range(side*2):reconstructed[p][row*stride:row*stride+side]=[1<<(depth-1)]*side
            records=[]
            for index,(packet,values) in enumerate([(first,planes),(second,reconstructed)]):
                file=f'{name}-{index}.packet';files[file]=packet;record={'packet':file}
                for component,data in zip(['y','cb','cr'],values):
                    file=f'{name}-{index}.{component}';files[file]=b''.join(v.to_bytes(2,'little') for v in data);record[component]=file
                records.append(record)
            files[name+'.mp4']=mux(configuration,[(0,True,first),(1,False,second)],32,32,50)
            cases.append({'file':name+'.mp4','configuration':name+'.avcc','luma_depth':y,'chroma_depth':c,'field':field,'frames':records})
    return files,cases
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True);files,cases=generate()
    for name,data in files.items():(OUT/name).write_bytes(data)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'Ten authored40ms32x32 MBAFF videos. IDR PCM reference; next AU uses a left intra DC slice and right zero-motion P slice. Independent placed samples and midpoint DC oracle. No external codec/private media/network.','cases':cases,'files':{n:hashlib.sha256(d).hexdigest() for n,d in files.items()}},indent=2)+'\n')
