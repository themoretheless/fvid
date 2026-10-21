"""Owned shared 4:4:4 P motion/weights/residual/mixed-intra; scalar six-tap oracle."""
import itertools,json,struct
from pathlib import Path
from generate_avc_pcm_formats import Writer,configuration as base_configuration,mux
from generate_avc_monochrome_fractional import prediction
from generate_avc_444_deblock import coefficient
from generate_avc_separate_8x8_deblock import residual_8x8_single
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-inter'
WEIGHTS=[(3,-5),(1,2),(-1,1)]
def configuration(yd,cd,weighted,max_refs=1,bipred=0,offsets=(0,0),cabac=False):
    cfg=base_configuration(3,yd,cd,True,cabac=cabac,weighted=weighted,max_refs=max_refs,bipred=bipred,level=41)
    n=int.from_bytes(cfg[6:8],'big')
    b=Writer();b.ue(0);b.ue(0);b.u(int(cabac));b.u(0);b.ue(0);b.ue(0);b.ue(0);b.u(int(weighted));b.u(bipred,2)
    b.se(0);b.se(0);b.se(offsets[0]);b.u(1);b.u(0);b.u(0);b.u(1);b.u(0);b.se(offsets[1])
    pps=b.nal(0x68);return cfg[:8+n]+bytes([1])+len(pps).to_bytes(2,'big')+pps

def header(index,weighted):
    b=Writer();b.ue(0);b.ue(2 if index==0 else 0);b.ue(0);b.u(index,4)
    if index==0:b.ue(0)
    b.u(index*2,4)
    if index==0:b.u(0);b.u(0)
    else:
        b.u(0);b.u(0)
        if weighted:
            b.ue(1 if index==1 else 0);b.ue(1 if index==1 else 0)
            b.u(int(index==1))
            if index==1:b.se(WEIGHTS[0][0]);b.se(WEIGHTS[0][1])
            b.u(int(index==1))
            if index==1:
                for weight,offset in WEIGHTS[1:]:b.se(weight);b.se(offset)
        b.u(0)
    b.se(0);b.ue(1);return b

def make_case(yd,cd,vector,kind,weighted):
    depths=[yd,cd,cd]
    source=[[(x*1703+y*7907+x*y*113+p*349)&((1<<depth)-1)for y in range(16)for x in range(32)]for p,depth in enumerate(depths)]
    b=header(0,weighted)
    for mb in range(2):
        b.ue(25);b.align()
        for p,depth in enumerate(depths):
            for y in range(16):
                for x in range(16):b.u(source[p][y*32+mb*16+x],depth)
    nal=b.nal(0x65);packets=[len(nal).to_bytes(4,'big')+nal]
    actual_vector=[0,0]if kind=='skip'else vector
    predicted=[prediction(plane,depth,actual_vector)for plane,depth in zip(source,depths)]
    if weighted:predicted=[[max(0,min((1<<depth)-1,((WEIGHTS[p][0]*v+1)//2)+(WEIGHTS[p][1]<<(depth-8))))for v in plane]for p,(plane,depth)in enumerate(zip(predicted,depths))]
    target=[list(p)for p in predicted];b=header(1,weighted)
    if kind=='skip':b.ue(2)
    else:
        for mb in range(2):
            b.ue(0)
            if kind=='mixed' and mb==1:
                b.ue(8);b.se(0)
                for p in range(3):
                    b.u(1)
                    dc=(sum(target[p][y*32+15]for y in range(16))+8)//16
                    for y in range(16):target[p][y*32+16:y*32+32]=[dc]*16
                continue
            b.ue(0);b.se(vector[0]if mb==0 else 0);b.se(vector[1]if mb==0 else 0)
            residual=kind in ['res4','res8','mixed'];eight=kind=='res8';b.ue(9 if residual else 0)
            if residual:
                b.u(int(eight));b.se(0)
                for p,depth in enumerate(depths):
                    sign=-1 if (p+mb)%2 else 1
                    if eight:
                        for sub in range(16):
                            if sub%4==0:coefficient(b,4*sign,0)
                            else:b.u(1)
                        delta=residual_8x8_single(0,4*sign,26,depth,16)[0]
                    else:
                        for _ in range(16):coefficient(b,4*sign,0)
                        q=26+6*(depth-8);v=4*sign*16*[10,11,13,14,16,18][q%6]
                        scaled=v<<(q//6-4)if q//6>=4 else(v+(1<<(3-q//6)))>>(4-q//6)
                        delta=(scaled+32)>>6
                    for y in range(16):
                        for x in range(16):
                            at=y*32+mb*16+x;target[p][at]=max(0,min((1<<depth)-1,target[p][at]+delta))
    nal=b.nal(0x41);packets.append(len(nal).to_bytes(4,'big')+nal)
    b=header(2,weighted);b.ue(2);nal=b.nal(0x41);packets.append(len(nal).to_bytes(4,'big')+nal)
    return configuration(yd,cd,weighted),packets,sum(source+target+target,[])

def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for (yd,cd),vector,kind,weighted in itertools.product([(8,8),(10,12),(14,8)],[(0,0),(-3,2),(8,-4)],['skip','mc','res4','res8','mixed'],[False,True]):
        name=f'p-y{yd}-c{cd}-x{vector[0]}-y{vector[1]}-{kind}-w{int(weighted)}'
        cfg,packets,expected=make_case(yd,cd,vector,kind,weighted)
        for i,p in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(p)
        for ext,data in [('avcc',cfg),('mp4',mux(cfg,[(i,i==0,p)for i,p in enumerate(packets)],32,16,25)),('y16',struct.pack('<4608H',*expected))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,yd=yd,cd=cd,kind=kind,weighted=weighted,vector=vector))
    assert len(cases)==90
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
