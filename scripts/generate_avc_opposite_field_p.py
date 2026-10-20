"""Owned mono/422 P opposite-parity references, qpel and component weights."""
import json,struct
from pathlib import Path
from generate_avc_mono_field_p import p_field
from generate_avc_monochrome_fractional import prediction
from generate_avc_pcm_formats import mux
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
def chroma(source,depth,vector,phase=0):
    at=lambda x,y:source[max(0,min(15,y))*16+max(0,min(15,x))]
    out=[]
    for y in range(16):
        for x in range(16):
            u=x*8+vector[0];v=y*8+2*vector[1]+phase;ix,iy=u//8,v//8;fx,fy=u%8,v%8
            out.append(((8-fx)*(8-fy)*at(ix,iy)+fx*(8-fy)*at(ix+1,iy)+(8-fx)*fy*at(ix,iy+1)+fx*fy*at(ix+1,iy+1)+32)//64)
    return out

def weighted(values,depth,component,enabled):
    if not enabled:return values
    w,o,denom=[(3,-5,1),(3,7,2),(-2,64,2)][component]
    return [max(0,min((1<<depth)-1,((w*v+(1<<(denom-1)))>>denom)+o*(1<<(depth-8)))) for v in values]

def generate():
    for fmt,label in [(0,'mono'),(2,'422')]:
        base=ROOT/f'avc-{label}-field-p';out=ROOT/f'avc-{label}-field-p-opposite';out.mkdir(exist_ok=True);cases=[]
        for case in json.loads((base/'manifest.json').read_text()):
            original=case['name'];name='opposite-'+original;yd,cd=case['luma_depth'],case['chroma_depth'];n=1024 if fmt==0 else 2048
            raw=(base/(original+'.y16')).read_bytes();samples=struct.unpack('<'+str(2*n)+'H',raw)
            planes=[samples[:1024]] if fmt==0 else [samples[:1024],samples[1024:1536],samples[1536:2048]]
            packets=[(base/f'{original}-{i}.packet').read_bytes() for i in range(2)];fields={}
            for index,bottom in enumerate([True,False] if case['bottom_first'] else [False,True]):
                packets.append(p_field(index,bottom,case['cabac'],tuple(case['motion']),case['weighted'],chroma=fmt==2,opposite=True));target=[]
                for p,(source,w,d) in enumerate(zip(planes,[32,16,16],[yd,cd,cd])):
                    compact=[v for y in range(16) for v in source[(2*y+int(not bottom))*w:(2*y+int(not bottom)+1)*w]]
                    values=prediction(compact,d,case['motion']) if p==0 else chroma(compact,d,case['motion'])
                    values=weighted(values,d,p,case['weighted'])
                    if p:
                        phase=2 if bottom else -2
                        wrong=weighted(chroma(compact,d,case['motion'],phase),d,p,case['weighted'])
                        assert values!=wrong, '422 must differ from the 420 parity phase'
                    target.append(values)
                fields[bottom]=target
            woven=[v for p,w in enumerate([32] if fmt==0 else [32,16,16]) for y in range(16) for bottom in [False,True] for v in fields[bottom][p][y*w:(y+1)*w]]
            assert woven!=list(samples[n:]),'opposite parity must differ from matching parity'
            for i,data in enumerate(packets):(out/f'{name}-{i}.packet').write_bytes(data)
            config=(base/(original+'.avcc')).read_bytes()
            for ext,data in [('avcc',config),('mp4',mux(config,[(i,i==0,p) for i,p in enumerate(packets)],32,32,50)),('y16',raw[:2*n]+struct.pack('<'+str(n)+'H',*woven))]:(out/f'{name}.{ext}').write_bytes(data)
            cases.append(dict(case,name=name,opposite_parity=True))
        assert len(cases)==96
        (out/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
