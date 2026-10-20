"""Own progressive 422 P cross-slice bS=1 filtering, scalar border oracle."""
import json,struct,itertools
from pathlib import Path
from generate_avc_pcm_formats import Writer,configuration,mux
from generate_avc_mbaff_direct_samples import CabacWriter
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-inter-deblock'
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for yd,cd in [(8,8),(10,12),(14,8)]:
        for cabac in [False,True]:
            config=configuration(2,yd,cd,False,cabac,height_mbs=2)
            for axis in [0,1]:
                scale=1<<(cd-8)
                source=[[1<<(yd-1)]*1024]+[[(1<<(cd-1))+(1 if p==0 else -1)*4*scale*int((x if axis==0 else y)>=(8 if axis==0 else 16)) for y in range(32) for x in range(16)] for p in range(2)]
                for mode,motion_delta in itertools.product([0,1,2],[4,0]):
                    name=f'422-inter-filter-y{yd}-c{cd}-e{int(cabac)}-a{axis}-m{mode}'+('-same-motion' if motion_delta==0 else '')
                    packets=[]
                    for frame in range(2):
                        nals=[]
                        for mb in range(4):
                            b=Writer();b.ue(mb);b.ue(2 if frame==0 else 0);b.ue(0);b.u(frame,4)
                            if frame==0:b.ue(0)
                            b.u(frame*2,4)
                            if frame==0:b.u(0);b.u(0)
                            else:
                                b.u(0);b.u(0);b.u(0)
                                if cabac:b.ue(0)
                            b.se(0);filter_mode=1 if frame==0 else mode;b.ue(filter_mode)
                            if filter_mode!=1:b.se(0);b.se(0)
                            if cabac:
                                while len(b.bits)%8:b.u(1)
                                c=CabacWriter(-1 if frame==0 else 0,26)
                            if frame==0:
                                if cabac:c.decision(3,1);b.bits.extend(c.finish());b.align()
                                else:b.ue(25);b.align()
                                for p,(w,bw,depth) in enumerate([(32,16,yd),(16,8,cd),(16,8,cd)]):
                                    for y in range(16):
                                        for x in range(bw):b.u(source[p][(mb//2*16+y)*w+mb%2*bw+x],depth)
                                if cabac:c.low=0;c.range=510;c.width=9;b.bits.extend(c.finish())
                            else:
                                vector=[0,0];vector[axis]=motion_delta*([mb%2,mb//2][axis])
                                if cabac:
                                    for ctx in [11,14,15,16]:c.decision(ctx,0)
                                    c.mvd(0,vector[0]);c.mvd(1,vector[1])
                                    for ctx in [73,74,75,76,77]:c.decision(ctx,0)
                                    b.bits.extend(c.finish())
                                else:b.ue(0);b.ue(0);b.se(vector[0]);b.se(vector[1]);b.ue(0)
                            nals.append(b.nal(0x65 if frame==0 else 0x41,trailing=not cabac))
                        packets.append(b''.join(len(n).to_bytes(4,'big')+n for n in nals))
                    target=[p[:] for p in source]
                    if mode==0 and motion_delta!=0:
                        for p in [1,2]:
                            plane=target[p];step=1 if axis==0 else 16
                            for line in range(32 if axis==0 else 16):
                                at=line*16+8 if axis==0 else 16*16+line
                                a,a1,q,q1=plane[at-step],plane[at-2*step],plane[at],plane[at+step]
                                assert abs(a-q)<15*scale and abs(a1-a)<6*scale and abs(q1-q)<6*scale
                                tc=scale+1;delta=max(-tc,min(tc,(4*(q-a)+a1-q1+4)//8))
                                plane[at-step]=a+delta;plane[at]=q-delta
                            assert plane!=source[p]
                    for i,packet in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(packet)
                    for ext,data in [('avcc',config),('mp4',mux(config,[(0,True,packets[0]),(1,False,packets[1])],32,32,25)),('y16',struct.pack('<4096H',*(sum(source,[])+sum(target,[]))))]:(OUT/f'{name}.{ext}').write_bytes(data)
                    cases.append(dict(name=name,luma_depth=yd,chroma_depth=cd,cabac=cabac,axis=axis,filter_mode=mode,motion_delta=motion_delta))
    assert len(cases)==72
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
