"""Owned 422 MBAFF explicit/implicit B weights with non-half temporal distances."""
import itertools,json,struct
from functools import lru_cache
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_component_mbaff_pcm import MODES,picture
from generate_avc_422_mbaff_p import prediction
from generate_avc_422_mbaff_b import packet
from generate_avc_422_mbaff_p_fractional import expected
from generate_avc_422_mbaff_b_colocated import SWAP,future_samples
from generate_avc_pcm_formats import mux
from avc_fixture_weights import B_PROFILES,uni,bi
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-mbaff-b-weighted'
@lru_cache(None)
def samples(yd,cd,topology,profile,mode):
    reference_topology=SWAP[topology];motion=(-3,-3) if profile=='extreme' else (3,3)
    source,later=future_samples(yd,cd,reference_topology,motion);vectors=[]
    for current_field,col_field in zip(MODES[topology],MODES[reference_topology]):
        if mode.startswith('temporal'):
            vy=motion[1]*2 if col_field and not current_field else int(motion[1]/2) if current_field and not col_field else motion[1]
            col=(motion[0],vy);a=tuple((85*v+128)//256 for v in col);z=tuple(x-y for x,y in zip(a,col));vectors.append([a,z])
        elif mode in ['bi-motion','l0','l1']:vectors.append([(1,-1),(-3,1)])
        else:vectors.append([(0,0),(0,0)])
    opposite=mode=='bi-motion'
    a=expected(source,yd,cd,topology,(0,0),opposite,pair_motions=tuple(v[0] for v in vectors))
    z=expected(later,yd,cd,topology,(0,0),opposite,pair_motions=tuple(v[1] for v in vectors))
    default=a if mode=='l0' else z if mode=='l1' else [(left+right+1)//2 for left,right in zip(a,z)]
    if profile=='implicit':
        if mode in ['l0','l1']:target=default
        else:target=[(43*left+21*right+32)//64 for left,right in zip(a,z)];assert target!=default
        clipped=0
    elif mode in ['l0','l1']:target,clipped=uni(a if mode=='l0' else z,[yd,cd,cd],B_PROFILES[profile],int(mode=='l1'))
    else:target,clipped=bi(a,z,[yd,cd,cd],B_PROFILES[profile])
    if profile=='default':assert target==default
    elif profile!='implicit':assert target!=default
    return source,later,target,vectors,motion,clipped

def generate():
    OUT.mkdir(exist_ok=True);cases=[];clipped=0
    for (yd,cd),cabac,topology,profile,mode in itertools.product([(8,8),(10,12),(14,8)],[False,True],MODES,['mixed','extreme','default','implicit'],['l0','l1','bi-motion','spatial','temporal','temporal-skip']):
        reference_topology=SWAP[topology];crop=profile in ['mixed','extreme'];aso=mode in ['l1','temporal-skip'];bipred=2 if profile=='implicit' else 1
        name=f'422-mbaff-wb-y{yd}-c{cd}-e{int(cabac)}-{topology}-{profile}-{mode}'
        source,later,target,vectors,motion,n=samples(yd,cd,topology,profile,mode);clipped+=n
        config=configuration(2,yd,cd,crop,cabac,height_mbs=2,mbaff=True,bipred=bipred);first,original=picture(2,yd,cd,cabac,topology,aso,0);assert original==source
        weights=B_PROFILES.get(profile)
        packets=[first,prediction(cabac,reference_topology,aso,motion,False,1,poc=6),packet(cabac,topology,aso,mode,pair_vectors=vectors,poc=2,weights=weights),prediction(cabac,topology,aso,(0,0),False,2,poc=8)]
        for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
        for ext,data in [('avcc',config),('mp4',mux(config,[(pts,i==0,p) for i,(pts,p) in enumerate(zip([0,2,1,3],packets))],32,64,25)),('y16',struct.pack('<16384H',*(source+target+later+later)))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,format=2,luma_depth=yd,chroma_depth=cd,cabac=cabac,topology=topology,reference_topology=reference_topology,profile=profile,mode=mode,bipred=bipred,weights=weights,future_motion=motion,pair_vectors=vectors,decode_order=[0,2,1,3],crop=[2,2,2,2] if crop else [0,0,0,0]))
    assert len(cases)==576 and clipped>0
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');print('576 own weighted B MBAFF movies; clipped predictions:',clipped)
if __name__=='__main__':generate()
