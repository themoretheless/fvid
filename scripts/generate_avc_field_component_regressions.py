"""Authored PAFF PCM component depths and exact woven sample arrays."""
import json,hashlib
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer
from avc_fixture_mp4 import mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-field-components'
def configuration(y,c,weighted=False):
    b=Writer()
    for value in [244,0,10]:b.u(value,8)
    b.ue(0);b.ue(1);b.ue(y-8);b.ue(c-8);b.u(0);b.u(0)
    b.ue(0);b.ue(0);b.ue(0);b.ue(3);b.u(0);b.ue(1);b.ue(0);b.u(0);b.u(0);b.u(1);b.u(0);b.u(0)
    sps=b.nal(0x67)
    b=Writer();b.ue(0);b.ue(0);b.u(0);b.u(0);b.ue(0);b.ue(0);b.ue(0);b.u(int(weighted));b.u(0,2);b.se(0);b.se(0);b.se(0);b.u(1);b.u(0);b.u(0)
    pps=b.nal(0x68)
    return bytes([1,244,0,10,255,225])+len(sps).to_bytes(2,'big')+sps+b'\1'+len(pps).to_bytes(2,'big')+pps

def field(y,c,bottom,pair,idr,second,mode):
    b=Writer();b.ue(0);b.ue(2);b.ue(0);b.u(pair,4);b.u(1);b.u(int(bottom))
    if idr:b.ue(0)
    b.u(pair*4+int(second),4)
    if idr:b.u(0);b.u(0)
    else:b.u(0)
    b.se(0);b.ue(1 if mode=="pcm" else 0)
    if mode!="pcm":b.se(0);b.se(0)
    planes=[[0]*512,[0]*128,[0]*128]
    for address in range(2):
        if mode=="pcm":b.ue(25);b.align()
        else:b.ue(3);b.ue(0);b.se(0);b.u(1)
        for p,depth in enumerate([y,c,c]):
            side=16 if p==0 else 8;width=side*2;maximum=(1<<depth)-1
            for row in range(side):
                for col in range(side):
                    value=(pair*97+int(bottom)*53+p*43+row*73+(address*side+col)*31)&maximum
                    if row==0 and col<4:value=[0,maximum,1,maximum-1][col]
                    if mode=="pcm":b.u(value,depth)
                    else:value=1<<(depth-1)
                    planes[p][row*width+address*side+col]=value
    nal=b.nal(0x65 if idr else 0x41)
    return len(nal).to_bytes(4,'big')+nal,planes

def generate():
    files={};cases=[]
    for y in [8,10,12,14]:
        for c in [8,10,12,14]:
            for mode in ["pcm","dc-filtered"]:
                for reverse in [False,True]:
                    name=f'y{y}-c{c}-'+('bottom' if reverse else 'top')+'-'+mode;config=configuration(y,c);frames=[];oracles=[]
                    for pair in range(2):
                        fields={}
                        for bottom in ([True,False] if reverse else [False,True]):
                            index=len(frames);packet,planes=field(y,c,bottom,pair,index==0,index%2==1,mode);files[f'{name}-{index}.packet']=packet
                            frames.append((index,index==0,packet));fields[bottom]=planes
                        expected={}
                        for p,key in enumerate(['y','cb','cr']):
                            width=32 if p==0 else 16;rows=16 if p==0 else 8;values=[]
                            for row in range(rows):
                                for bottom in [False,True]:values+=fields[bottom][p][row*width:(row+1)*width]
                            filename=f'{name}-{pair}.{key}';files[filename]=b''.join(v.to_bytes(2,'little') for v in values);expected[key]=filename
                        oracles.append(expected)
                    files[name+'.mp4']=mux(config,frames,32,32,100);files[name+'.avcc']=config
                    damaged=None
                    if mode=="pcm" and (y,c) in [(8,14),(14,8)]:
                        truncated=frames[1][2][4:-16];packet=len(truncated).to_bytes(4,'big')+truncated
                        damaged=name+'-truncated.packet';files[damaged]=packet
                        files[name+'-truncated.mp4']=mux(config,[frames[0],(1,False,packet)],32,32,100)
                    cases.append({'file':name+'.mp4','configuration':name+'.avcc','packets':[f'{name}-{i}.packet' for i in range(4)],'luma_depth':y,'chroma_depth':c,'bottom_first':reverse,'mode':mode,'truncated_packet':damaged,'frames':oracles})
    files['manifest.json']=(json.dumps({'provenance':'64 authored40ms PAFF PCM/filtered-DC clips; independent woven component arrays. No private media, encoder or network.','cases':cases,'sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
