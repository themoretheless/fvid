"""Owned complementary CAVLC PCM fields in monochrome/422/444 geometry."""
import json,struct
from pathlib import Path
from generate_avc_pcm_formats import Writer,mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-component-fields'
def configuration(fmt,yd,cd,crop,cabac=False,height_mbs=1,weighted=False,bipred=0,mbaff=False,bottom_poc=False):
    b=Writer()
    for v in [244,0,51]:b.u(v,8)
    b.ue(0);b.ue(fmt)
    if fmt==3:b.u(0)
    b.ue(yd-8);b.ue(cd-8);b.u(0);b.u(0);b.ue(0);b.ue(0);b.ue(0);b.ue(2);b.u(0);b.ue(1);b.ue(height_mbs-1)
    b.u(0);b.u(int(mbaff));b.u(1);b.u(int(crop))
    if crop:
        for _ in range(4):b.ue(1)
    b.u(0);sps=b.nal(0x67)
    b=Writer();b.ue(0);b.ue(0);b.u(int(cabac));b.u(int(bottom_poc));b.ue(0);b.ue(0);b.ue(0);b.u(int(weighted));b.u(bipred,2);b.se(0);b.se(0);b.se(0);b.u(1);b.u(0);b.u(0);pps=b.nal(0x68)
    return bytes([1,244,0,51,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([1])+len(pps).to_bytes(2,'big')+pps
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for fmt in [0,2,3]:
        sx=2 if fmt==2 else 1
        for yd,cd in [(8,8),(10,12),(14,8)]:
            planes=[[(x*179+y*911+p*43+x*y*13)&((1<<depth)-1) for y in range(32) for x in range(w)] if w else [] for p,(w,depth) in enumerate([(32,yd),(0 if fmt==0 else 32//sx,cd),(0 if fmt==0 else 32//sx,cd)])]
            for reverse in [False,True]:
                for crop in [False,True]:
                    name=f'field-f{fmt}-y{yd}-c{cd}-r{int(reverse)}-crop{int(crop)}';config=configuration(fmt,yd,cd,crop);packets=[]
                    for index,bottom in enumerate([True,False] if reverse else [False,True]):
                        b=Writer();b.ue(0);b.ue(2);b.ue(0);b.u(0,4);b.u(1);b.u(int(bottom))
                        if index==0:b.ue(0)
                        b.u(index,4)
                        if index==0:b.u(0);b.u(0)
                        else:b.u(0)
                        b.se(0);b.ue(1)
                        for mb in range(2):
                            b.ue(25);b.align()
                            for p,(w,bw,depth) in enumerate([(32,16,yd),(32//sx,16//sx,cd),(32//sx,16//sx,cd)]):
                                if fmt==0 and p:continue
                                for y in range(16):
                                    for x in range(bw):b.u(planes[p][(2*y+int(bottom))*w+mb*bw+x],depth)
                        nal=b.nal(0x65 if index==0 else 0x41);packets.append(len(nal).to_bytes(4,'big')+nal)
                    for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
                    raw=sum(planes,[])
                    for ext,data in [('avcc',config),('mp4',mux(config,[(i,i==0,p) for i,p in enumerate(packets)],32,32,50)),('y16',struct.pack('<'+str(len(raw))+'H',*raw))]:(OUT/f'{name}.{ext}').write_bytes(data)
                    cases.append(dict(name=name,format=fmt,luma_depth=yd,chroma_depth=cd,bottom_first=reverse,crop=[sx,sx,2,2] if crop else [0,0,0,0]))
    assert len(cases)==36
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
