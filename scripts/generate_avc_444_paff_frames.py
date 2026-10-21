"""Owned full 4:4:4 CAVLC/CABAC I frames under field-capable PAFF SPS geometry."""
import itertools,json,struct
from pathlib import Path
from generate_avc_444_fields import configuration,field,mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-paff-frames'
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for (yd,cd),cabac,offsets,mode in itertools.product([(8,8),(10,12),(14,8)],[False,True],[(0,0),(-12,12),(12,-12)],[0,1,2]):
        name=f'y{yd}-c{cd}-e{int(cabac)}-o{offsets[0]}-{offsets[1]}-f{mode}'
        cfg=configuration(yd,cd,cabac,offsets,height_map_units=1)
        packet,planes,_=field(yd,cd,cabac,offsets,mode,'dc',None,0)
        for ext,data in [('avcc',cfg),('packet',packet),('mp4',mux(cfg,[(0,True,packet)],32,32,25)),('y16',struct.pack('<3072H',*sum(planes,[])))]: (OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,yd=yd,cd=cd,mode=mode))
    assert len(cases)==54
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance=__doc__,cases=cases),indent=2)+'\n')
    print('PAFF frame cases',len(cases))
if __name__=='__main__':generate()
