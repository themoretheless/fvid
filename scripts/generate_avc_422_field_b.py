"""Own 422 reordered B fields with uni/bi/direct/skip and per-component weights."""
import json,struct,itertools
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_mono_field_b import b_field,future_field
from generate_avc_pcm_formats import mux
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
BASE=ROOT/'avc-component-fields';OUT=ROOT/'avc-422-field-b'
WEIGHTS=[[(3,-5),(1,7),(-2,64)],[(-1,90),(3,-4),(1,-32)]]
def blend(a,z,depth,component,mode,weight):
    scale=1<<(depth-8);denom=1 if component==0 else 2
    if mode in ['l0','l1']:
        l=int(mode=='l1');v=z if l else a
        if weight==1:
            w,o=WEIGHTS[l][component];v=((w*v+(1<<(denom-1)))>>denom)+o*scale
    elif weight==0:v=(a+z+1)//2
    elif weight==2:v=(43*a+21*z+32)//64
    else:
        wa,oa=WEIGHTS[0][component];wz,oz=WEIGHTS[1][component]
        v=((wa*a+wz*z+(1<<denom))>>(denom+1))+(((oa+oz)*scale+1)>>1)
    return max(0,min((1<<depth)-1,v))
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for (yd,cd),cabac,reverse,weight,mode in itertools.product([(8,8),(10,12),(14,8)],[False,True],[False,True],[0,1,2],['l0','l1','bi','spatial','temporal','skip']):
        name=f'422-field-b-y{yd}-c{cd}-e{int(cabac)}-r{int(reverse)}-w{weight}-{mode}'
        base=f'field-f2-y{yd}-c{cd}-r{int(reverse)}-crop0';config=configuration(2,yd,cd,False,cabac,bipred=weight)
        source_dir=ROOT/'avc-component-fields-cabac' if cabac else BASE;source_name='cabac-'+base+'-single' if cabac else base
        packets=[(source_dir/f'{source_name}-{i}.packet').read_bytes() for i in range(2)]
        raw=(BASE/(base+'.y16')).read_bytes();samples=struct.unpack('<2048H',raw);past=[samples[:1024],samples[1024:1536],samples[1536:]]
        future=[[(3*v+29+p*17)&((1<<d)-1) for v in plane] for p,(plane,d) in enumerate(zip(past,[yd,cd,cd]))];assert all(list(a)!=b for a,b in zip(past,future))
        order=[True,False] if reverse else [False,True]
        packets.extend(future_field(future[0],yd,bottom,i,cabac,future[1:],cd) for i,bottom in enumerate(order))
        packets.extend(b_field(bottom,i,cabac,mode,weight,chroma=True) for i,bottom in enumerate(order))
        target=[[blend(a,z,d,p,mode,weight) for a,z in zip(left,right)] for p,(left,right,d) in enumerate(zip(past,future,[yd,cd,cd]))]
        for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
        frames=[(pts,i==0,p) for i,(pts,p) in enumerate(zip([0,1,4,5,2,3],packets))]
        for ext,data in [('avcc',config),('mp4',mux(config,frames,32,32,50)),('y16',raw+struct.pack('<4096H',*(sum(target,[])+sum(future,[]))))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,luma_depth=yd,chroma_depth=cd,cabac=cabac,bottom_first=reverse,weight=weight,mode=mode,decode_order=[0,2,1]))
    assert len(cases)==216
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
