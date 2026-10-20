#!/usr/bin/env python3
"""Short authored MBAFF I_PCM size series for allocation qualification."""
import json,hashlib
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer
from generate_avc_component_depths import configuration as base_configuration
from avc_fixture_mp4 import mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-mbaff-pcm-memory'
def config(width,height):
    b=Writer()
    for v in [244,0,40]:b.u(v,8)
    b.ue(0);b.ue(1);b.ue(0);b.ue(2);b.u(0);b.u(0)
    b.ue(0);b.ue(0);b.ue(0);b.ue(1);b.u(0);b.ue(width//16-1);b.ue(height//32-1)
    b.u(0);b.u(1);b.u(1);b.u(0);b.u(0);sps=b.nal(0x67)
    old=base_configuration(8,10);return bytes([1,244,0,40,255,225])+len(sps).to_bytes(2,'big')+sps+old[8+int.from_bytes(old[6:8],'big'):]
def picture(width,height,field,index):
    b=Writer();b.ue(0);b.ue(2);b.ue(0);b.u(index,4);b.u(0)
    if index==0:b.ue(0)
    b.u(index*2,4)
    if index==0:b.u(0);b.u(0)
    else:b.u(0)
    b.se(0);b.ue(1)
    planes=[[0]*(width*height),[0]*(width*height//4),[0]*(width*height//4)]
    for address in range(width*height//256):
        if address%2==0:b.u(int(field))
        b.ue(25);b.align()
        for p,depth in enumerate([8,10,10]):
            side=16 if p==0 else 8;stride=width if p==0 else width//2
            x=(address//2%(width//16))*side;base=(address//2//(width//16))*side*2
            for row in range(side):
                dest_row=base+row*2+address%2 if field else base+row+(address%2)*side
                for col in range(side):
                    value=(dest_row*73+(x+col)*31+p*43+index*17)&((1<<depth)-1);b.u(value,depth);planes[p][dest_row*stride+x+col]=value
    nal=b.nal(0x65 if index==0 else 0x41);return len(nal).to_bytes(4,'big')+nal,planes
def generate():
    files={};cases=[]
    for side in [64,128,256]:
        for field in [False,True]:
            name=f'{side}-'+('field' if field else 'frame');configuration=config(side,side);files[name+'.avcc']=configuration;frames=[];records=[]
            for index in range(2):
                packet,planes=picture(side,side,field,index);file=f'{name}-{index}.packet';files[file]=packet;record={'packet':file}
                for p,values in zip(['y','cb','cr'],planes):
                    file=f'{name}-{index}.{p}';files[file]=b''.join(v.to_bytes(2,'little') for v in values);record[p]=file
                frames.append((index,index==0,packet));records.append(record)
            files[name+'.mp4']=mux(configuration,frames,side,side,50);cases.append({'file':name+'.mp4','configuration':name+'.avcc','size':side,'field':field,'frames':records})
    return files,cases
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True);files,cases=generate()
    for name,data in files.items():(OUT/name).write_bytes(data)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'Six40ms authored 64/128/256-square MBAFF I_PCM clips,8-bit luma/10-bit chroma, frame/field pairs. Exact independently placed arrays. No encoder/private media/network.','cases':cases,'files':{n:hashlib.sha256(d).hexdigest() for n,d in files.items()}},indent=2)+'\n')
