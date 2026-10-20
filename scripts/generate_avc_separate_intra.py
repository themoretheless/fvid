"""Own separate-plane intra PCM/I16/I4 with signed DC and plane-specific scaling."""
import json, struct
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer, CabacWriter
from avc_fixture_mp4 import mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-separate-intra'
KINDS=['positive','negative','zero','i4-zero','pcm']

def lists(b,weights):
    for weight in weights:
        b.u(1);b.se(weight-8)
        for _ in range(15):b.se(0)

def config(depth,cabac,scaling):
    b=Writer()
    for v in [244,0,51]:b.u(v,8)
    b.ue(0);b.ue(3);b.u(1);b.ue(depth-8);b.ue(0);b.u(0);b.u(int(scaling=='sps'))
    if scaling=='sps':
        lists(b,[8,16,24,16,16,16])
        for _ in range(6):b.u(0)
    b.ue(0);b.ue(0);b.ue(0);b.ue(1);b.u(0);b.ue(1);b.ue(0);b.u(1);b.u(1);b.u(1)
    for _ in range(4):b.ue(1)
    b.u(0);sps=b.nal(0x67)
    b=Writer();b.ue(0);b.ue(0);b.u(int(cabac));b.u(0);b.ue(0);b.ue(0);b.ue(0);b.u(0);b.u(0,2)
    b.se(0);b.se(0);b.se(0);b.u(1);b.u(0);b.u(0)
    if scaling=='pps':
        b.u(0);b.u(1);lists(b,[24,16,8,16,16,16]);b.se(0)
    pps=b.nal(0x68)
    return bytes([1,244,0,51,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([1])+len(pps).to_bytes(2,'big')+pps

def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for depth in [8,10,14]:
        for cabac in [False,True]:
            for scaling in ['flat','sps','pps']:
                weights={'flat':[16]*3,'sps':[8,16,24],'pps':[24,16,8]}[scaling]
                for reverse in [False,True]:
                    for rotation in range(5):
                        name=f'separate-intra-{depth}-e{int(cabac)}-{scaling}-r{int(reverse)}-k{rotation}'
                        cfg=config(depth,cabac,scaling);nals=[];planes=[]
                        for plane in range(3):
                            kind=KINDS[(rotation+plane)%5];macroblocks=[]
                            for address in range(2):
                                b=Writer();b.ue(address);b.ue(2);b.ue(0);b.u(plane,2);b.u(0,4);b.ue(0);b.u(0,4);b.u(0);b.u(0);b.se(24);b.ue(1)
                                sign=-1 if (kind=='negative') ^ bool(address) else 1
                                if cabac:
                                    while len(b.bits)%8:b.u(1)
                                    c=CabacWriter(-1,50);c.decision(3,int(kind!='i4-zero'))
                                    if kind=='pcm':
                                        b.bits.extend(c.finish());b.align()
                                        samples=[(x*17+y*23+address*41+plane*59)% (1<<depth)for y in range(16)for x in range(16)]
                                        for v in samples:b.u(v,depth)
                                        c.low=0;c.range=510;c.width=9;b.bits.extend(c.finish())
                                    elif kind=='i4-zero':
                                        for _ in range(16):c.decision(68,1)
                                        for ctx in [73,74,75,76]:c.decision(ctx,0)
                                        b.bits.extend(c.finish())
                                    else:
                                        c.range-=2;c.renormalize();c.decision(6,0);c.decision(7,0);c.decision(9,1);c.decision(10,0)
                                        c.decision(60,0);c.decision(88,int(kind!='zero'))
                                        if kind!='zero':
                                            c.decision(105,1);c.decision(166,1);c.decision(228,0);c.bypass(int(sign<0))
                                        b.bits.extend(c.finish())
                                else:
                                    b.ue(25 if kind=='pcm' else 0 if kind=='i4-zero' else 3)
                                    if kind=='pcm':
                                        b.align();samples=[(x*17+y*23+address*41+plane*59)% (1<<depth)for y in range(16)for x in range(16)]
                                        for v in samples:b.u(v,depth)
                                    elif kind=='i4-zero':
                                        for _ in range(16):b.u(1)
                                        b.ue(1)
                                    else:
                                        b.se(0)
                                        if kind=='zero':b.u(1)
                                        else:b.u(1,2);b.u(int(sign<0));b.u(1)
                                if kind!='pcm':
                                    qp=50+6*(depth-8)
                                    dc=sign*13*weights[plane]*(1<<(qp//6-6)) if kind in ['positive','negative'] else 0
                                    value=max(0,min((1<<depth)-1,(1<<(depth-1))+((dc+32)>>6)))
                                    samples=[value]*256
                                macroblocks.append(samples);nals.append(b.nal(0x65,trailing=not cabac))
                            planes.append([v for y in range(16)for mb in macroblocks for v in mb[y*16:y*16+16]])
                        if reverse:nals.reverse()
                        packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals)
                        values=sum(planes,[])
                        for suffix,data in [('avcc',cfg),('packet',packet),('mp4',mux(cfg,[(0,True,packet)],32,16,25)),('y16',struct.pack('<1536H',*values))]:(OUT/f'{name}.{suffix}').write_bytes(data)
                        cases.append(dict(name=name,depth=depth,cabac=cabac,scaling=scaling,rotation=rotation,reverse=reverse))
    assert len(cases)==180
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
