"""Owned compact 422 I-field filtering with horizontal bS3/frame-bS4 witnesses."""
import json,struct,itertools
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_pcm_formats import Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
from generate_avc_422_deblock import filtered
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-field-deblock'
def field(yd,cd,cabac,bottom,index,mode,orientation):
    internal=orientation=='internal';vertical=orientation=='vertical';nals=[];values=[]
    qp=26+6*(cd-8)+3;value=6*18*16
    dc=value<<(qp//6-6) if qp//6>=6 else (value+(1<<(5-qp//6)))>>(6-qp//6)
    for mb in range(4):
        sign=(1 if internal or (mb%2 if vertical else mb//2) else -1)*(-1 if bottom else 1)
        values.append([(1<<(cd-1))+((s*dc+32)//64) for s in [sign,-sign]])
        b=Writer();b.ue(mb);b.ue(2);b.ue(0);b.u(0,4);b.u(1);b.u(int(bottom))
        if index==0:b.ue(0)
        b.u(index,4)
        if index==0:b.u(0);b.u(0)
        else:b.u(0)
        b.se(0);b.ue(mode)
        if mode!=1:b.se(0);b.se(0)
        if not cabac:
            b.ue(7);b.ue(0);b.se(0);b.u(1)
            for s in [sign,-sign]:b.u(15,7);b.u(1,10 if s<0 else 9);b.u(2,4) if internal else b.u(1)
        else:
            while len(b.bits)%8:b.u(1)
            c=CabacWriter(-1,26);c.decision(3,1);c.range-=2;c.renormalize()
            for ctx,v in [(6,0),(7,1),(8,0),(9,1),(10,0),(64,0),(60,0),(88,0)]:c.decision(ctx,v)
            for s in [sign,-sign]:
                c.decision(100,1)
                if internal:
                    for ctx,v in [(321,0),(321,0),(322,0),(322,1),(383,1)]:c.decision(ctx,v)
                else:c.decision(321,1);c.decision(382,1)
                for ctx,v in [(258,1),(262,1),(262,1),(262,1),(262,1),(262,0)]:c.decision(ctx,v)
                c.bypass(int(s<0))
            b.bits.extend(c.finish())
        nals.append(b.nal(0x65 if index==0 else 0x41,trailing=not cabac))
    planes=[[1<<(yd-1)]*1024]
    for component in range(2):
        plane=[values[y//16*2+x//8][component] for y in range(32) for x in range(16)]
        if internal:plane=[(1<<(cd-1))+(([1,-1,-1,1][y%16//4]*(1 if component==0 else -1)*(-1 if bottom else 1)*dc+32)//64) for y in range(32) for x in range(16)]
        decoded=filtered(plane,cd,mode,field=True)
        if mode==0 or (mode==2 and internal):assert decoded!=plane
        else:assert decoded==plane
        if mode==0 and orientation=='horizontal':assert decoded!=filtered(plane,cd,mode), 'field bS3 must differ from frame bS4'
        planes.append(decoded)
    return b''.join(len(n).to_bytes(4,'big')+n for n in nals),planes
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for (yd,cd),cabac,reverse,mode,orientation in itertools.product([(8,8),(10,12),(14,8)],[False,True],[False,True],[0,1,2],['vertical','horizontal','internal']):
        name=f'422-field-filter-y{yd}-c{cd}-e{int(cabac)}-r{int(reverse)}-m{mode}-{orientation}'
        config=configuration(2,yd,cd,False,cabac,height_mbs=2);packets=[];fields={}
        for index,bottom in enumerate([True,False] if reverse else [False,True]):
            p,planes=field(yd,cd,cabac,bottom,index,mode,orientation);packets.append(p);fields[bottom]=planes
        assert fields[False]!=fields[True]
        woven=[]
        for p,w in enumerate([32,16,16]):
            for y in range(32):
                for bottom in [False,True]:woven.extend(fields[bottom][p][y*w:(y+1)*w])
        for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
        for ext,data in [('avcc',config),('mp4',mux(config,[(i,i==0,p) for i,p in enumerate(packets)],32,64,50)),('y16',struct.pack('<4096H',*woven))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,format=2,luma_depth=yd,chroma_depth=cd,cabac=cabac,bottom_first=reverse,filter_mode=mode,orientation=orientation,crop=[0,0,0,0]))
    assert len(cases)==108
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
