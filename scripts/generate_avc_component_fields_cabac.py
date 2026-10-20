"""Own CABAC PCM complementary fields, restarts, multislice and ASO."""
import json,struct
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_pcm_formats import Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
BASE=ROOT/'avc-component-fields';OUT=ROOT/'avc-component-fields-cabac'
def field(case,source,index,bottom,layout):
    fmt=case['format'];yd,cd=case['luma_depth'],case['chroma_depth'];sx=2 if fmt==2 else 1
    c=0 if fmt==0 else 1024//sx;offsets=[0,1024,1024+c];nals=[]
    for addresses in ([list(range(2))] if layout=='single' else [[0],[1]]):
        b=Writer();b.ue(addresses[0]);b.ue(2);b.ue(0);b.u(0,4);b.u(1);b.u(int(bottom))
        if index==0:b.ue(0)
        b.u(index,4)
        if index==0:b.u(0);b.u(0)
        else:b.u(0)
        b.se(0);b.ue(1)
        while len(b.bits)%8:b.u(1)
        engine=CabacWriter(-1,26)
        for position,mb in enumerate(addresses):
            engine.decision(3+int(position>0),1);b.bits.extend(engine.finish());b.align()
            for p,(w,bw,depth) in enumerate([(32,16,yd),(32//sx,16//sx,cd),(32//sx,16//sx,cd)]):
                if fmt==0 and p:continue
                for y in range(16):
                    for x in range(bw):b.u(source[offsets[p]+(2*y+int(bottom))*w+mb*bw+x],depth)
            engine.low=0;engine.range=510;engine.width=9
            if position<len(addresses)-1:engine.range-=2;engine.renormalize()
        b.bits.extend(engine.finish());nals.append(b.nal(0x65 if index==0 else 0x41,trailing=False))
    if layout=='aso':nals.reverse()
    return b''.join(len(n).to_bytes(4,'big')+n for n in nals)
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for case in json.loads((BASE/'manifest.json').read_text()):
        original=case['name'];raw=(BASE/(original+'.y16')).read_bytes();source=struct.unpack('<'+str(len(raw)//2)+'H',raw)
        config=configuration(case['format'],case['luma_depth'],case['chroma_depth'],any(case['crop']),True)
        for layout in ['single','multi','aso']:
            name='cabac-'+original+'-'+layout
            packets=[field(case,source,i,bottom,layout) for i,bottom in enumerate([True,False] if case['bottom_first'] else [False,True])]
            for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
            for ext,data in [('avcc',config),('mp4',mux(config,[(i,i==0,p) for i,p in enumerate(packets)],32,32,50)),('y16',raw)]:(OUT/f'{name}.{ext}').write_bytes(data)
            cases.append(dict(case,name=name,cabac=True,layout=layout))
    assert len(cases)==108
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
