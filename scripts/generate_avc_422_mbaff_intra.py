"""Owned unfiltered 422 MBAFF I4/I16, rectangular DC/AC and physical prediction."""
import json,struct,itertools
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_component_mbaff_pcm import MODES
from generate_avc_pcm_formats import Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-mbaff-intra'
def flags(component,parity,pair,index,ac):
    return [0]*8 if ac=='none' else [1]*8 if ac=='full' else [v^component^parity^(pair%2)^index for v in [1,0,0,1,1,0,0,1]]
def picture(yd,cd,cabac,topology,aso,luma,dc_sign,ac,index,filter_mode=1):
    planes=[[1<<(yd-1)]*2048,[0]*1024,[0]*1024];nals=[];witnesses=0;pattern=2 if ac!='none' else int(dc_sign!=0)
    qp=26+6*(cd-8);qpdc=qp+3
    for pair,field in enumerate(MODES[topology]):
        available=[[False]*1024 for _ in range(2)];step=2 if field else 1
        b=Writer();b.ue(pair);b.ue(2);b.ue(0);b.u(index,4);b.u(0)
        if index==0:b.ue(0)
        b.u(index*2,4)
        if index==0:b.u(0);b.u(0)
        else:b.u(0)
        b.se(0);b.ue(filter_mode)
        if filter_mode!=1:b.se(0);b.se(0)
        if cabac:
            while len(b.bits)%8:b.u(1)
            c=CabacWriter(-1,26);c.decision(70,int(field))
        for parity in range(2):
            i16=luma=='i16';negative=(pair+parity+index)%2
            if cabac:
                c.decision(3+int(i16 and parity==1 and not field),int(i16))
                if i16:
                    c.range-=2;c.renormalize();c.decision(6,0);c.decision(7,int(pattern>0))
                    if pattern:c.decision(8,int(pattern==2))
                    c.decision(9,1);c.decision(10,0)
                else:
                    for _ in range(16):c.decision(68,1)
                c.decision(64,0)
                if not i16:
                    for ctx in ([75,76,75,76] if parity==1 and not field else [73,74,75,76]):c.decision(ctx,0)
                    c.decision(77+2*int(parity==1 and not field and pattern>0),int(pattern>0))
                    if pattern:c.decision(81+2*int(parity==1 and not field and pattern==2),int(pattern==2))
                if i16 or pattern:c.decision(60,0)
                if i16:c.decision(86 if parity==1 and not field else 88,0)
                if pattern:
                    for component in range(2):
                        sign=dc_sign*(1 if component==0 else -1)*(-1 if negative else 1)
                        c.decision(98 if parity==1 and not field and dc_sign==0 else 100,int(sign!=0))
                        if sign:c.decision(321 if field else 149,1);c.decision(382 if field else 210,1);c.decision(258,0);c.bypass(int(sign<0))
                if pattern==2:
                    for component in range(2):
                        current=flags(component,parity,pair,index,ac);previous=flags(component,0,pair,index,ac)
                        for block,coded in enumerate(current):
                            left=current[block-1] if block%2 else 1
                            top=current[block-2] if block>=2 else previous[6+block] if parity==1 and not field else 1
                            c.decision(101+left+2*top,coded)
                            if coded:c.decision(324 if field else 152,1);c.decision(385 if field else 213,1);c.decision(267,0);c.bypass((component+pair+parity+index+block)%2)
            else:
                if parity==0:b.u(int(field))
                b.ue(3+4*pattern if i16 else 0)
                if not i16:
                    for _ in range(16):b.u(1)
                b.ue(0)
                if not i16:b.ue([3,16,41][pattern])
                if i16 or pattern:b.se(0)
                if i16:b.u(1)
                if pattern:
                    for component in range(2):
                        sign=dc_sign*(1 if component==0 else -1)*(-1 if negative else 1)
                        if sign:b.u(1,2);b.u(int(sign<0));b.u(1)
                        else:b.u(1)
                if pattern==2:
                    for component in range(2):
                        for block,coded in enumerate(flags(component,parity,pair,index,ac)):
                            if coded:b.u(1,2);b.u((component+pair+parity+index+block)%2);b.u(1)
                            else:b.u(1)
            ox=pair%2*8;oy=pair//2*32+(parity if field else parity*16)
            for component in range(2):
                plane=planes[component+1];ready=available[component]
                top_at=[(oy-step)*16+ox+x for x in range(8)] if oy>=step else []
                left_at=[(oy+y*step)*16+ox-1 for y in range(16)] if ox else []
                top=[plane[a] for a in top_at] if top_at and all(ready[a] for a in top_at) else None
                left=[plane[a] for a in left_at] if left_at and all(ready[a] for a in left_at) else None
                sign=dc_sign*(1 if component==0 else -1)*(-1 if negative else 1);v=sign*18*16
                dc=v<<(qpdc//6-6) if qpdc//6>=6 else (v+(1<<(5-qpdc//6)))>>(6-qpdc//6)
                for y in range(16):
                    for x in range(8):
                        ts=sum(top[x//4*4:x//4*4+4]) if top is not None else None;ls=sum(left[y//4*4:y//4*4+4]) if left is not None else None
                        if x>=4 and y<4:edge=ts if ts is not None else ls;pred=(edge+2)//4 if edge is not None else 1<<(cd-1)
                        elif x<4 and y>=4:edge=ls if ls is not None else ts;pred=(edge+2)//4 if edge is not None else 1<<(cd-1)
                        elif ts is not None and ls is not None:pred=(ts+ls+4)//8
                        else:edge=ts if ts is not None else ls;pred=(edge+2)//4 if edge is not None else 1<<(cd-1)
                        block=y//4*2+x//4;s=-1 if (component+pair+parity+index+block)%2 else 1
                        amplitude=s*8*(1<<(qp//6)) if flags(component,parity,pair,index,ac)[block] else 0
                        residual=(dc+[2,1,-1,-2][y%4 if field else x%4]*amplitude+32)//64
                        value=max(0,min((1<<cd)-1,pred+residual))
                        wrong=(dc+[2,1,-1,-2][x%4 if field else y%4]*amplitude+32)//64
                        witnesses+=int(value!=max(0,min((1<<cd)-1,pred+wrong)))
                        at=(oy+y*step)*16+ox+x;plane[at]=value;ready[at]=True
        if cabac:b.bits.extend(c.finish())
        nals.append(b.nal(0x65 if index==0 else 0x41,trailing=not cabac))
    if ac!='none':assert witnesses>0,'422 AC orientation must affect decoded samples'
    if aso:nals.reverse()
    return b''.join(len(n).to_bytes(4,'big')+n for n in nals),sum(planes,[])
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for (yd,cd),cabac,topology,aso,crop,luma,dc,ac in itertools.product([(8,8),(10,12),(14,8)],[False,True],MODES,[False,True],[False,True],['i4','i16'],[-1,0,1],['none','full','mask']):
        name=f'422-mbaff-y{yd}-c{cd}-e{int(cabac)}-{topology}-a{int(aso)}-crop{int(crop)}-{luma}-dc{dc}-{ac}'
        config=configuration(2,yd,cd,crop,cabac,height_mbs=2,mbaff=True);packets=[];words=[]
        for index in range(2):
            packet,values=picture(yd,cd,cabac,topology,aso,luma,dc,ac,index);packets.append(packet);words+=values;(OUT/f'{name}-{index}.packet').write_bytes(packet)
        for ext,data in [('avcc',config),('mp4',mux(config,[(i,i==0,p) for i,p in enumerate(packets)],32,64,25)),('y16',struct.pack('<8192H',*words))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,format=2,luma_depth=yd,chroma_depth=cd,cabac=cabac,topology=topology,aso=aso,luma=luma,dc_sign=dc,ac_mode=ac,crop=[2,2,2,2] if crop else [0,0,0,0]))
    assert len(cases)==1728
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
