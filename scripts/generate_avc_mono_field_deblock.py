"""Own monochrome I-field luma filtering with scalar strong/weak raster oracle."""
import json,struct,itertools
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_pcm_formats import Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-mono-field-deblock'
def filtered(source,depth,mode,field=True):
    out=source[:]
    if mode==1:return out
    scale=1<<(depth-8);alpha=15*scale;beta=6*scale;maximum=(1<<depth)-1
    for my in range(2):
        for mx in range(2):
            for vertical in [True,False]:
                for edge in [0,4,8,12]:
                    if edge==0 and (mode==2 or (mx==0 if vertical else my==0)):continue
                    strength=4 if edge==0 and (vertical or not field) else 3
                    step=1 if vertical else 32
                    for line in range(16):
                        x=mx*16+(edge if vertical else line);y=my*16+(line if vertical else edge);at=y*32+x
                        p=[out[at-(i+1)*step] for i in range(4)];q=[out[at+i*step] for i in range(4)];a=p[:];b=q[:]
                        if abs(p[0]-q[0])>=alpha or abs(p[1]-p[0])>=beta or abs(q[1]-q[0])>=beta:continue
                        ap=abs(p[2]-p[0])<beta;aq=abs(q[2]-q[0])<beta
                        if strength==4:
                            strong=abs(p[0]-q[0])<alpha//4+2
                            if strong and ap:a[:3]=[(p[2]+2*p[1]+2*p[0]+2*q[0]+q[1]+4)//8,(p[2]+p[1]+p[0]+q[0]+2)//4,(2*p[3]+3*p[2]+p[1]+p[0]+q[0]+4)//8]
                            else:a[0]=(2*p[1]+p[0]+q[1]+2)//4
                            if strong and aq:b[:3]=[(q[2]+2*q[1]+2*q[0]+2*p[0]+p[1]+4)//8,(q[2]+q[1]+q[0]+p[0]+2)//4,(2*q[3]+3*q[2]+q[1]+q[0]+p[0]+4)//8]
                            else:b[0]=(2*q[1]+q[0]+p[1]+2)//4
                        else:
                            tc=scale+int(ap)+int(aq);delta=max(-tc,min(tc,(4*(q[0]-p[0])+p[1]-q[1]+4)//8))
                            a[0]=max(0,min(maximum,p[0]+delta));b[0]=max(0,min(maximum,q[0]-delta));average=(p[0]+q[0]+1)//2
                            if ap:a[1]+=max(-scale,min(scale,(p[2]+average-2*p[1])//2))
                            if aq:b[1]+=max(-scale,min(scale,(q[2]+average-2*q[1])//2))
                        for i in range(3):out[at-(i+1)*step]=a[i];out[at+i*step]=b[i]
    return out

def field(depth,cabac,bottom,index,mode,axis):
    nals=[];values=[];qp=26+6*(depth-8);v=4*13*16
    dc=v<<(qp//6-6) if qp//6>=6 else (v+(1<<(5-qp//6)))>>(6-qp//6)
    for mb in range(4):
        sign=(1 if [mb%2,mb//2][axis] else -1)*(-1 if bottom else 1)
        values.append((1<<(depth-1))+(sign*dc+32)//64)
        b=Writer();b.ue(mb);b.ue(2);b.ue(0);b.u(0,4);b.u(1);b.u(int(bottom))
        if index==0:b.ue(0)
        b.u(index,4)
        if index==0:b.u(0);b.u(0)
        else:b.u(0)
        b.se(0);b.ue(mode)
        if mode!=1:b.se(0);b.se(0)
        if not cabac:b.ue(3);b.se(0);b.u(5,6);b.u(1,6 if sign<0 else 5);b.u(1)
        else:
            while len(b.bits)%8:b.u(1)
            c=CabacWriter(-1,26);c.decision(3,1);c.range-=2;c.renormalize()
            for ctx,value in [(6,0),(7,0),(9,1),(10,0),(60,0),(88,1),(277,1),(338,1),(228,1),(232,1),(232,1),(232,0)]:c.decision(ctx,value)
            c.bypass(int(sign<0));b.bits.extend(c.finish())
        nals.append(b.nal(0x65 if index==0 else 0x41,trailing=not cabac))
    source=[values[y//16*2+x//16] for y in range(32) for x in range(32)]
    target=filtered(source,depth,mode)
    if mode==0:
        assert target!=source
        if axis==1:assert target!=filtered(source,depth,mode,False), 'field horizontal bS3 must differ from frame bS4'
    else:assert target==source
    return b''.join(len(n).to_bytes(4,'big')+n for n in nals),target

def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for depth,cabac,reverse,mode,axis in itertools.product([8,10,14],[False,True],[False,True],[0,1,2],[0,1]):
        name=f'mono-field-filter-y{depth}-e{int(cabac)}-r{int(reverse)}-m{mode}-a{axis}';config=configuration(0,depth,8,False,cabac,height_mbs=2);packets=[];fields={}
        for index,bottom in enumerate([True,False] if reverse else [False,True]):
            p,plane=field(depth,cabac,bottom,index,mode,axis);packets.append(p);fields[bottom]=plane
        assert fields[False]!=fields[True]
        woven=[v for y in range(32) for bottom in [False,True] for v in fields[bottom][y*32:(y+1)*32]]
        for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
        for ext,data in [('avcc',config),('mp4',mux(config,[(i,i==0,p) for i,p in enumerate(packets)],32,64,50)),('y16',struct.pack('<2048H',*woven))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,format=0,luma_depth=depth,chroma_depth=8,cabac=cabac,bottom_first=reverse,filter_mode=mode,axis=axis,crop=[0,0,0,0]))
    assert len(cases)==72
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
