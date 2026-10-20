"""Own monochrome P field DC/field-AC residual over scalar weighted qpel."""
import json,struct
from pathlib import Path
from generate_avc_mono_field_p import p_field
from generate_avc_pcm_formats import mux
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
BASE=ROOT/'avc-mono-field-p';OUT=ROOT/'avc-mono-field-p-residual'
def generate(base_dir=BASE,out_dir=OUT):
    BASE,OUT=base_dir,out_dir
    OUT.mkdir(exist_ok=True);cases=[]
    for case in json.loads((BASE/'manifest.json').read_text()):
        original=case['name'];depth=case['luma_depth'];scale=1<<(depth-8)
        config=(BASE/(original+'.avcc')).read_bytes();raw=(BASE/(original+'.y16')).read_bytes()
        samples=struct.unpack('<2048H',raw)
        for kind in ['dc','ac']:
            name=original+'-'+kind;packets=[(BASE/f'{original}-{i}.packet').read_bytes() for i in range(2)];target=[];witnesses=0
            for y in range(32):
                for x in range(32):
                    mb=x//16;bx=x%16//4;sign=1 if (mb+bx)%2==0 else -1
                    if kind=='dc':residual=(sign*13*16*scale+32)//64
                    else:residual=([2,1,-1,-2][(y//2)%4]*sign*8*16*scale+32)//64
                    prediction=samples[1024+y*32+x];v=max(0,min((1<<depth)-1,prediction+residual));target.append(v)
                    if kind=='ac':
                        wrong=([2,1,-1,-2][x%4]*sign*8*16*scale+32)//64
                        witnesses+=int(v!=max(0,min((1<<depth)-1,prediction+wrong)))
            if kind=='ac':assert witnesses>0,'field residual must differ from frame scan'
            assert target!=list(samples[1024:])
            for index,bottom in enumerate([True,False] if case['bottom_first'] else [False,True]):packets.append(p_field(index,bottom,case['cabac'],tuple(case['motion']),case['weighted'],kind,opposite=case.get('opposite_parity',False)))
            for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
            for ext,data in [('avcc',config),('mp4',mux(config,[(i,i==0,p) for i,p in enumerate(packets)],32,32,50)),('y16',raw[:2048]+struct.pack('<1024H',*target))]:(OUT/f'{name}.{ext}').write_bytes(data)
            cases.append(dict(case,name=name,residual=kind))
    assert len(cases)==192
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
