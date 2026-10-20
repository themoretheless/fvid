"""Profile-invalid High 4:4:4 Predictive/FMO refusal fixtures, not acceptance."""
import json,struct
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer
from generate_avc_separate_intra import lists
from avc_fixture_mp4 import mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-separate-fmo'

def mapping(kind,direction):
    if kind==0:return [0,1,0,1]
    if kind==1:return [0,1,1,0]
    if kind==2:return [0,1,1,1]
    if kind==3:return [0,1,0,1] if direction else [1,1,0,0]
    if kind==4:return [1,1,0,0] if direction else [0,0,1,1]
    if kind==5:return [1,0,1,0] if direction else [0,1,0,1]
    return [1,0,0,1]

def configuration(depth,kind,direction):
    b=Writer()
    for v in [244,0,51]:b.u(v,8)
    b.ue(0);b.ue(3);b.u(1);b.ue(depth-8);b.ue(0);b.u(0);b.u(1)
    lists(b,[8,16,24,16,16,16])
    for _ in range(6):b.u(0)
    b.ue(0);b.ue(0);b.ue(0);b.ue(1);b.u(0);b.ue(1);b.ue(1);b.u(1);b.u(1);b.u(1)
    for _ in range(4):b.ue(1)
    b.u(0);sps=b.nal(0x67)
    b=Writer();b.ue(0);b.ue(0);b.u(0);b.u(0);b.ue(1);b.ue(kind)
    if kind in [0,2]:b.ue(0);b.ue(0)
    elif kind in [3,4,5]:b.u(int(direction));b.ue(0)
    elif kind==6:
        b.ue(3)
        for group in mapping(kind,direction):b.u(group)
    b.ue(0);b.ue(0);b.u(0);b.u(0,2);b.se(0);b.se(0);b.se(0);b.u(1);b.u(0);b.u(0)
    pps=b.nal(0x68)
    return bytes([1,244,0,51,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([1])+len(pps).to_bytes(2,'big')+pps

def stream(depth,kind,direction,reverse,sign):
    nals=[];planes=[];groups=mapping(kind,direction)
    for component in range(3):
        plane=[0]*1024
        for group in [0,1]:
            addresses=[a for a,g in enumerate(groups) if g==group]
            b=Writer();b.ue(addresses[0]);b.ue(2);b.ue(0);b.u(component,2);b.u(0,4);b.ue(0);b.u(0,4);b.u(0);b.u(0);b.se(24);b.ue(1)
            if kind in [3,4,5]:b.u(2,3)
            done={}
            for index,address in enumerate(addresses):
                x,y=address%2*16,address//2*16
                if index==0:
                    b.ue(25);b.align()
                    values=[(u*17+v*23+component*59+address*41)&((1<<depth)-1) for v in range(16)for u in range(16)]
                    for value in values:b.u(value,depth)
                    done[address]=16
                else:
                    top=address-2 in done
                    left=address%2!=0 and address-1 in done
                    counts=[done[a] for a,available in [(address-2,top),(address-1,left)] if available]
                    nc=(sum(counts)+1)//2 if len(counts)==2 else counts[0] if counts else 0
                    b.ue(3);b.se(0)
                    level=sign*(-1 if (component+address)%2 else 1)
                    if level:b.u(1,6 if nc>=8 else 2);b.u(int(level<0));b.u(1)
                    elif nc>=8:b.u(3,6)
                    else:b.u(1)
                    edges=[]
                    if top:edges+=plane[(y-1)*32+x:(y-1)*32+x+16]
                    if left:edges += [plane[(y+j)*32+x-1]for j in range(16)]
                    prediction=(sum(edges)+len(edges)//2)//len(edges) if edges else 1<<(depth-1)
                    qp=50+6*(depth-8)
                    dc=level*13*[8,16,24][component]*(1<<(qp//6-6))
                    value=max(0,min((1<<depth)-1,prediction+((dc+32)>>6)))
                    values=[value]*256;done[address]=0
                for row in range(16):plane[(y+row)*32+x:(y+row)*32+x+16]=values[row*16:(row+1)*16]
            nal=b.nal(0x65);nals.append((component,addresses[0],len(nal).to_bytes(4,'big')+nal))
        planes.append(plane)
    nals.sort(key=lambda n:(n[0],n[1]),reverse=reverse)
    return b''.join(n[2]for n in nals),sum(planes,[])

def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for depth in [8]:
        for kind in range(7):
            for direction in ([False,True] if kind in [3,4,5] else [False]):
                for reverse in [False]:
                    for sign in [0]:
                        name=f'separate-fmo-{depth}-t{kind}-d{int(direction)}-r{int(reverse)}-s{sign}'
                        config=configuration(depth,kind,direction);packet,values=stream(depth,kind,direction,reverse,sign)
                        for suffix,data in [('avcc',config),('packet',packet),('mp4',mux(config,[(0,True,packet)],32,32,25))]:(OUT/f'{name}.{suffix}').write_bytes(data)
                        cases.append(dict(name=name,depth=depth,map_type=kind,direction=direction,reverse=reverse,sign=sign))
    assert len(cases)==10
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
