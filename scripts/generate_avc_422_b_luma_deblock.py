"""Owned B bi luma-residual bS=2 using the separate QP32 scalar oracle."""
import json
from pathlib import Path
from generate_avc_pcm_formats import mux
from generate_avc_422_p_luma_deblock import packet
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
SCALAR=ROOT/'avc-422-p-luma-deblock';BASE=ROOT/'avc-422-b-deblock';OUT=ROOT/'avc-422-b-luma-deblock'
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for case in json.loads((SCALAR/'manifest.json').read_text()):
        yd,cd=case['luma_depth'],case['chroma_depth'];cabac=case['cabac'];axis=case['axis'];name='b-'+case['name']
        base=f'b-list0-422-inter-filter-y{yd}-c{cd}-e{int(cabac)}-a{axis}-m1'
        config=(BASE/(base+'.avcc')).read_bytes();first=(BASE/(base+'-0.packet')).read_bytes();future=(BASE/(base+'-1.packet')).read_bytes()
        middle=packet(cabac,axis,case['filter_mode'],True)
        raw=(SCALAR/(case['name']+'.y16')).read_bytes()
        for i,data in enumerate([first,future,middle]):(OUT/f'{name}-{i}.packet').write_bytes(data)
        for ext,data in [('avcc',config),('mp4',mux(config,[(0,True,first),(2,False,future),(1,False,middle)],32,32,25)),('y16',raw+raw[:4096])]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(case,name=name,decode_order=[0,2,1]))
    assert len(cases)==36
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
