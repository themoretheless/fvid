"""Shared 444 inter component-specific nonzero strengths and mapped chroma QPs."""
import itertools,json,struct
from pathlib import Path
from generate_avc_444_inter import configuration,header,Writer,mux
from generate_avc_444_deblock import coefficient,chroma_qp
from generate_avc_separate_8x8_deblock import filter_edge_sample
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-inter-deblock'
def filtered(plane,depth,qp,active,mode):
    out=list(plane)
    if mode==1 or not active:return out
    for mb in range(2):
        for vertical in [True,False]:
            for edge in [0,4,8,12]:
                if edge==0 and (not vertical or mb==0):continue
                for line in range(16):
                    x=mb*16+(edge if vertical else line);y=line if vertical else edge;at=y*32+x;step=1 if vertical else 32
                    p=[out[at-(i+1)*step]for i in range(4)];q=[out[at+i*step]for i in range(4)]
                    p,q=filter_edge_sample(p,q,qp,depth,2)
                    for i in range(3):out[at-(i+1)*step]=p[i];out[at+i*step]=q[i]
    return out

def generate():
    OUT.mkdir(exist_ok=True);cases=[];witness={}
    for (yd,cd),activity,offsets,mode in itertools.product([(8,8),(10,12),(14,8)],['all','cb','cr'],[(0,0),(-6,6)],[0,1,2]):
        depths=[yd,cd,cd];active=[activity=='all',activity in ['all','cb'],activity in ['all','cr']]
        cfg=configuration(yd,cd,False,offsets=offsets);source=[[(90+20*p+2*((x//4+y//4)%2))*(1<<(depth-8))for y in range(16)for x in range(32)]for p,depth in enumerate(depths)]
        b=header(0,False)
        for mb in range(2):
            b.ue(25);b.align()
            for p,depth in enumerate(depths):
                for y in range(16):
                    for x in range(16):b.u(source[p][y*32+mb*16+x],depth)
        nal=b.nal(0x65);packets=[len(nal).to_bytes(4,'big')+nal]
        b=Writer();b.ue(0);b.ue(0);b.ue(0);b.u(1,4);b.u(2,4);b.u(0);b.u(0);b.u(0);b.se(0);b.ue(mode)
        if mode!=1:b.se(0);b.se(0)
        raw=[list(p)for p in source];qps=[26,chroma_qp(26,offsets[0],cd),chroma_qp(26,offsets[1],cd)]
        for mb in range(2):
            b.ue(0);b.ue(0);b.se(0);b.se(0);b.ue(9);b.u(0);b.se(0)
            for p,depth in enumerate(depths):
                sign=-1 if (p+mb)%2 else 1
                for _ in range(16):
                    if active[p]:coefficient(b,4*sign,0)
                    else:b.u(1)
                if active[p]:
                    q=qps[p]+6*(depth-8);v=4*sign*16*[10,11,13,14,16,18][q%6]
                    dc=v<<(q//6-4)if q//6>=4 else(v+(1<<(3-q//6)))>>(4-q//6);delta=(dc+32)>>6
                    for y in range(16):
                        for x in range(16):at=y*32+mb*16+x;raw[p][at]=max(0,min((1<<depth)-1,raw[p][at]+delta))
        nal=b.nal(0x41);packets.append(len(nal).to_bytes(4,'big')+nal)
        target=[filtered(plane,depth,qps[p],active[p],mode)for p,(plane,depth)in enumerate(zip(raw,depths))]
        if mode!=1:
            for p in range(3):
                count=sum(a!=b for a,b in zip(raw[p],target[p]));assert active[p] or count==0
                if active[p]:key=f'{activity}-f{mode}-p{p}';witness[key]=witness.get(key,0)+count
        b=header(2,False);b.ue(2);nal=b.nal(0x41);packets.append(len(nal).to_bytes(4,'big')+nal)
        name=f'filter-y{yd}-c{cd}-{activity}-o{offsets[0]}-{offsets[1]}-f{mode}'
        for i,p in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(p)
        for ext,data in [('avcc',cfg),('mp4',mux(cfg,[(i,i==0,p)for i,p in enumerate(packets)],32,16,25)),('y16',struct.pack('<4608H',*sum(source+target+target,[])))]: (OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,yd=yd,cd=cd,activity=activity,offsets=offsets,mode=mode))
    assert len(cases)==54 and len(witness)==10 and all(v>0 for v in witness.values())
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');(OUT/'witnesses.json').write_text(json.dumps(witness,indent=2)+'\n')
if __name__=='__main__':generate()
