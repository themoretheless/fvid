"""Owned P luma-residual bS=2 at QP32, independent scalar raster filtering."""
import json,struct
from pathlib import Path
from generate_avc_pcm_formats import Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
from generate_avc_field_b_cabac_mixed_samples import mb_type
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
BASE=ROOT/'avc-422-inter-deblock';OUT=ROOT/'avc-422-p-luma-deblock'
def filtered(source,depth,chroma,mode,strength=2):
    out=source[:]
    if mode==1:return out
    width=16 if chroma else 32;sx=8 if chroma else 16;scale=1<<(depth-8);maximum=(1<<depth)-1
    alpha,beta=(28*scale,8*scale) if chroma else (32*scale,9*scale)
    tc0=(2 if strength==2 else 1)*scale
    for my in range(2):
        for mx in range(2):
            for vertical in [True,False]:
                for edge in ([0,4] if chroma and vertical else [0,4,8,12]):
                    if edge==0 and (mode==2 or (mx==0 if vertical else my==0)):continue
                    step=1 if vertical else width
                    for line in range(16 if vertical else sx):
                        x=mx*sx+(edge if vertical else line);y=my*16+(line if vertical else edge);at=y*width+x
                        p=[out[at-(i+1)*step] for i in range(3)];q=[out[at+i*step] for i in range(3)]
                        if abs(p[0]-q[0])>=alpha or abs(p[1]-p[0])>=beta or abs(q[1]-q[0])>=beta:continue
                        ap=abs(p[2]-p[0])<beta;aq=abs(q[2]-q[0])<beta
                        tc=tc0+(1 if chroma else int(ap)+int(aq))
                        delta=max(-tc,min(tc,(4*(q[0]-p[0])+p[1]-q[1]+4)//8))
                        out[at-step]=max(0,min(maximum,p[0]+delta));out[at]=max(0,min(maximum,q[0]-delta))
                        if not chroma:
                            average=(p[0]+q[0]+1)//2
                            if ap:out[at-2*step]=p[1]+max(-tc0,min(tc0,(p[2]+average-2*p[1])//2))
                            if aq:out[at+step]=q[1]+max(-tc0,min(tc0,(q[2]+average-2*q[1])//2))
    return out
def packet(cabac,axis,mode,b_slice=False):
    nals=[]
    for mb in range(4):
        sign=1 if [mb%2,mb//2][axis] else -1
        b=Writer();b.ue(mb);b.ue(1 if b_slice else 0);b.ue(0);b.u(2 if b_slice else 1,4);b.u(2,4)
        if b_slice:b.u(1)
        b.u(0);b.u(0);b.u(0)
        if cabac:b.ue(0)
        b.se(6);b.ue(mode)
        if mode!=1:b.se(0);b.se(0)
        if cabac:
            while len(b.bits)%8:b.u(1)
            c=CabacWriter(0,32)
            if b_slice:c.decision(24,0);mb_type(c,3,0)
            else:
                for ctx in [11,14,15,16]:c.decision(ctx,0)
            for _ in range(2 if b_slice else 1):c.mvd(0,0);c.mvd(1,0)
            for _ in range(4):c.decision(73,1)
            c.decision(77,0);c.decision(60,0)
            done=set()
            for block in range(16):
                bx=(block&1)+((block>>2)&1)*2;by=((block>>1)&1)+(block>>3)*2
                inc=int((bx-1,by) in done)+2*int((bx,by-1) in done)
                c.decision(93+inc,1);c.decision(134,1);c.decision(195,1);c.decision(248,0);c.bypass(int(sign<0));done.add((bx,by))
            b.bits.extend(c.finish())
        else:
            b.ue(0);b.ue(3 if b_slice else 0)
            for _ in range(2 if b_slice else 1):b.se(0);b.se(0)
            b.ue(11);b.se(0)
            for _ in range(16):b.u(1,2);b.u(int(sign<0));b.u(1)
        nals.append(b.nal(0x01 if b_slice else 0x41,trailing=not cabac))
    return b''.join(len(n).to_bytes(4,'big')+n for n in nals)
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for yd,cd in [(8,8),(10,12),(14,8)]:
        for cabac in [False,True]:
            for axis in [0,1]:
                base=f'422-inter-filter-y{yd}-c{cd}-e{int(cabac)}-a{axis}-m1'
                config=(BASE/(base+'.avcc')).read_bytes();first=(BASE/(base+'-0.packet')).read_bytes()
                raw=(BASE/(base+'.y16')).read_bytes()[:4096];samples=struct.unpack('<2048H',raw)
                planes=[list(samples[:1024]),list(samples[1024:1536]),list(samples[1536:])]
                scale=1<<(yd-8)
                unfiltered=[p[:] for p in planes]
                for y in range(32):
                    for x in range(32):
                        sign=1 if [x//16,y//16][axis] else -1
                        unfiltered[0][y*32+x]+=(sign*13*32*scale+32)//64
                for mode in [0,1,2]:
                    name=f'422-p-luma-filter-y{yd}-c{cd}-e{int(cabac)}-a{axis}-m{mode}'
                    second=packet(cabac,axis,mode)
                    target=[filtered(p,d,i!=0,mode) for i,(p,d) in enumerate(zip(unfiltered,[yd,cd,cd]))]
                    if mode==0:
                        assert target[0]!=unfiltered[0]
                        assert target[0]!=filtered(unfiltered[0],yd,False,mode,1),'bS2 must differ from bS1'
                    else:assert target==unfiltered
                    for i,data in enumerate([first,second]):(OUT/f'{name}-{i}.packet').write_bytes(data)
                    for ext,data in [('avcc',config),('mp4',mux(config,[(0,True,first),(1,False,second)],32,32,25)),('y16',raw+struct.pack('<2048H',*sum(target,[]))) ]:(OUT/f'{name}.{ext}').write_bytes(data)
                    cases.append(dict(name=name,luma_depth=yd,chroma_depth=cd,cabac=cabac,axis=axis,filter_mode=mode,qp=32))
    assert len(cases)==36
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
