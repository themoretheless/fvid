"""Own complementary separate-plane CAVLC/CABAC fields and physical parity oracle."""
import json,struct
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer,CabacWriter
from generate_avc_separate_intra import lists,KINDS
from avc_fixture_mp4 import mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-separate-fields'

def configuration(depth,cabac,scaled):
    b=Writer()
    for v in [244,0,41]:b.u(v,8)
    b.ue(0);b.ue(3);b.u(1);b.ue(depth-8);b.ue(0);b.u(0);b.u(int(scaled))
    if scaled:
        lists(b,[8,16,24,16,16,16])
        for _ in range(6):b.u(0)
    b.ue(0);b.ue(0);b.ue(0);b.ue(2);b.u(0);b.ue(1);b.ue(0);b.u(0);b.u(0);b.u(1);b.u(1)
    for _ in range(4):b.ue(1)
    b.u(0);sps=b.nal(0x67)
    b=Writer();b.ue(0);b.ue(0);b.u(int(cabac));b.u(0);b.ue(0);b.ue(0);b.ue(0);b.u(0);b.u(0,2)
    b.se(0);b.se(0);b.se(0);b.u(1);b.u(0);b.u(0);pps=b.nal(0x68)
    return bytes([1,244,0,41,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([1])+len(pps).to_bytes(2,'big')+pps

def field(depth,cabac,scaled,rotation,bottom,index):
    nals=[];planes=[]
    for component in range(3):
        blocks=[];kind=KINDS[(rotation+component)%5]
        for address in range(2):
            b=Writer();b.ue(address);b.ue(2);b.ue(0);b.u(component,2);b.u(0,4);b.u(1);b.u(int(bottom))
            if index==0:b.ue(0)
            b.u(index,4)
            if index==0:b.u(0);b.u(0)
            else:b.u(0)
            b.se(24);b.ue(1)
            sign=-1 if (kind=='negative') ^ bool(address) ^ bottom else 1
            pcm=[(x*17+y*23+component*59+address*41+int(bottom)*79)&((1<<depth)-1) for y in range(16)for x in range(16)]
            if cabac:
                while len(b.bits)%8:b.u(1)
                c=CabacWriter(-1,50);c.decision(3,int(kind!='i4-zero'))
                if kind=='pcm':
                    b.bits.extend(c.finish());b.align()
                    for v in pcm:b.u(v,depth)
                    c.low=0;c.range=510;c.width=9;b.bits.extend(c.finish())
                elif kind=='i4-zero':
                    for _ in range(16):c.decision(68,1)
                    for ctx in [73,74,75,76]:c.decision(ctx,0)
                    b.bits.extend(c.finish())
                else:
                    c.range-=2;c.renormalize();c.decision(6,0);c.decision(7,0);c.decision(9,1);c.decision(10,0)
                    c.decision(60,0);c.decision(88,int(kind!='zero'))
                    if kind!='zero':c.decision(277,1);c.decision(338,1);c.decision(228,0);c.bypass(int(sign<0))
                    b.bits.extend(c.finish())
            else:
                b.ue(25 if kind=='pcm' else 0 if kind=='i4-zero' else 3)
                if kind=='pcm':
                    b.align()
                    for v in pcm:b.u(v,depth)
                elif kind=='i4-zero':
                    for _ in range(16):b.u(1)
                    b.ue(1)
                else:
                    b.se(0)
                    if kind=='zero':b.u(1)
                    else:b.u(1,2);b.u(int(sign<0));b.u(1)
            if kind=='pcm':values=pcm
            else:
                weight=[8,16,24][component] if scaled else 16
                qp=50+6*(depth-8);dc=sign*13*weight*(1<<(qp//6-6)) if kind in ['positive','negative'] else 0
                value=max(0,min((1<<depth)-1,(1<<(depth-1))+((dc+32)>>6)));values=[value]*256
            blocks.append(values);nal=b.nal(0x65 if index==0 else 0x41,trailing=not cabac)
            nals.append(len(nal).to_bytes(4,'big')+nal)
        planes.append([v for y in range(16)for block in blocks for v in block[y*16:y*16+16]])
    return b''.join(nals),planes

def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for depth in [8,10,14]:
        for cabac in [False,True]:
            for scaled in [False,True]:
                for bottom_first in [False,True]:
                    for rotation in range(5):
                        name=f'separate-field-{depth}-e{int(cabac)}-w{int(scaled)}-b{int(bottom_first)}-k{rotation}'
                        config=configuration(depth,cabac,scaled);packets=[];physical={}
                        for index,bottom in enumerate([True,False] if bottom_first else [False,True]):
                            packet,planes=field(depth,cabac,scaled,rotation,bottom,index);packets.append(packet);physical[bottom]=planes
                            (OUT/f'{name}-{index}.packet').write_bytes(packet)
                        assert physical[False]!=physical[True]
                        woven=[]
                        for c in range(3):
                            for y in range(16):
                                for bottom in [False,True]:woven.extend(physical[bottom][c][y*32:(y+1)*32])
                        for suffix,data in [('avcc',config),('mp4',mux(config,[(i,i==0,p)for i,p in enumerate(packets)],32,32,50)),('y16',struct.pack('<3072H',*woven))]:(OUT/f'{name}.{suffix}').write_bytes(data)
                        cases.append(dict(name=name,depth=depth,cabac=cabac,scaled=scaled,bottom_first=bottom_first,rotation=rotation))
    assert len(cases)==120
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
