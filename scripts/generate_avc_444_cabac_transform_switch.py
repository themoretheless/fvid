"""Owned 4:4:4 CABAC transform-size switches requiring category-aware neighbour contexts."""
import itertools,json,struct
from pathlib import Path
from generate_avc_444_cabac_intra import make_case,mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-cabac-transform-switch'
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for (yd,cd),modes,offsets,filter_mode in itertools.product([(8,8),(10,12),(14,8)],
        [('i4-dc','i8-dc'),('i8-dc','i4-dc'),('i16-dc','i8-dc'),('i8-dc','i16-dc'),('i4-cb-only','i8-cb-only'),('i8-cb-only','i4-cb-only')],
        [(0,0),(-6,6),(6,-6)],[0,1,2]):
        name=f'{modes[0]}-to-{modes[1]}-y{yd}-c{cd}-o{offsets[0]}-{offsets[1]}-f{filter_mode}'
        cfg,packet,expected=make_case(yd,cd,modes[0],True,offsets,filter_mode,modes=modes)
        for ext,data in [('avcc',cfg),('packet',packet),('mp4',mux(cfg,[(0,True,packet)],32,16,25)),('y16',struct.pack('<1536H',*expected))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,yd=yd,cd=cd,crop=[1]*4))
    assert len(cases)==162
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance=__doc__,cases=cases),indent=2)+'\n')
    print('transform switch cases',len(cases))
if __name__=='__main__':generate()
