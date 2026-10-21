"""Owned reordered mono B fields: uni/bi/direct/skip and three weight modes."""
import json,struct,itertools
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_pcm_formats import Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
from generate_avc_field_b_cabac_mixed_samples import mb_type
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
BASE=ROOT/'avc-component-fields';OUT=ROOT/'avc-mono-field-b'
def future_field(source,depth,bottom,index,cabac,chroma_planes=None,chroma_depth=None):
    b=Writer();b.ue(0);b.ue(2);b.ue(0);b.u(1,4);b.u(1);b.u(int(bottom));b.u(6+index,4);b.u(0);b.se(0);b.ue(1)
    if cabac:
        while len(b.bits)%8:b.u(1)
        c=CabacWriter(-1,26)
    for mb in range(2):
        if cabac:c.decision(3+int(mb>0),1);b.bits.extend(c.finish());b.align()
        else:b.ue(25);b.align()
        for y in range(16):
            for x in range(16):b.u(source[(2*y+int(bottom))*32+mb*16+x],depth)
        if chroma_planes is not None:
            for plane in chroma_planes:
                for y in range(16):
                    for x in range(8):b.u(plane[(2*y+int(bottom))*16+mb*8+x],chroma_depth)
        if cabac:
            c.low=0;c.range=510;c.width=9
            if mb==0:c.range-=2;c.renormalize()
    if cabac:b.bits.extend(c.finish())
    nal=b.nal(0x41,trailing=not cabac);return len(nal).to_bytes(4,'big')+nal

def b_field(bottom,index,cabac,mode,weight,vectors=None,chroma=False,full_resolution=False,init_idc=0):
    vectors=vectors or [[0,0],[0,0]]
    b=Writer();b.ue(0);b.ue(1);b.ue(0);b.u(2,4);b.u(1);b.u(int(bottom));b.u(2+index,4)
    b.u(int(mode not in ['temporal','skip']));b.u(0)
    for difference in [3,1]:b.u(1);b.ue(0);b.ue(difference);b.ue(3)
    if weight==1:
        b.ue(1)
        if chroma:b.ue(2)
        for l,(w,offset) in enumerate([(3,-5),(-1,90)]):
            b.u(1);b.se(w);b.se(offset)
            if chroma:
                b.u(1)
                for cw,co in ([ (1,7),(-2,64) ] if l==0 else [ (3,-4),(1,-32) ]):b.se(cw);b.se(co)
    if cabac:b.ue(init_idc)
    b.se(0);b.ue(1)
    code={'l0':1,'l1':2,'bi':3,'spatial':0,'temporal':0,'skip':0}[mode]
    if cabac:
        while len(b.bits)%8:b.u(1)
        c=CabacWriter(init_idc,26)
        for mb in range(2):
            c.decision(24+int(mb>0 and mode!='skip'),int(mode=='skip'))
            if mode!='skip':
                mb_type(c,code,int(mb>0 and code!=0))
                for l in ([0,1] if code==3 else [code-1] if code in [1,2] else []):
                    for component in range(2):c.mvd(component,vectors[l][component] if mb==0 else 0,abs(vectors[l][component]) if mb else 0)
                for ctx in ([73,74,75,76] if mb==0 else [74,74,76,76]):c.decision(ctx,0)
                if chroma and not full_resolution:c.decision(77,0)
            if mb==0:c.range-=2;c.renormalize()
        b.bits.extend(c.finish())
    elif mode=='skip':b.ue(2)
    else:
        for mb in range(2):
            b.ue(0);b.ue(code)
            for l in ([0,1] if code==3 else [code-1] if code in [1,2] else []):b.se(vectors[l][0] if mb==0 else 0);b.se(vectors[l][1] if mb==0 else 0)
            b.ue(0)
    nal=b.nal(0x01,trailing=not cabac);return len(nal).to_bytes(4,'big')+nal

def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for (yd,cd),cabac,reverse,weight,mode in itertools.product([(8,8),(10,12),(14,8)],[False,True],[False,True],[0,1,2],['l0','l1','bi','spatial','temporal','skip']):
        name=f'mono-field-b-y{yd}-c{cd}-e{int(cabac)}-r{int(reverse)}-w{weight}-{mode}'
        base=f'field-f0-y{yd}-c{cd}-r{int(reverse)}-crop0';config=configuration(0,yd,cd,False,cabac,bipred=weight)
        source_dir=ROOT/'avc-component-fields-cabac' if cabac else BASE;source_name='cabac-'+base+'-single' if cabac else base
        packets=[(source_dir/f'{source_name}-{i}.packet').read_bytes() for i in range(2)]
        raw=(BASE/(base+'.y16')).read_bytes();past=struct.unpack('<1024H',raw);future=[(3*v+29)&((1<<yd)-1) for v in past];assert future!=list(past)
        order=[True,False] if reverse else [False,True]
        packets.extend(future_field(future,yd,bottom,index,cabac) for index,bottom in enumerate(order))
        packets.extend(b_field(bottom,index,cabac,mode,weight) for index,bottom in enumerate(order));target=[];scale=1<<(yd-8)
        for a,z in zip(past,future):
            if mode in ['l0','l1']:
                v=a if mode=='l0' else z
                if weight==1:
                    w,o=(3,-5) if mode=='l0' else (-1,90);v=((w*v+1)>>1)+o*scale
            elif weight==0:v=(a+z+1)//2
            elif weight==2:v=(43*a+21*z+32)//64
            else:v=((3*a-z+2)>>2)+((85*scale+1)>>1)
            target.append(max(0,min((1<<yd)-1,v)))
        for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
        frames=[(pts,i==0,p) for i,(pts,p) in enumerate(zip([0,1,4,5,2,3],packets))]
        for ext,data in [('avcc',config),('mp4',mux(config,frames,32,32,50)),('y16',raw+struct.pack('<2048H',*(target+future))) ]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,luma_depth=yd,chroma_depth=cd,cabac=cabac,bottom_first=reverse,weight=weight,mode=mode,decode_order=[0,2,1]))
    assert len(cases)==216
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
