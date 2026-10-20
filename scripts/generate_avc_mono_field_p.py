"""Owned mono P fields: same-parity list reordering, qpel, explicit weights."""
import json,struct,itertools
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_pcm_formats import Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
from generate_avc_monochrome_fractional import prediction
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
BASE=ROOT/'avc-component-fields';OUT=ROOT/'avc-mono-field-p'
def p_field(index,bottom,cabac,vector,weighted,residual=None,poc_base=2,chroma=False,opposite=False):
    assert not (chroma and residual is not None), 'use the dedicated 422 field residual author'
    b=Writer();b.ue(0);b.ue(0);b.ue(0);b.u(1,4);b.u(1);b.u(int(bottom));b.u(index+poc_base,4)
    b.u(0);b.u(1);b.ue(0);b.ue(2 if opposite else 1);b.ue(3) # CurrPicNum3 -> previous opposite PicNum0 or matching PicNum1
    if weighted:
        b.ue(1)
        if chroma:b.ue(2)
        b.u(1);b.se(3);b.se(-5)
        if chroma:b.u(1);b.se(3);b.se(7);b.se(-2);b.se(64)
    b.u(0)
    if cabac:b.ue(0)
    b.se(0);b.ue(1)
    skip=vector==(0,0) and residual is None
    if cabac:
        while len(b.bits)%8:b.u(1)
        c=CabacWriter(0,26)
        for mb in range(2):
            c.decision(11+int(mb>0 and not skip),int(skip))
            if not skip:
                for ctx in [14,15,16]:c.decision(ctx,0)
                for component in range(2):c.mvd(component,vector[component] if mb==0 else 0,abs(vector[component]) if mb else 0)
                if residual is None:
                    for ctx in ([73,74,75,76] if mb==0 else [74,74,76,76]):c.decision(ctx,0)
                    if chroma:c.decision(77,0)
                else:
                    for _ in range(4):c.decision(73,1)
                    c.decision(60,0)
                    done=set()
                    for block in range(16):
                        bx=(block&1)+((block>>2)&1)*2;by=((block>>1)&1)+(block>>3)*2
                        inc=int(bx>0 or mb>0)+2*int(by>0)
                        c.decision(93+inc,1)
                        if residual=='ac':c.decision(306,0);c.decision(307,1);c.decision(368,1)
                        else:c.decision(306,1);c.decision(367,1)
                        c.decision(248,0);c.bypass((mb+block)%2);done.add((bx,by))
            if mb==0:c.range-=2;c.renormalize()
        b.bits.extend(c.finish())
    elif skip:b.ue(2)
    else:
        for mb in range(2):
            b.ue(0);b.ue(0);b.se(vector[0] if mb==0 else 0);b.se(vector[1] if mb==0 else 0);b.ue(9 if residual else 0)
            if residual:
                b.se(0)
                for block in range(16):
                    b.u(1,2);b.u((mb+block)%2)
                    b.u(3,3) if residual=='ac' else b.u(1)
    nal=b.nal(0x41,trailing=not cabac);return len(nal).to_bytes(4,'big')+nal

def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for (yd,cd),cabac,reverse,vector,weighted in itertools.product([(8,8),(10,12),(14,8)],[False,True],[False,True],[(0,0),(1,1),(-3,-1),(4,-4)],[False,True]):
        name=f'mono-field-p-y{yd}-c{cd}-e{int(cabac)}-r{int(reverse)}-x{vector[0]}-y{vector[1]}-w{int(weighted)}'
        base=f'field-f0-y{yd}-c{cd}-r{int(reverse)}-crop0';config=configuration(0,yd,cd,False,cabac,weighted=weighted)
        # I reference fields use separately owned CAVLC packets for CAVLC and
        # the restart-tested CABAC single-slice packets for CABAC configurations.
        source_dir=ROOT/'avc-component-fields-cabac' if cabac else BASE
        source_name='cabac-'+base+'-single' if cabac else base
        packets=[(source_dir/f'{source_name}-{i}.packet').read_bytes() for i in range(2)]
        raw=(BASE/(base+'.y16')).read_bytes();source=struct.unpack('<1024H',raw);fields={}
        for index,bottom in enumerate([True,False] if reverse else [False,True]):
            packets.append(p_field(index,bottom,cabac,vector,weighted))
            compact=[v for y in range(16) for v in source[(2*y+int(bottom))*32:(2*y+int(bottom)+1)*32]]
            target=prediction(compact,yd,vector)
            if weighted:target=[max(0,min((1<<yd)-1,((3*v+1)//2)-5*(1<<(yd-8)))) for v in target]
            if vector!=(0,0) or weighted:assert target!=compact
            fields[bottom]=target
        woven=[v for y in range(16) for bottom in [False,True] for v in fields[bottom][y*32:(y+1)*32]]
        for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
        for ext,data in [('avcc',config),('mp4',mux(config,[(i,i==0,p) for i,p in enumerate(packets)],32,32,50)),('y16',raw+struct.pack('<1024H',*woven))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,luma_depth=yd,chroma_depth=cd,cabac=cabac,bottom_first=reverse,motion=vector,weighted=weighted))
    assert len(cases)==96
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
