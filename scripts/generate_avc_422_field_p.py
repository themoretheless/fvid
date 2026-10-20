"""Owned 422 P fields: qpel, full-height chroma MC and distinct component weights."""
import json,struct,itertools
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_mono_field_p import p_field
from generate_avc_monochrome_fractional import prediction
from generate_avc_422_inter import chroma
from generate_avc_pcm_formats import mux
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
BASE=ROOT/'avc-component-fields';OUT=ROOT/'avc-422-field-p'
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for (yd,cd),cabac,reverse,vector,weighted in itertools.product([(8,8),(10,12),(14,8)],[False,True],[False,True],[(0,0),(1,1),(-3,-1),(4,-4)],[False,True]):
        name=f'422-field-p-y{yd}-c{cd}-e{int(cabac)}-r{int(reverse)}-x{vector[0]}-y{vector[1]}-w{int(weighted)}'
        base=f'field-f2-y{yd}-c{cd}-r{int(reverse)}-crop0';config=configuration(2,yd,cd,False,cabac,weighted=weighted)
        source_dir=ROOT/'avc-component-fields-cabac' if cabac else BASE;source_name='cabac-'+base+'-single' if cabac else base
        packets=[(source_dir/f'{source_name}-{i}.packet').read_bytes() for i in range(2)]
        raw=(BASE/(base+'.y16')).read_bytes();samples=struct.unpack('<2048H',raw);planes=[samples[:1024],samples[1024:1536],samples[1536:]];fields={}
        for index,bottom in enumerate([True,False] if reverse else [False,True]):
            packets.append(p_field(index,bottom,cabac,vector,weighted,chroma=True));target=[]
            for component,(source,w,d) in enumerate(zip(planes,[32,16,16],[yd,cd,cd])):
                compact=[v for y in range(16) for v in source[(2*y+int(bottom))*w:(2*y+int(bottom)+1)*w]]
                predicted=prediction(compact,d,vector) if component==0 else chroma(compact,d,vector)
                if weighted:
                    weight,offset,denom=[(3,-5,1),(3,7,2),(-2,64,2)][component]
                    predicted=[max(0,min((1<<d)-1,((weight*v+(1<<(denom-1)))>>denom)+offset*(1<<(d-8)))) for v in predicted]
                if vector!=(0,0) or weighted:assert predicted!=compact
                target.append(predicted)
            fields[bottom]=target
        woven=[]
        for component,w in enumerate([32,16,16]):
            for y in range(16):
                for bottom in [False,True]:woven.extend(fields[bottom][component][y*w:(y+1)*w])
        for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
        for ext,data in [('avcc',config),('mp4',mux(config,[(i,i==0,p) for i,p in enumerate(packets)],32,32,50)),('y16',raw+struct.pack('<2048H',*woven))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,luma_depth=yd,chroma_depth=cd,cabac=cabac,bottom_first=reverse,motion=vector,weighted=weighted))
    assert len(cases)==96
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
