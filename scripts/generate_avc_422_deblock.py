"""Owned 422 CAVLC/CABAC cross-slice vertical/horizontal chroma filtering."""
import json,struct
from pathlib import Path
from generate_avc_pcm_formats import configuration,Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-deblock'
def filtered(samples,depth,mode,field=False):
    out=samples[:]
    if mode==1:return out
    scale=1<<(depth-8);maximum=(1<<depth)-1
    for my in range(2):
        for mx in range(2):
            for vertical in [True,False]:
                for edge in ([0,4]if vertical else[0,4,8,12]):
                    if edge==0 and ((mx==0 if vertical else my==0)or mode==2):continue
                    for line in range(16 if vertical else 8):
                        x=mx*8+(edge if vertical else line);y=my*16+(line if vertical else edge)
                        at=y*16+x;step=1 if vertical else 16
                        p,p1,q,q1=out[at-step],out[at-2*step],out[at],out[at+step]
                        if abs(p-q)>=15*scale or abs(p1-p)>=6*scale or abs(q1-q)>=6*scale:continue
                        if edge==0 and (vertical or not field):
                            a=(2*p1+p+q1+2)//4;b=(2*q1+q+p1+2)//4
                        else:
                            tc=scale+1;delta=max(-tc,min(tc,(4*(q-p)+p1-q1+4)//8))
                            a=max(0,min(maximum,p+delta));b=max(0,min(maximum,q-delta))
                        out[at-step]=a;out[at]=b
    return out
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for yd,cd in [(8,8),(10,12),(14,8)]:
        for cabac in [False,True]:
            for mode in [0,1,2]:
                for orientation in ['horizontal','vertical','internal']:
                    vertical=orientation=='vertical';internal=orientation=='internal'
                    name=f'422-filter-y{yd}-c{cd}-e{int(cabac)}-m{mode}-v{int(vertical)}'+('-internal'if internal else'');config=configuration(2,yd,cd,False,cabac,height_mbs=2);nals=[];values=[]
                    qp=26+6*(cd-8)+3;v=4*18*16
                    dc=v<<(qp//6-6)if qp//6>=6 else(v+(1<<(5-qp//6)))>>(6-qp//6)
                    for address in range(4):
                        sign=1 if internal or (address%2 if vertical else address//2) else -1
                        values.append([(1<<(cd-1))+((s*dc+32)>>6)for s in [sign,-sign]])
                        b=Writer();b.ue(address);b.ue(2);b.ue(0);b.u(0,4);b.ue(0);b.u(0,4);b.u(0);b.u(0);b.se(0);b.ue(mode)
                        if mode!=1:b.se(0);b.se(0)
                        if not cabac:
                            b.ue(7);b.ue(0);b.se(0);b.u(1) # I16 DC, chroma CBP1, zero luma DC
                            for s in [sign,-sign]:
                                b.u(15,7);b.u(1,6 if s<0 else 5);b.u(2,4)if internal else b.u(1)
                        else:
                            while len(b.bits)%8:b.u(1)
                            c=CabacWriter(-1,26);c.decision(3,1);c.range-=2;c.renormalize()
                            for ctx,value in [(6,0),(7,1),(8,0),(9,1),(10,0),(64,0),(60,0),(88,0)]:c.decision(ctx,value)
                            for s in [sign,-sign]:
                                c.decision(100,1)
                                if internal:
                                    for ctx,value in [(149,0),(149,0),(150,0),(150,1),(211,1)]:c.decision(ctx,value)
                                else:c.decision(149,1);c.decision(210,1)
                                for ctx,value in [(258,1),(262,1),(262,1),(262,0)]:c.decision(ctx,value)
                                c.bypass(int(s<0))
                            b.bits.extend(c.finish())
                        nals.append(b.nal(0x65,trailing=not cabac))
                    planes=[[1<<(yd-1)]*1024]
                    for component in range(2):
                        plane=[values[y//16*2+x//8][component]for y in range(32)for x in range(16)]
                        if internal:plane=[(1<<(cd-1))+((([1,-1,-1,1][y%16//4])*(1 if component==0 else -1)*dc+32)>>6)for y in range(32)for x in range(16)]
                        original=plane[:]
                        plane=filtered(plane,cd,mode)
                        if mode==0 or (mode==2 and internal):assert plane!=original
                        else:assert plane==original
                        planes.append(plane)
                    packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals)
                    for suffix,data in [('avcc',config),('packet',packet),('mp4',mux(config,[(0,True,packet)],32,32,25)),('y16',struct.pack('<2048H',*sum(planes,[])))]: (OUT/(name+'.'+suffix)).write_bytes(data)
                    cases.append(dict(name=name,luma_depth=yd,chroma_depth=cd,cabac=cabac,mode=mode,vertical=vertical,internal=internal))
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
