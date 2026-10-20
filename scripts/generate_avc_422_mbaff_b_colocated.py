"""Owned 422 MBAFF nonzero temporal-direct vectors, conversion and explicit twins."""
import itertools,json,struct
from functools import lru_cache
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_component_mbaff_pcm import MODES,picture
from generate_avc_422_mbaff_p import prediction
from generate_avc_422_mbaff_b import packet
from generate_avc_422_mbaff_p_fractional import expected
from generate_avc_pcm_formats import mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-mbaff-b-colocated'
SWAP={'frame':'field','field':'frame','columns':'checker','checker':'columns'}
@lru_cache(None)
def future_samples(yd,cd,reference_topology,motion):
    _,source=picture(2,yd,cd,False,'frame',False,0)
    return source,expected(source,yd,cd,reference_topology,motion,False)
@lru_cache(None)
def samples(yd,cd,topology,reference_topology,motion,temporal):
    source,later=future_samples(yd,cd,reference_topology,motion);vectors=[]
    for current_field,source_field in zip(MODES[topology],MODES[reference_topology]):
        vertical=motion[1]*2 if source_field and not current_field else int(motion[1]/2) if current_field and not source_field else motion[1]
        colocated=(motion[0],vertical)
        # POC0 -> POC4, B at POC2: distance scale factor 128. Signed
        # rounding is arithmetic (v+1)//2, after converting vertical units.
        left=tuple((v+1)//2 for v in colocated);right=tuple(a-b for a,b in zip(left,colocated))
        assert any(colocated) and any(left) and any(right)
        vectors.append([left,right] if temporal else [(0,0),(0,0)])
    a=expected(source,yd,cd,topology,(0,0),False,pair_motions=tuple(v[0] for v in vectors))
    z=expected(later,yd,cd,topology,(0,0),False,pair_motions=tuple(v[1] for v in vectors))
    target=[(left+right+1)//2 for left,right in zip(a,z)]
    if temporal:
        assert target!=[(left+right+1)//2 for left,right in zip(source,later)],'temporal scaling must change pixels'
    assert target!=a and target!=z,'both lists must affect reconstructed B samples'
    return source,later,target,vectors

def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for (yd,cd),cabac,topology,convert,motion,mode in itertools.product([(8,8),(10,12),(14,8)],[False,True],MODES,[False,True],[(8,4),(-8,-4),(3,3),(-3,-3)],['temporal','temporal-skip','spatial','spatial-skip','bi-derived']):
        reference_topology=SWAP[topology] if convert else topology;temporal=not mode.startswith('spatial');aso=(motion[0]<0)^cabac;crop=convert
        name=f'422-mbaff-col-y{yd}-c{cd}-e{int(cabac)}-{topology}-convert{int(convert)}-mv{motion[0]}_{motion[1]}-{mode}'
        config=configuration(2,yd,cd,crop,cabac,height_mbs=2,mbaff=True)
        source,later,target,vectors=samples(yd,cd,topology,reference_topology,motion,temporal)
        first,original=picture(2,yd,cd,cabac,topology,aso,0);assert original==source
        packets=[first,prediction(cabac,reference_topology,aso,motion,False,1,poc=4),packet(cabac,topology,aso,mode,pair_vectors=vectors if mode=='bi-derived' else None),prediction(cabac,topology,aso,(0,0),False,2,poc=6)]
        for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
        for ext,data in [('avcc',config),('mp4',mux(config,[(pts,i==0,p) for i,(pts,p) in enumerate(zip([0,2,1,3],packets))],32,64,25)),('y16',struct.pack('<16384H',*(source+target+later+later)))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,format=2,luma_depth=yd,chroma_depth=cd,cabac=cabac,topology=topology,reference_topology=reference_topology,convert=convert,aso=aso,motion=motion,mode=mode,decode_order=[0,2,1,3],pair_vectors=vectors,crop=[2,2,2,2] if crop else [0,0,0,0]))
    assert len(cases)==960
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');print('960 own nonzero co-located 422 MBAFF B movies')
if __name__=='__main__':generate()
