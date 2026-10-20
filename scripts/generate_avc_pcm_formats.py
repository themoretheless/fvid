"""Owned progressive AVC PCM colour formats and per-plane sample oracles."""
import json,struct
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer
from avc_fixture_mp4 import mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-pcm-formats'

def configuration(fmt,ydepth,cdepth,cropped,cabac=False,weighted=False,height_mbs=1,max_refs=1,bipred=0,separate=False,level=51):
    b=Writer()
    for value in [244,0,level]:b.u(value,8)
    b.ue(0);b.ue(fmt)
    if fmt==3:b.u(int(separate))
    b.ue(ydepth-8);b.ue(cdepth-8);b.u(0);b.u(0)
    b.ue(0);b.ue(0);b.ue(0);b.ue(max_refs);b.u(0);b.ue(1);b.ue(height_mbs-1)
    b.u(1);b.u(1);b.u(int(cropped))
    if cropped:
        for _ in range(4):b.ue(1)
    b.u(0);sps=b.nal(0x67)
    b=Writer();b.ue(0);b.ue(0);b.u(int(cabac));b.u(0);b.ue(0);b.ue(0);b.ue(0);b.u(int(weighted));b.u(bipred,2)
    b.se(0);b.se(0);b.se(0);b.u(1);b.u(0);b.u(0);pps=b.nal(0x68)
    return bytes([1,244,0,level,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([1])+len(pps).to_bytes(2,'big')+pps

def generate():
    OUT.mkdir(exist_ok=True);cases=[];golden=bytearray()
    for fmt in [0,2,3]:
        sx,sy=(2,1)if fmt==2 else(1,1)
        for yd,cd in [(8,8),(10,12),(14,8)]:
            for cropped in [False,True]:
                name=f'avc-pcm-format{fmt}-y{yd}-c{cd}-{int(cropped)}'
                config=configuration(fmt,yd,cd,cropped);(OUT/(name+'.avcc')).write_bytes(config)
                frames=[];records=[]
                for i in range(2):
                    planes=[]
                    for p,depth in enumerate([yd,cd,cd]):
                        w,h=(32,16)if p==0 else(0,0)if fmt==0 else(32//sx,16//sy)
                        maximum=(1<<depth)-1
                        values=[(x*31+y*7+p*43+i*17)&maximum for y in range(h)for x in range(w)]
                        if values:values[:4]=[0,maximum,1,maximum-1]
                        planes.append(values)
                    b=Writer();b.ue(0);b.ue(2);b.ue(0);b.u(i,4)
                    if i==0:b.ue(0)
                    b.u(i*2,4)
                    if i==0:b.u(0);b.u(0)
                    else:b.u(0)
                    b.se(0);b.ue(1)
                    for mb in range(2):
                        b.ue(25);b.align()
                        for p,depth in enumerate([yd,cd,cd]):
                            if p and fmt==0:continue
                            width,height=(16,16)if p==0 else(16//sx,16//sy)
                            stride=32 if p==0 else 32//sx
                            for y in range(height):
                                for x in range(width):b.u(planes[p][y*stride+mb*width+x],depth)
                    nal=b.nal(0x65 if i==0 else 0x41);packet=len(nal).to_bytes(4,'big')+nal
                    (OUT/f'{name}-{i}.packet').write_bytes(packet);frames.append((i,i==0,packet))
                    at=len(golden)//2;all_samples=sum(planes,[]);golden.extend(struct.pack('<'+str(len(all_samples))+'H',*all_samples));records.append(dict(packet=f'{name}-{i}.packet',offset=at))
                (OUT/(name+'.mp4')).write_bytes(mux(config,frames,32,16,25))
                cases.append(dict(file=name+'.mp4',configuration=name+'.avcc',format=fmt,luma_depth=yd,chroma_depth=cd,crop=[sx,sx,sy,sy]if cropped else[0,0,0,0],frames=records))
    (OUT/'samples.y16').write_bytes(golden)
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance='Owned High 4:4:4 Predictive profile SPS/PPS/CAVLC PCM, no separate colour planes. Monochrome/422/444 per-plane integer samples, offline, no external codec or private media.',cases=cases),indent=2)+'\n')
if __name__=='__main__':generate()
