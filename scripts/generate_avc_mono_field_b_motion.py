"""Owned mono B fields with moving colocated P fields and nested scalar MC."""
import json,struct
from pathlib import Path
from generate_avc_mono_field_b import b_field
from generate_avc_mono_field_p import p_field
from generate_avc_monochrome_fractional import prediction
from generate_avc_pcm_formats import mux
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
BASE=ROOT/'avc-mono-field-b';OUT=ROOT/'avc-mono-field-b-motion'
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for case in json.loads((BASE/'manifest.json').read_text()):
        original=case['name'];depth=case['luma_depth'];scale=1<<(depth-8);maximum=(1<<depth)-1
        config=(BASE/(original+'.avcc')).read_bytes();raw=(BASE/(original+'.y16')).read_bytes()[:2048];past=struct.unpack('<1024H',raw)
        for vector in [(8,-4),(-7,5)]:
            name=original+f'-mv{vector[0]}-{vector[1]}';packets=[(BASE/f'{original}-{i}.packet').read_bytes() for i in range(2)]
            scaled=[(85*v+128)//256 for v in vector];back=[scaled[i]-vector[i] for i in range(2)];mode=case['mode'];weight=case['weight']
            vectors=[[0,0],[0,0]] if mode=='spatial' else [scaled,back]
            futures={};targets={};order=[True,False] if case['bottom_first'] else [False,True]
            for bottom in [False,True]:
                compact=[v for y in range(16) for v in past[(2*y+int(bottom))*32:(2*y+int(bottom)+1)*32]]
                future=prediction(compact,depth,vector);futures[bottom]=future
                a=prediction(compact,depth,vectors[0]);z=prediction(future,depth,vectors[1]);target=[]
                for x,y in zip(a,z):
                    if mode in ['l0','l1']:
                        v=x if mode=='l0' else y
                        if weight==1:
                            w,o=(3,-5) if mode=='l0' else (-1,90);v=((w*v+1)>>1)+o*scale
                    elif weight==0:v=(x+y+1)//2
                    elif weight==2:v=(43*x+21*y+32)//64
                    else:v=((3*x-y+2)>>2)+((85*scale+1)>>1)
                    target.append(max(0,min(maximum,v)))
                if mode=='temporal' and weight==0:assert target!=[(x+y+1)//2 for x,y in zip(compact,future)], 'temporal motion must differ from zero motion'
                targets[bottom]=target
            packets.extend(p_field(i,bottom,case['cabac'],vector,False,poc_base=6) for i,bottom in enumerate(order))
            packets.extend(b_field(bottom,i,case['cabac'],mode,weight,vectors) for i,bottom in enumerate(order))
            woven=lambda fields:[v for y in range(16) for bottom in [False,True] for v in fields[bottom][y*32:(y+1)*32]]
            for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
            frames=[(pts,i==0,p) for i,(pts,p) in enumerate(zip([0,1,4,5,2,3],packets))]
            for ext,data in [('avcc',config),('mp4',mux(config,frames,32,32,50)),('y16',raw+struct.pack('<2048H',*(woven(targets)+woven(futures))))]:(OUT/f'{name}.{ext}').write_bytes(data)
            cases.append(dict(case,name=name,colocated_motion=vector,list_motion=vectors))
    assert len(cases)==432
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
