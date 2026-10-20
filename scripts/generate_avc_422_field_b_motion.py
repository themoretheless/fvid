"""Own 422 B fields with moving future P fields and nested scalar component MC."""
import json,struct
from pathlib import Path
from generate_avc_422_field_b import blend
from generate_avc_mono_field_b import b_field
from generate_avc_mono_field_p import p_field
from generate_avc_monochrome_fractional import prediction
from generate_avc_422_inter import chroma
from generate_avc_pcm_formats import mux
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
BASE=ROOT/'avc-422-field-b';OUT=ROOT/'avc-422-field-b-motion'
def mc(planes,depths,vector):return [prediction(planes[0],depths[0],vector),chroma(planes[1],depths[1],vector),chroma(planes[2],depths[2],vector)]
def weave(fields):return [v for p,w in enumerate([32,16,16]) for y in range(16) for bottom in [False,True] for v in fields[bottom][p][y*w:(y+1)*w]]
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for case in json.loads((BASE/'manifest.json').read_text()):
        original=case['name'];depths=[case['luma_depth'],case['chroma_depth'],case['chroma_depth']];mode=case['mode'];weight=case['weight']
        config=(BASE/(original+'.avcc')).read_bytes();raw=(BASE/(original+'.y16')).read_bytes()[:4096];samples=struct.unpack('<2048H',raw);past=[samples[:1024],samples[1024:1536],samples[1536:]]
        for vector in [(8,-4),(-7,5)]:
            name=original+f'-mv{vector[0]}-{vector[1]}';packets=[(BASE/f'{original}-{i}.packet').read_bytes() for i in range(2)]
            scaled=[(85*v+128)//256 for v in vector];back=[scaled[i]-vector[i] for i in range(2)];vectors=[[0,0],[0,0]] if mode=='spatial' else [scaled,back]
            futures={};targets={};order=[True,False] if case['bottom_first'] else [False,True]
            for bottom in [False,True]:
                compact=[[v for y in range(16) for v in plane[(2*y+int(bottom))*w:(2*y+int(bottom)+1)*w]] for plane,w in zip(past,[32,16,16])]
                future=mc(compact,depths,vector);futures[bottom]=future
                a=mc(compact,depths,vectors[0]);z=mc(future,depths,vectors[1])
                target=[[blend(x,y,d,p,mode,weight) for x,y in zip(left,right)] for p,(left,right,d) in enumerate(zip(a,z,depths))]
                if mode=='temporal' and weight==0:
                    for p in range(3):assert target[p]!=[(x+y+1)//2 for x,y in zip(compact[p],future[p])], 'temporal component must differ from zero motion'
                targets[bottom]=target
            packets.extend(p_field(i,bottom,case['cabac'],vector,False,poc_base=6,chroma=True) for i,bottom in enumerate(order))
            packets.extend(b_field(bottom,i,case['cabac'],mode,weight,vectors,chroma=True) for i,bottom in enumerate(order))
            for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
            frames=[(pts,i==0,p) for i,(pts,p) in enumerate(zip([0,1,4,5,2,3],packets))]
            for ext,data in [('avcc',config),('mp4',mux(config,frames,32,32,50)),('y16',raw+struct.pack('<4096H',*(weave(targets)+weave(futures))))]:(OUT/f'{name}.{ext}').write_bytes(data)
            cases.append(dict(case,name=name,colocated_motion=vector,list_motion=vectors))
    assert len(cases)==432
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
