#!/usr/bin/env python3
"""Authored MBAFF intra PCM/DC clips with independent component precision."""
import hashlib,json
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer,CabacWriter
from generate_avc_component_depths import configuration as progressive_configuration
from avc_fixture_mp4 import mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-mbaff-component-depths'
def configuration(y,c,cabac=False):
    b=Writer()
    for v in [244,0,10]:b.u(v,8)
    b.ue(0);b.ue(1);b.ue(y-8);b.ue(c-8);b.u(0);b.u(0)
    b.ue(0);b.ue(0);b.ue(0);b.ue(1);b.u(0);b.ue(0);b.ue(0)
    b.u(0);b.u(1);b.u(1);b.u(0);b.u(0)
    sps=b.nal(0x67);old=progressive_configuration(y,c)
    tail=old[8+int.from_bytes(old[6:8],'big'):]
    if cabac:
        p=Writer();p.ue(0);p.ue(0);p.u(1);p.u(0);p.ue(0);p.ue(0);p.ue(0);p.u(0);p.u(0,2)
        p.se(0);p.se(0);p.se(0);p.u(1);p.u(0);p.u(0);pps=p.nal(0x68)
        tail=b'\1'+len(pps).to_bytes(2,'big')+pps
    return old[:6]+len(sps).to_bytes(2,'big')+sps+tail

def picture(y,c,field,index,mode):
    b=Writer();b.ue(0);b.ue(2);b.ue(0);b.u(index,4);b.u(0)
    if index==0:b.ue(0)
    b.u(index*2,4)
    if index==0:b.u(0);b.u(0)
    else:b.u(0)
    b.se(0);b.ue(0 if mode.endswith("filtered") else 1)
    if mode.endswith("filtered"):b.se(0);b.se(0)
    cabac=mode=='pcm-cabac'
    if cabac:
        while len(b.bits)%8:b.u(1)
        entropy=CabacWriter(-1,26)
    planes=[[0]*512,[0]*128,[0]*128]
    for address in range(2):
        if cabac:
            if address==0:entropy.decision(70,int(field))
            entropy.decision(3+int(address==1 and not field),1)
            b.bits.extend(entropy.finish());b.align()
        elif address==0:b.u(int(field))
        is_dc=mode.startswith('dc') or (mode=='mixed-pcm-dc' and address==1)
        if is_dc:
            b.ue(3);b.ue(0);b.se(0)
            if mode=='mixed-pcm-dc' and not field:b.u(3,6)
            else:b.u(1)
        elif not cabac:
            b.ue(25);b.align()
        for p,depth in enumerate([y,c,c]):
            side=16 if p==0 else 8;width=side
            for row in range(side):
                dest_row=row*2+address if field else row+address*side
                for col in range(side):
                    if is_dc:
                        value=1<<(depth-1)
                        if mode=='mixed-pcm-dc' and not field:
                            start=(side-1)*width+(0 if p==0 else col//4*4)
                            top=planes[p][start:start+(16 if p==0 else 4)]
                            value=(sum(top)+(8 if p==0 else 2))>>(4 if p==0 else 2)
                    else:
                        maximum=(1<<depth)-1;value=(row*73+col*31+p*43+address*19+index*17)&maximum
                        if row==0 and col<4:value=[0,maximum,1,maximum-1][col]
                        b.u(value,depth)
                    planes[p][dest_row*width+col]=value
        if cabac:entropy.low=0;entropy.range=510;entropy.width=9
    if cabac:b.bits.extend(entropy.finish())
    nal=b.nal(0x65 if index==0 else 0x41,trailing=not cabac)
    return len(nal).to_bytes(4,'big')+nal,planes

def generate():
    files={};cases=[]
    for y,c in [(y,c) for y in [8,10,12,14] for c in [8,10,12,14]]:
        for field in [False,True]:
            for mode in ['pcm','dc','dc-filtered','pcm-cabac','mixed-pcm-dc']:
                name=f'y{y}-c{c}-'+('field' if field else 'frame')+'-'+mode
                config=configuration(y,c,mode=='pcm-cabac');files[name+'.avcc']=config;frames=[];records=[]
                for index in range(2):
                    packet,planes=picture(y,c,field,index,mode);file=f'{name}-{index}.packet';files[file]=packet;record={'packet':file}
                    for p,values in zip(['y','cb','cr'],planes):
                        file=f'{name}-{index}.{p}';files[file]=b''.join(v.to_bytes(2,'little') for v in values);record[p]=file
                    records.append(record);frames.append((index,index==0,packet))
                files[name+'.mp4']=mux(config,frames,16,32,50)
                cases.append({'file':name+'.mp4','configuration':name+'.avcc','luma_depth':y,'chroma_depth':c,'field':field,'mode':mode,'frames':records})
    for case in cases:
        if case['mode']!='pcm' or (case['luma_depth'],case['chroma_depth']) not in [(8,14),(14,8)]:continue
        b=Writer();b.ue(0);b.ue(0);b.ue(0);b.u(1,4);b.u(0);b.u(2,4)
        b.u(0);b.u(0);b.u(0);b.se(0);b.ue(1);b.ue(2)
        nal=b.nal(0x41);packet=len(nal).to_bytes(4,'big')+nal
        name=case['file'][:-4]+'-inter-unsupported'
        files[name+'.packet']=packet
        files[name+'.mp4']=mux(files[case['configuration']],[(0,True,files[case['frames'][0]['packet']]),(1,False,packet)],16,32,50)
    return files,cases
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True);files,cases=generate()
    for name,data in files.items():(OUT/name).write_bytes(data)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'160 authored16x32 two-frame40ms high444-profile MBAFF intra PCM/DC clips, frame/field pairs. Independent per-component arrays. No private media, encoder or network.','cases':cases,'files':{n:hashlib.sha256(d).hexdigest() for n,d in files.items()}},indent=2)+'\n')
